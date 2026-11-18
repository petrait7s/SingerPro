//! Installment agreements: schedule, collections with transparent allocation, overdue tracking,
//! early settlement, rescheduling, explicit late fees, reversals, follow-ups.
//!
//! Money model (all shown separately to the user):
//!   price_total   = invoice total of the goods (incl. tax)
//!   down_payment  = received at signing (a normal sale receipt)
//!   principal     = price_total - down_payment            (financed amount)
//!   markup        = principal x markup_bp                  (default 0%)
//!   fees          = explicit extra charges (default none)
//!   total_payable = principal + markup + fees              (collected over the schedule)
//! Accounting: see docs/ACCOUNTING_POLICY.md.

use crate::accounting::{self, code, cr, dr, method_account, PostIn};
use crate::audit;
use crate::authz::{perm, Principal};
use crate::customers;
use crate::{sequence, time, CoreError, CoreResult, Id, Money};
use chrono::{Duration, Months, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Clone, Deserialize)]
pub struct FeeIn {
    pub label: String,
    pub amount: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomDue {
    pub due_date: String,
    pub amount: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallmentIn {
    /// `None` = the shop default. Anything else needs `installments.set_markup`.
    pub markup_bp: Option<i64>,
    pub count: i64,
    /// weekly | monthly | custom
    pub frequency: String,
    pub first_due_date: String,
    #[serde(default)]
    pub custom: Vec<CustomDue>,
    #[serde(default)]
    pub fees: Vec<FeeIn>,
    #[serde(default)]
    pub notes: String,
}

fn parse_date(label: &str, s: &str) -> CoreResult<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| CoreError::Validation(format!("{label} must be a date like 2026-01-31")))
}

async fn setting(conn: &mut SqliteConnection, key: &str) -> CoreResult<String> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or_default(),
    )
}

async fn setting_i64(conn: &mut SqliteConnection, key: &str) -> CoreResult<i64> {
    Ok(setting(conn, key).await?.parse().unwrap_or(0))
}

/// Schedule (due dates and amounts). The rounding remainder always lands on the LAST installment.
pub fn build_schedule(
    frequency: &str,
    first_due: NaiveDate,
    count: i64,
    custom: &[CustomDue],
    total: i64,
) -> CoreResult<Vec<(NaiveDate, i64)>> {
    if total <= 0 {
        return Err(CoreError::Validation("nothing to schedule".into()));
    }
    match frequency {
        "custom" => {
            if custom.is_empty() || custom.len() > 120 {
                return Err(CoreError::Validation(
                    "a custom schedule needs 1 to 120 installments".into(),
                ));
            }
            let mut out = Vec::with_capacity(custom.len());
            let mut prev: Option<NaiveDate> = None;
            let mut sum = 0i64;
            for c in custom {
                let d = parse_date("installment date", &c.due_date)?;
                if c.amount <= 0 {
                    return Err(CoreError::Validation(
                        "each installment must be positive".into(),
                    ));
                }
                if prev.is_some_and(|p| d <= p) {
                    return Err(CoreError::Validation(
                        "installment dates must be in increasing order".into(),
                    ));
                }
                prev = Some(d);
                sum = sum.checked_add(c.amount).ok_or(CoreError::Overflow)?;
                out.push((d, c.amount));
            }
            if sum != total {
                return Err(CoreError::Validation(format!(
                    "custom installments add up to {} but {} must be scheduled",
                    Money::from_paisa(sum),
                    Money::from_paisa(total)
                )));
            }
            Ok(out)
        }
        "weekly" | "monthly" => {
            if !(1..=120).contains(&count) {
                return Err(CoreError::Validation(
                    "number of installments must be 1 to 120".into(),
                ));
            }
            let amounts = Money::from_paisa(total).split_even_remainder_last(count as u32)?;
            let mut out = Vec::with_capacity(count as usize);
            for (i, a) in amounts.into_iter().enumerate() {
                let d = if frequency == "weekly" {
                    first_due + Duration::days(7 * i as i64)
                } else {
                    first_due
                        .checked_add_months(Months::new(i as u32))
                        .ok_or_else(|| CoreError::Validation("date out of range".into()))?
                };
                out.push((d, a.paisa()));
            }
            Ok(out)
        }
        _ => Err(CoreError::Validation(
            "frequency must be weekly, monthly or custom".into(),
        )),
    }
}

/// Markup income earned so far under the proportional policy.
/// `u` unearned total at signing, `t` total payable at signing, `c` payments collected, `r` rebates granted.
pub fn earned_markup(u: i64, t: i64, c: i64, r: i64) -> i64 {
    if t <= 0 {
        return 0;
    }
    if c + r >= t {
        return u - r;
    }
    let num = (u as i128) * (c.max(0) as i128);
    let rounded = (num * 2 + t as i128) / (t as i128 * 2);
    (rounded as i64).min(u - r).max(0)
}

pub(crate) struct AgreementCtx<'a> {
    pub sale_id: &'a str,
    pub sale_number: &'a str,
    pub customer_id: &'a str,
    pub sale_date: &'a str,
    pub cash_price: i64,
    pub price_total: i64,
    pub down_payment: i64,
}

pub(crate) async fn create_agreement(
    conn: &mut SqliteConnection,
    p: &Principal,
    ctx: &AgreementCtx<'_>,
    inst: &InstallmentIn,
) -> CoreResult<(String, i64)> {
    let sale_date = parse_date("sale date", ctx.sale_date)?;
    let first_due = parse_date("first due date", &inst.first_due_date)?;
    if first_due < sale_date {
        return Err(CoreError::Validation(
            "the first installment cannot be due before the sale date".into(),
        ));
    }
    let default_bp = setting_i64(conn, "installment.default_markup_bp").await?;
    let markup_bp = inst.markup_bp.unwrap_or(default_bp);
    if !(0..=10_000).contains(&markup_bp) {
        return Err(CoreError::Validation(
            "markup must be between 0% and 100%".into(),
        ));
    }
    if markup_bp != default_bp {
        p.require(perm::INSTALLMENTS_MARKUP)?;
    }
    let mut fees_total = 0i64;
    if !inst.fees.is_empty() {
        p.require(perm::INSTALLMENTS_MARKUP)?;
    }
    for f in &inst.fees {
        if f.label.trim().is_empty() || f.label.chars().count() > 60 || f.amount <= 0 {
            return Err(CoreError::Validation(
                "each fee needs a label and a positive amount".into(),
            ));
        }
        fees_total = fees_total
            .checked_add(f.amount)
            .ok_or(CoreError::Overflow)?;
    }
    let min_bp = setting_i64(conn, "installment.min_down_payment_bp").await?;
    let min_dp = Money::from_paisa(ctx.price_total)
        .percent_bp(min_bp)?
        .paisa();
    if ctx.down_payment < min_dp {
        return Err(CoreError::Validation(format!(
            "the down payment must be at least {} ({}% of the price)",
            Money::from_paisa(min_dp),
            min_bp as f64 / 100.0
        )));
    }
    let principal = ctx.price_total - ctx.down_payment;
    if principal <= 0 {
        return Err(CoreError::Validation(
            "the down payment covers the whole price; record a normal sale instead".into(),
        ));
    }
    let markup = Money::from_paisa(principal).percent_bp(markup_bp)?.paisa();
    let total_payable = principal + markup + fees_total;
    let schedule = build_schedule(
        &inst.frequency,
        first_due,
        inst.count,
        &inst.custom,
        total_payable,
    )?;
    let policy = setting(conn, "installment.markup_recognition").await?;
    let policy = if policy == "on_agreement" {
        "on_agreement"
    } else {
        "proportional_to_collections"
    };

    sequence::ensure(conn, "installment", "INS-", 6).await?;
    let number = sequence::next(conn, "installment").await?;
    let aid = Id::new().to_string();
    sqlx::query("INSERT INTO installment_agreements (id, number, sale_id, customer_id, cash_price, price_total, down_payment, principal, markup_bp, markup, fees_total, total_payable, frequency, installment_count, first_due_date, recognition_policy, notes, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&aid)
        .bind(&number)
        .bind(ctx.sale_id)
        .bind(ctx.customer_id)
        .bind(ctx.cash_price)
        .bind(ctx.price_total)
        .bind(ctx.down_payment)
        .bind(principal)
        .bind(markup_bp)
        .bind(markup)
        .bind(fees_total)
        .bind(total_payable)
        .bind(&inst.frequency)
        .bind(schedule.len() as i64)
        .bind(first_due.to_string())
        .bind(policy)
        .bind(inst.notes.trim())
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *conn)
        .await?;
    for f in &inst.fees {
        sqlx::query(
            "INSERT INTO installment_charges (id, agreement_id, label, amount) VALUES (?, ?, ?, ?)",
        )
        .bind(Id::new().to_string())
        .bind(&aid)
        .bind(f.label.trim())
        .bind(f.amount)
        .execute(&mut *conn)
        .await?;
    }
    for (i, (d, a)) in schedule.iter().enumerate() {
        sqlx::query("INSERT INTO installment_schedule (id, agreement_id, seq, due_date, amount) VALUES (?, ?, ?, ?, ?)").bind(Id::new().to_string()).bind(&aid).bind(i as i64 + 1).bind(d.to_string()).bind(a).execute(&mut *conn).await?;
    }
    if markup + fees_total > 0 {
        customers::post_ledger(
            conn,
            ctx.customer_id,
            "adjustment",
            markup + fees_total,
            "installment_agreement",
            &aid,
            &format!("{number}: markup and fees"),
            Some(p.user_id),
        )
        .await?;
    }
    post_agreement(conn, &aid).await?;
    audit::log(conn, p, "installment.created", "installment_agreement", &aid, serde_json::json!({"number": number, "sale": ctx.sale_number, "price_total": ctx.price_total, "down_payment": ctx.down_payment, "principal": principal, "markup_bp": markup_bp, "markup": markup, "fees": fees_total, "total_payable": total_payable, "installments": schedule.len(), "frequency": inst.frequency})).await?;
    if markup_bp != default_bp || fees_total > 0 {
        audit::log(conn, p, "installment.markup_set", "installment_agreement", &aid, serde_json::json!({"markup_bp": markup_bp, "default_bp": default_bp, "fees": fees_total})).await?;
    }
    Ok((number, total_payable))
}

// ---------------------------------------------------------------- state

#[derive(Debug, Clone, Serialize)]
pub struct ItemState {
    pub id: String,
    pub seq: i64,
    pub due_date: String,
    pub amount: i64,
    pub late_fee: i64,
    pub paid: i64,
    pub superseded: bool,
    /// What this row obliges the customer to pay: amount + late fee, or (once superseded) only what was already paid.
    pub effective: i64,
    pub remaining: i64,
    /// paid | partial | overdue | pending | superseded
    pub status: String,
    pub days_overdue: i64,
}

async fn item_states(
    conn: &mut SqliteConnection,
    agreement_id: &str,
    today: NaiveDate,
    grace_days: i64,
) -> CoreResult<Vec<ItemState>> {
    let rows: Vec<(String, i64, String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT s.id, s.seq, s.due_date, s.amount, s.superseded,
            COALESCE((SELECT amount FROM installment_late_fees WHERE schedule_id = s.id), 0),
            COALESCE((SELECT SUM(amount) FROM collection_allocations WHERE schedule_id = s.id), 0)
         FROM installment_schedule s WHERE s.agreement_id = ? ORDER BY s.seq",
    )
    .bind(agreement_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = vec![];
    for (id, seq, due, amount, sup, fee, paid) in rows {
        let superseded = sup == 1;
        let effective = if superseded { paid } else { amount + fee };
        let remaining = (effective - paid).max(0);
        let due_d = parse_date("due date", &due)?;
        let overdue_days = (today - (due_d + Duration::days(grace_days))).num_days();
        let status = if superseded {
            "superseded"
        } else if remaining == 0 {
            "paid"
        } else if overdue_days > 0 {
            "overdue"
        } else if paid > 0 {
            "partial"
        } else {
            "pending"
        };
        out.push(ItemState {
            id,
            seq,
            due_date: due,
            amount,
            late_fee: fee,
            paid,
            superseded,
            effective,
            remaining,
            status: status.into(),
            days_overdue: if remaining > 0 && !superseded && overdue_days > 0 {
                overdue_days
            } else {
                0
            },
        });
    }
    Ok(out)
}

async fn totals(
    conn: &mut SqliteConnection,
    agreement_id: &str,
) -> CoreResult<(i64, i64, i64, i64)> {
    // (total_payable, late_fees, payments collected, rebates granted)
    let (t,): (i64,) =
        sqlx::query_as("SELECT total_payable FROM installment_agreements WHERE id = ?")
            .bind(agreement_id)
            .fetch_one(&mut *conn)
            .await?;
    let fees: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0) FROM installment_late_fees WHERE agreement_id = ?",
    )
    .bind(agreement_id)
    .fetch_one(&mut *conn)
    .await?;
    let c: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0) FROM installment_collections WHERE agreement_id = ? AND kind = 'payment'").bind(agreement_id).fetch_one(&mut *conn).await?;
    let r: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0) FROM installment_collections WHERE agreement_id = ? AND kind = 'rebate'").bind(agreement_id).fetch_one(&mut *conn).await?;
    Ok((t, fees, c, r))
}

/// Amount still to be collected.
pub(crate) async fn outstanding(
    conn: &mut SqliteConnection,
    agreement_id: &str,
) -> CoreResult<i64> {
    let (t, f, c, r) = totals(conn, agreement_id).await?;
    Ok(t + f - c - r)
}

async fn sync_status(conn: &mut SqliteConnection, agreement_id: &str) -> CoreResult<()> {
    let o = outstanding(conn, agreement_id).await?;
    sqlx::query("UPDATE installment_agreements SET status = ? WHERE id = ?")
        .bind(if o == 0 { "completed" } else { "active" })
        .bind(agreement_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------- collections

#[derive(Debug, Deserialize)]
pub struct CollectIn {
    pub agreement_id: String,
    pub amount: i64,
    pub method: String,
    pub reference: String,
    pub paid_on: Option<String>,
    /// Pay this schedule row first; the rest follows oldest-first.
    pub target_schedule_id: Option<String>,
    pub client_key: String,
}

#[derive(Debug, Serialize)]
pub struct CollectResult {
    pub receipt: String,
    pub allocations: Vec<(i64, i64)>, // (installment no., amount)
    pub outstanding: i64,
    pub completed: bool,
}

fn check_method(m: &str) -> CoreResult<()> {
    if ["cash", "bank", "mobile_wallet", "cheque", "other"].contains(&m) {
        Ok(())
    } else {
        Err(CoreError::Validation("unknown payment method".into()))
    }
}

/// Transparent, deterministic allocation: the chosen installment first, then oldest due first.
fn plan_allocation(
    states: &[ItemState],
    amount: i64,
    target: Option<&str>,
) -> CoreResult<Vec<(String, i64, i64)>> {
    let mut order: Vec<&ItemState> = states
        .iter()
        .filter(|s| !s.superseded && s.remaining > 0)
        .collect();
    if let Some(t) = target {
        match order.iter().position(|s| s.id == t) {
            Some(i) => {
                let first = order.remove(i);
                order.insert(0, first);
            }
            None => {
                return Err(CoreError::Validation(
                    "the selected installment has nothing left to pay".into(),
                ))
            }
        }
    }
    let mut left = amount;
    let mut plan = vec![];
    for s in order {
        if left == 0 {
            break;
        }
        let part = left.min(s.remaining);
        plan.push((s.id.clone(), s.seq, part));
        left -= part;
    }
    if left != 0 {
        return Err(CoreError::Validation(
            "amount exceeds what is outstanding on this agreement".into(),
        ));
    }
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
async fn write_collection(
    conn: &mut SqliteConnection,
    p: &Principal,
    agreement_id: &str,
    customer_id: &str,
    kind: &str,
    amount: i64,
    method: &str,
    reference: &str,
    paid_on: &str,
    note: &str,
    client_key: Option<&str>,
    reverses_id: Option<&str>,
    plan: &[(String, i64, i64)],
) -> CoreResult<(String, String)> {
    let cid = Id::new().to_string();
    sequence::ensure(conn, "receipt", "RCT-", 6).await?;
    let rno = sequence::next(conn, "receipt").await?;
    sqlx::query("INSERT INTO installment_collections (id, agreement_id, receipt_no, kind, amount, method, paid_on, reference, note, client_key, reverses_id, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&cid)
        .bind(agreement_id)
        .bind(&rno)
        .bind(kind)
        .bind(amount)
        .bind(method)
        .bind(paid_on)
        .bind(reference)
        .bind(note)
        .bind(client_key)
        .bind(reverses_id)
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *conn)
        .await
        .map_err(|e| crate::catalogue::map_unique(e, "reversal of this collection"))?;
    for (sid, _, part) in plan {
        sqlx::query("INSERT INTO collection_allocations (id, collection_id, schedule_id, amount) VALUES (?, ?, ?, ?)").bind(Id::new().to_string()).bind(&cid).bind(sid).bind(part).execute(&mut *conn).await?;
    }
    if kind == "payment" {
        customers::post_ledger(
            conn,
            customer_id,
            "payment",
            -amount,
            "installment_collection",
            &cid,
            &rno,
            Some(p.user_id),
        )
        .await?;
    } else {
        customers::post_ledger(
            conn,
            customer_id,
            "adjustment",
            -amount,
            "installment_collection",
            &cid,
            &format!("{rno}: early-settlement rebate"),
            Some(p.user_id),
        )
        .await?;
    }
    post_collection(conn, &cid).await?;
    Ok((cid, rno))
}

async fn agreement_basics(
    conn: &mut SqliteConnection,
    id: &str,
) -> CoreResult<(String, String, String)> {
    let r: Option<(String, String, String)> = sqlx::query_as(
        "SELECT number, customer_id, status FROM installment_agreements WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    r.ok_or_else(|| CoreError::NotFound("installment agreement".into()))
}

pub async fn collect(pool: &SqlitePool, p: &Principal, c: CollectIn) -> CoreResult<CollectResult> {
    p.require(perm::INSTALLMENTS_COLLECT)?;
    if c.amount <= 0 {
        return Err(CoreError::Validation("amount must be positive".into()));
    }
    check_method(&c.method)?;
    let key = c.client_key.trim();
    if key.is_empty() || key.len() > 60 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    let paid_on = match &c.paid_on {
        Some(d) => parse_date("payment date", d)?.to_string(),
        None => time::today_dhaka().to_string(),
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some((cid, rno)) = sqlx::query_as::<_, (String, String)>(
        "SELECT id, receipt_no FROM installment_collections WHERE client_key = ?",
    )
    .bind(key)
    .fetch_optional(&mut *tx)
    .await?
    {
        let allocs: Vec<(i64, i64)> = sqlx::query_as("SELECT s.seq, a.amount FROM collection_allocations a JOIN installment_schedule s ON s.id = a.schedule_id WHERE a.collection_id = ? ORDER BY s.seq").bind(&cid).fetch_all(&mut *tx).await?;
        let o = outstanding(&mut tx, &c.agreement_id).await?;
        return Ok(CollectResult {
            receipt: rno,
            allocations: allocs,
            outstanding: o,
            completed: o == 0,
        });
    }
    let (number, customer_id, _) = agreement_basics(&mut tx, &c.agreement_id).await?;
    let today = time::today_dhaka();
    let grace = setting_i64(&mut tx, "installment.grace_days").await?;
    let states = item_states(&mut tx, &c.agreement_id, today, grace).await?;
    let o = outstanding(&mut tx, &c.agreement_id).await?;
    if o <= 0 {
        return Err(CoreError::Conflict(format!(
            "{number} is already fully paid"
        )));
    }
    if c.amount > o {
        return Err(CoreError::Validation(format!(
            "amount exceeds the outstanding balance of {}",
            Money::from_paisa(o)
        )));
    }
    let plan = plan_allocation(&states, c.amount, c.target_schedule_id.as_deref())?;
    let (cid, rno) = write_collection(
        &mut tx,
        p,
        &c.agreement_id,
        &customer_id,
        "payment",
        c.amount,
        &c.method,
        c.reference.trim(),
        &paid_on,
        &format!("Collection on {number}"),
        Some(key),
        None,
        &plan,
    )
    .await?;
    sync_status(&mut tx, &c.agreement_id).await?;
    let new_o = outstanding(&mut tx, &c.agreement_id).await?;
    audit::log(&mut tx, p, "installment.collected", "installment_agreement", &c.agreement_id, serde_json::json!({"number": number, "collection": cid, "receipt": rno, "amount": c.amount, "method": c.method, "allocations": plan.iter().map(|x| (x.1, x.2)).collect::<Vec<_>>(), "outstanding": new_o})).await?;
    tx.commit().await?;
    Ok(CollectResult {
        receipt: rno,
        allocations: plan.iter().map(|x| (x.1, x.2)).collect(),
        outstanding: new_o,
        completed: new_o == 0,
    })
}

pub async fn reverse_collection(
    pool: &SqlitePool,
    p: &Principal,
    collection_id: Id,
    reason: &str,
) -> CoreResult<Id> {
    p.require(perm::INSTALLMENTS_REVERSE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to reverse a collection".into(),
        ));
    }
    let oid = collection_id.to_string();
    let mut tx = pool.begin().await?;
    let row: Option<(String, String, i64, String, String, Option<String>)> = sqlx::query_as("SELECT agreement_id, kind, amount, method, receipt_no, reverses_id FROM installment_collections WHERE id = ?").bind(&oid).fetch_optional(&mut *tx).await?;
    let (agreement_id, kind, amount, method, receipt_no, reverses) =
        row.ok_or_else(|| CoreError::NotFound("collection".into()))?;
    if reverses.is_some() || amount <= 0 {
        return Err(CoreError::Conflict("a reversal cannot be reversed".into()));
    }
    let allocs: Vec<(String, i64, i64)> = sqlx::query_as("SELECT a.schedule_id, s.seq, a.amount FROM collection_allocations a JOIN installment_schedule s ON s.id = a.schedule_id WHERE a.collection_id = ? ORDER BY s.seq").bind(&oid).fetch_all(&mut *tx).await?;
    let on_superseded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM collection_allocations a JOIN installment_schedule s ON s.id = a.schedule_id WHERE a.collection_id = ? AND s.superseded = 1").bind(&oid).fetch_one(&mut *tx).await?;
    if on_superseded > 0 {
        return Err(CoreError::Conflict("this collection pre-dates a reschedule and can no longer be reversed; record an authorised adjustment instead".into()));
    }
    let (number, customer_id, _) = agreement_basics(&mut tx, &agreement_id).await?;
    let neg: Vec<(String, i64, i64)> = allocs.iter().map(|(s, q, a)| (s.clone(), *q, -a)).collect();
    let (rid, rno) = write_collection(
        &mut tx,
        p,
        &agreement_id,
        &customer_id,
        &kind,
        -amount,
        &method,
        "",
        &time::today_dhaka().to_string(),
        &format!("Reversal of {receipt_no}: {}", reason.trim()),
        None,
        Some(&oid),
        &neg,
    )
    .await?;
    sync_status(&mut tx, &agreement_id).await?;
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "installment.collection_reversed", entity_type: "installment_collection", entity_id: Some(&oid), detail: Some(serde_json::json!({"agreement": number, "receipt": receipt_no, "amount": amount, "reversal": rno})), reason: Some(reason.trim()) }).await?;
    tx.commit().await?;
    Id::parse(&rid)
}

// ---------------------------------------------------------------- early settlement

#[derive(Debug, Deserialize)]
pub struct SettleIn {
    pub agreement_id: String,
    /// Authorised reduction of unearned markup/fees (needs `installments.reschedule` and a reason).
    pub rebate: i64,
    pub reason: String,
    pub method: String,
    pub reference: String,
    pub client_key: String,
}

#[derive(Debug, Serialize)]
pub struct SettleQuote {
    pub outstanding: i64,
    /// Most that may be waived: markup/fees not yet earned.
    pub max_rebate: i64,
}

async fn max_rebate(conn: &mut SqliteConnection, agreement_id: &str) -> CoreResult<i64> {
    let (u, policy): (i64, String) = sqlx::query_as(
        "SELECT markup + fees_total, recognition_policy FROM installment_agreements WHERE id = ?",
    )
    .bind(agreement_id)
    .fetch_one(&mut *conn)
    .await?;
    let (t, _f, c, r) = totals(conn, agreement_id).await?;
    let unearned = if policy == "on_agreement" {
        u - r
    } else {
        u - earned_markup(u, t, c, r)
    };
    Ok(unearned.max(0))
}

pub async fn settle_quote(
    pool: &SqlitePool,
    p: &Principal,
    agreement_id: Id,
) -> CoreResult<SettleQuote> {
    p.require(perm::INSTALLMENTS_VIEW)?;
    let mut conn = pool.acquire().await?;
    let id = agreement_id.to_string();
    agreement_basics(&mut conn, &id).await?;
    let o = outstanding(&mut conn, &id).await?;
    Ok(SettleQuote {
        outstanding: o,
        max_rebate: max_rebate(&mut conn, &id).await?.min(o - 1).max(0),
    })
}

/// Pays the whole remaining balance (minus an authorised rebate). The agreement closes only at exactly zero.
pub async fn settle_early(
    pool: &SqlitePool,
    p: &Principal,
    s: SettleIn,
) -> CoreResult<CollectResult> {
    p.require(perm::INSTALLMENTS_COLLECT)?;
    check_method(&s.method)?;
    if s.rebate < 0 {
        return Err(CoreError::Validation("rebate cannot be negative".into()));
    }
    if s.rebate > 0 {
        p.require(perm::INSTALLMENTS_RESCHEDULE)?;
        if s.reason.trim().is_empty() {
            return Err(CoreError::Validation(
                "a reason is required to waive part of the balance".into(),
            ));
        }
    }
    let key = s.client_key.trim();
    if key.is_empty() || key.len() > 50 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let pay_key = format!("{key}#pay");
    if let Some((cid, rno)) = sqlx::query_as::<_, (String, String)>(
        "SELECT id, receipt_no FROM installment_collections WHERE client_key = ?",
    )
    .bind(&pay_key)
    .fetch_optional(&mut *tx)
    .await?
    {
        let allocs: Vec<(i64, i64)> = sqlx::query_as("SELECT s.seq, a.amount FROM collection_allocations a JOIN installment_schedule s ON s.id = a.schedule_id WHERE a.collection_id = ? ORDER BY s.seq").bind(&cid).fetch_all(&mut *tx).await?;
        return Ok(CollectResult {
            receipt: rno,
            allocations: allocs,
            outstanding: 0,
            completed: true,
        });
    }
    let (number, customer_id, _) = agreement_basics(&mut tx, &s.agreement_id).await?;
    let o = outstanding(&mut tx, &s.agreement_id).await?;
    if o <= 0 {
        return Err(CoreError::Conflict(format!(
            "{number} is already fully paid"
        )));
    }
    let limit = max_rebate(&mut tx, &s.agreement_id).await?;
    if s.rebate > limit || s.rebate >= o {
        return Err(CoreError::Validation(format!("the rebate cannot exceed the unearned markup and fees ({}) and must leave something to pay", Money::from_paisa(limit.min(o - 1).max(0)))));
    }
    let today = time::today_dhaka();
    let grace = setting_i64(&mut tx, "installment.grace_days").await?;
    if s.rebate > 0 {
        let states = item_states(&mut tx, &s.agreement_id, today, grace).await?;
        let plan = plan_allocation(&states, s.rebate, None)?;
        let (rid, _) = write_collection(
            &mut tx,
            p,
            &s.agreement_id,
            &customer_id,
            "rebate",
            s.rebate,
            "rebate",
            "",
            &today.to_string(),
            &format!("Early-settlement rebate on {number}: {}", s.reason.trim()),
            Some(&format!("{key}#rebate")),
            None,
            &plan,
        )
        .await?;
        audit::record(
            &mut tx,
            audit::AuditEntry {
                actor_id: Some(p.user_id),
                action: "installment.rebate",
                entity_type: "installment_agreement",
                entity_id: Some(&s.agreement_id),
                detail: Some(
                    serde_json::json!({"number": number, "rebate": s.rebate, "collection": rid}),
                ),
                reason: Some(s.reason.trim()),
            },
        )
        .await?;
    }
    let states = item_states(&mut tx, &s.agreement_id, today, grace).await?;
    let pay = o - s.rebate;
    let plan = plan_allocation(&states, pay, None)?;
    let (cid, rno) = write_collection(
        &mut tx,
        p,
        &s.agreement_id,
        &customer_id,
        "payment",
        pay,
        &s.method,
        s.reference.trim(),
        &today.to_string(),
        &format!("Early settlement of {number}"),
        Some(&pay_key),
        None,
        &plan,
    )
    .await?;
    sync_status(&mut tx, &s.agreement_id).await?;
    let left = outstanding(&mut tx, &s.agreement_id).await?;
    if left != 0 {
        return Err(CoreError::Integrity(format!(
            "settlement left {left} outstanding"
        )));
    }
    audit::log(
        &mut tx,
        p,
        "installment.settled_early",
        "installment_agreement",
        &s.agreement_id,
        serde_json::json!({"number": number, "collection": cid, "paid": pay, "rebate": s.rebate}),
    )
    .await?;
    tx.commit().await?;
    Ok(CollectResult {
        receipt: rno,
        allocations: plan.iter().map(|x| (x.1, x.2)).collect(),
        outstanding: 0,
        completed: true,
    })
}

// ---------------------------------------------------------------- rescheduling, late fees, follow-ups

#[derive(Debug, Deserialize)]
pub struct RescheduleIn {
    pub agreement_id: String,
    pub count: i64,
    pub frequency: String,
    pub first_due_date: String,
    #[serde(default)]
    pub custom: Vec<CustomDue>,
    pub reason: String,
}

/// Replaces every unpaid part of the schedule by a new schedule for the outstanding balance.
/// Past payments and past schedule rows are kept (superseded rows count only what was paid on them).
pub async fn reschedule(pool: &SqlitePool, p: &Principal, r: RescheduleIn) -> CoreResult<()> {
    p.require(perm::INSTALLMENTS_RESCHEDULE)?;
    if r.reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to reschedule".into(),
        ));
    }
    let first = parse_date("first due date", &r.first_due_date)?;
    if first < time::today_dhaka() {
        return Err(CoreError::Validation(
            "the new first due date cannot be in the past".into(),
        ));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let (number, _, status) = agreement_basics(&mut tx, &r.agreement_id).await?;
    let o = outstanding(&mut tx, &r.agreement_id).await?;
    if status != "active" || o <= 0 {
        return Err(CoreError::Conflict(format!(
            "{number} has nothing outstanding to reschedule"
        )));
    }
    let new = build_schedule(&r.frequency, first, r.count, &r.custom, o)?;
    let grace = setting_i64(&mut tx, "installment.grace_days").await?;
    let states = item_states(&mut tx, &r.agreement_id, time::today_dhaka(), grace).await?;
    let old_open = states
        .iter()
        .filter(|s| !s.superseded && s.remaining > 0)
        .count() as i64;
    let rid = Id::new().to_string();
    sqlx::query("UPDATE installment_schedule SET superseded = 1, reschedule_id = ? WHERE agreement_id = ? AND id IN (SELECT id FROM installment_schedule WHERE agreement_id = ? AND superseded = 0)")
        .bind(&rid)
        .bind(&r.agreement_id)
        .bind(&r.agreement_id)
        .execute(&mut *tx)
        .await?;
    // fully paid, un-superseded rows must stay as they are (they hold their own payments); undo for them
    for s in states.iter().filter(|s| !s.superseded && s.remaining == 0) {
        sqlx::query(
            "UPDATE installment_schedule SET superseded = 0, reschedule_id = NULL WHERE id = ?",
        )
        .bind(&s.id)
        .execute(&mut *tx)
        .await?;
    }
    let next_seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) FROM installment_schedule WHERE agreement_id = ?",
    )
    .bind(&r.agreement_id)
    .fetch_one(&mut *tx)
    .await?;
    for (i, (d, a)) in new.iter().enumerate() {
        sqlx::query("INSERT INTO installment_schedule (id, agreement_id, seq, due_date, amount, reschedule_id) VALUES (?, ?, ?, ?, ?, ?)").bind(Id::new().to_string()).bind(&r.agreement_id).bind(next_seq + 1 + i as i64).bind(d.to_string()).bind(a).bind(&rid).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO installment_reschedules (id, agreement_id, at, actor_id, reason, outstanding, old_count, new_count) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&rid)
        .bind(&r.agreement_id)
        .bind(time::now_utc_string())
        .bind(p.user_id.to_string())
        .bind(r.reason.trim())
        .bind(o)
        .bind(old_open)
        .bind(new.len() as i64)
        .execute(&mut *tx)
        .await?;
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "installment.rescheduled", entity_type: "installment_agreement", entity_id: Some(&r.agreement_id), detail: Some(serde_json::json!({"number": number, "outstanding": o, "old_open_installments": old_open, "new_installments": new.len(), "frequency": r.frequency})), reason: Some(r.reason.trim()) }).await?;
    tx.commit().await?;
    Ok(())
}

/// Explicit late fee on one overdue installment, only when the shop has enabled late fees.
pub async fn apply_late_fee(
    pool: &SqlitePool,
    p: &Principal,
    agreement_id: Id,
    schedule_id: Id,
    reason: &str,
) -> CoreResult<i64> {
    p.require(perm::INSTALLMENTS_RESCHEDULE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to charge a late fee".into(),
        ));
    }
    let aid = agreement_id.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if setting(&mut tx, "installment.late_fee_enabled").await? != "true" {
        return Err(CoreError::Conflict(
            "late fees are not enabled in Settings".into(),
        ));
    }
    let bp = setting_i64(&mut tx, "installment.late_fee_bp").await?;
    let grace = setting_i64(&mut tx, "installment.grace_days").await?;
    let (number, customer_id, _) = agreement_basics(&mut tx, &aid).await?;
    let states = item_states(&mut tx, &aid, time::today_dhaka(), grace).await?;
    let item = states
        .iter()
        .find(|s| s.id == schedule_id.to_string())
        .ok_or_else(|| CoreError::NotFound("installment".into()))?;
    if item.status != "overdue" {
        return Err(CoreError::Conflict(
            "a late fee can only be charged on an overdue installment (after the grace period)"
                .into(),
        ));
    }
    if item.late_fee > 0 {
        return Err(CoreError::Conflict(
            "a late fee was already charged on this installment".into(),
        ));
    }
    let fee = Money::from_paisa(item.remaining).percent_bp(bp)?.paisa();
    if fee <= 0 {
        return Err(CoreError::Validation(
            "the configured late-fee rate produces no fee".into(),
        ));
    }
    let fid = Id::new().to_string();
    sqlx::query("INSERT INTO installment_late_fees (id, agreement_id, schedule_id, amount, reason, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)").bind(&fid).bind(&aid).bind(schedule_id.to_string()).bind(fee).bind(reason.trim()).bind(p.user_id.to_string()).bind(time::now_utc_string()).execute(&mut *tx).await?;
    customers::post_ledger(
        &mut tx,
        &customer_id,
        "adjustment",
        fee,
        "installment_late_fee",
        &fid,
        &format!("{number}: late fee on installment {}", item.seq),
        Some(p.user_id),
    )
    .await?;
    post_late_fee(&mut tx, &fid).await?;
    sync_status(&mut tx, &aid).await?;
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "installment.late_fee", entity_type: "installment_agreement", entity_id: Some(&aid), detail: Some(serde_json::json!({"number": number, "installment": item.seq, "fee": fee, "rate_bp": bp})), reason: Some(reason.trim()) }).await?;
    tx.commit().await?;
    Ok(fee)
}

pub async fn add_followup(
    pool: &SqlitePool,
    p: &Principal,
    agreement_id: Id,
    note: &str,
    promise_date: Option<&str>,
) -> CoreResult<()> {
    p.require(perm::INSTALLMENTS_COLLECT)?;
    let note = note.trim();
    if note.is_empty() || note.chars().count() > 500 {
        return Err(CoreError::Validation(
            "a note (max 500 characters) is required".into(),
        ));
    }
    let promise = promise_date
        .filter(|s| !s.trim().is_empty())
        .map(|d| parse_date("promise date", d).map(|x| x.to_string()))
        .transpose()?;
    let mut tx = pool.begin().await?;
    agreement_basics(&mut tx, &agreement_id.to_string()).await?;
    sqlx::query("INSERT INTO installment_followups (id, agreement_id, at, actor_id, note, promise_date) VALUES (?, ?, ?, ?, ?, ?)").bind(Id::new().to_string()).bind(agreement_id.to_string()).bind(time::now_utc_string()).bind(p.user_id.to_string()).bind(note).bind(&promise).execute(&mut *tx).await?;
    audit::log(
        &mut tx,
        p,
        "installment.followup",
        "installment_agreement",
        &agreement_id.to_string(),
        serde_json::json!({"promise_date": promise}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------- posting

pub(crate) async fn post_agreement(
    conn: &mut SqliteConnection,
    agreement_id: &str,
) -> CoreResult<()> {
    let r: Option<(String, String, i64, i64, i64, String)> = sqlx::query_as("SELECT a.number, s.sale_date, a.principal, a.markup + a.fees_total, a.total_payable, a.recognition_policy FROM installment_agreements a JOIN sales s ON s.id = a.sale_id WHERE a.id = ?").bind(agreement_id).fetch_optional(&mut *conn).await?;
    let Some((number, date, principal, unearned, total, policy)) = r else {
        return Ok(());
    };
    let key = format!("installment_agreement:{agreement_id}");
    let memo = format!("Installment agreement {number}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &memo,
            source_type: "installment_agreement",
            source_id: agreement_id,
            lines: vec![
                dr(code::INSTALLMENT_AR, total, "Financed amount"),
                cr(code::AR, principal, "Moved from receivables"),
                cr(
                    code::UNEARNED_MARKUP,
                    unearned,
                    "Markup and fees not yet earned",
                ),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    if policy == "on_agreement" {
        let k2 = format!("installment_markup:{agreement_id}");
        accounting::post(
            conn,
            &PostIn {
                key: &k2,
                date: &date,
                memo: &format!("Markup recognised at signing {number}"),
                source_type: "installment_markup",
                source_id: agreement_id,
                lines: vec![
                    dr(code::UNEARNED_MARKUP, unearned, "Recognised"),
                    cr(code::MARKUP_INCOME, unearned, "Installment markup income"),
                ],
                reverses: None,
                actor: None,
            },
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn post_collection(
    conn: &mut SqliteConnection,
    collection_id: &str,
) -> CoreResult<()> {
    type CollRow = (
        String,
        String,
        i64,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        String,
    );
    let r: Option<CollRow> = sqlx::query_as(
        "SELECT c.agreement_id, c.kind, c.amount, c.method, c.paid_on, a.number, a.markup + a.fees_total, a.total_payable, c.rowid, a.recognition_policy
         FROM installment_collections c JOIN installment_agreements a ON a.id = c.agreement_id WHERE c.id = ?",
    )
    .bind(collection_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((agreement_id, kind, amount, method, date, number, u, t, rowid, policy)) = r else {
        return Ok(());
    };
    let key = format!("installment_collection:{collection_id}");
    let abs = amount.abs();
    let lines = if kind == "payment" {
        let acct = method_account(&method);
        if amount > 0 {
            vec![
                dr(acct, abs, "Money in"),
                cr(code::INSTALLMENT_AR, abs, "Installment collected"),
            ]
        } else {
            vec![
                dr(code::INSTALLMENT_AR, abs, "Collection reversed"),
                cr(acct, abs, "Money returned"),
            ]
        }
    } else {
        // rebate: waives unearned markup/fees (or income already recognised at signing)
        let contra = if policy == "on_agreement" {
            code::MARKUP_INCOME
        } else {
            code::UNEARNED_MARKUP
        };
        if amount > 0 {
            vec![
                dr(contra, abs, "Rebate granted"),
                cr(code::INSTALLMENT_AR, abs, "Balance waived"),
            ]
        } else {
            vec![
                dr(code::INSTALLMENT_AR, abs, "Rebate reversed"),
                cr(contra, abs, "Rebate reversed"),
            ]
        }
    };
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &format!(
                "Installment {} {}",
                if amount > 0 { "receipt" } else { "reversal" },
                number
            ),
            source_type: "installment_collection",
            source_id: collection_id,
            lines,
            reverses: None,
            actor: None,
        },
    )
    .await?;
    // markup recognition (proportional policy): the change in cumulative earned markup caused by this row
    recognise(
        conn,
        &agreement_id,
        collection_id,
        rowid,
        u,
        t,
        &date,
        &number,
        &policy,
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn recognise(
    conn: &mut SqliteConnection,
    agreement_id: &str,
    collection_id: &str,
    rowid: i64,
    u: i64,
    t: i64,
    date: &str,
    number: &str,
    policy: &str,
) -> CoreResult<()> {
    if policy == "on_agreement" || u == 0 {
        return Ok(());
    }
    let sum = |kind: &'static str, upto: i64| {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(amount), 0) FROM installment_collections WHERE agreement_id = ? AND kind = ? AND rowid <= ?").bind(agreement_id.to_string()).bind(kind).bind(upto)
    };
    let c_after = sum("payment", rowid).fetch_one(&mut *conn).await?;
    let r_after = sum("rebate", rowid).fetch_one(&mut *conn).await?;
    let c_before = sum("payment", rowid - 1).fetch_one(&mut *conn).await?;
    let r_before = sum("rebate", rowid - 1).fetch_one(&mut *conn).await?;
    let delta = earned_markup(u, t, c_after, r_after) - earned_markup(u, t, c_before, r_before);
    if delta == 0 {
        return Ok(());
    }
    let key = format!("installment_recognition:{collection_id}");
    let lines = if delta > 0 {
        vec![
            dr(code::UNEARNED_MARKUP, delta, "Earned"),
            cr(code::MARKUP_INCOME, delta, "Installment markup income"),
        ]
    } else {
        vec![
            dr(code::MARKUP_INCOME, -delta, "Earning reversed"),
            cr(code::UNEARNED_MARKUP, -delta, "Unearned again"),
        ]
    };
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date,
            memo: &format!("Markup earned on {number}"),
            source_type: "installment_recognition",
            source_id: collection_id,
            lines,
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_late_fee(conn: &mut SqliteConnection, fee_id: &str) -> CoreResult<()> {
    let r: Option<(String, i64, String)> = sqlx::query_as("SELECT a.number, f.amount, f.created_at FROM installment_late_fees f JOIN installment_agreements a ON a.id = f.agreement_id WHERE f.id = ?").bind(fee_id).fetch_optional(&mut *conn).await?;
    let Some((number, amount, at)) = r else {
        return Ok(());
    };
    let key = format!("installment_late_fee:{fee_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &accounting::dhaka_date(&at),
            memo: &format!("Late fee on {number}"),
            source_type: "installment_late_fee",
            source_id: fee_id,
            lines: vec![
                dr(code::INSTALLMENT_AR, amount, "Late fee charged"),
                cr(code::OTHER_INCOME, amount, "Late fee income"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn ensure_posted(pool: &SqlitePool) -> CoreResult<u64> {
    let mut n = 0;
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM installment_agreements WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'installment_agreement:' || installment_agreements.id)").fetch_all(pool).await?;
    for id in ids {
        let mut tx = pool.begin().await?;
        post_agreement(&mut tx, &id).await?;
        tx.commit().await?;
        n += 1;
    }
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM installment_collections WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'installment_collection:' || installment_collections.id) ORDER BY rowid").fetch_all(pool).await?;
    for id in ids {
        let mut tx = pool.begin().await?;
        post_collection(&mut tx, &id).await?;
        tx.commit().await?;
        n += 1;
    }
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM installment_late_fees WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'installment_late_fee:' || installment_late_fees.id)").fetch_all(pool).await?;
    for id in ids {
        let mut tx = pool.begin().await?;
        post_late_fee(&mut tx, &id).await?;
        tx.commit().await?;
        n += 1;
    }
    Ok(n)
}

// ---------------------------------------------------------------- queries

#[derive(Debug, Serialize)]
pub struct AgreementRow {
    pub id: String,
    pub number: String,
    pub sale_id: String,
    pub sale_number: String,
    pub customer_id: String,
    pub customer: String,
    pub customer_phone: String,
    pub status: String,
    pub down_payment: i64,
    /// Contract total still governed by the schedule: total payable + late fees.
    pub total_payable: i64,
    pub collected: i64,
    pub outstanding: i64,
    pub next_due_date: Option<String>,
    pub next_due_amount: i64,
    pub overdue_amount: i64,
    pub max_days_overdue: i64,
    pub installment_count: i64,
    pub created_on: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct AgreementFilter {
    pub search: Option<String>,
    pub status: Option<String>,
    pub customer_id: Option<String>,
    pub overdue_only: bool,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AgreementPage {
    pub total: i64,
    pub overdue_total: i64,
    pub outstanding_total: i64,
    pub rows: Vec<AgreementRow>,
}

type AgRaw = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    String,
);

const AG_SELECT: &str = "SELECT a.id, a.number, a.sale_id, s.number, a.customer_id, c.name, c.phone, a.status, a.down_payment, a.total_payable, a.installment_count, s.sale_date
    FROM installment_agreements a JOIN sales s ON s.id = a.sale_id JOIN customers c ON c.id = a.customer_id";

async fn build_row(
    conn: &mut SqliteConnection,
    r: AgRaw,
    today: NaiveDate,
    grace: i64,
) -> CoreResult<AgreementRow> {
    let states = item_states(conn, &r.0, today, grace).await?;
    let (t, fees, c, rb) = totals(conn, &r.0).await?;
    let open: Vec<&ItemState> = states
        .iter()
        .filter(|s| !s.superseded && s.remaining > 0)
        .collect();
    let next = open.first();
    Ok(AgreementRow {
        outstanding: t + fees - c - rb,
        collected: c + rb,
        total_payable: t + fees,
        next_due_date: next.map(|s| s.due_date.clone()),
        next_due_amount: next.map(|s| s.remaining).unwrap_or(0),
        overdue_amount: open
            .iter()
            .filter(|s| s.status == "overdue")
            .map(|s| s.remaining)
            .sum(),
        max_days_overdue: open.iter().map(|s| s.days_overdue).max().unwrap_or(0),
        id: r.0,
        number: r.1,
        sale_id: r.2,
        sale_number: r.3,
        customer_id: r.4,
        customer: r.5,
        customer_phone: r.6,
        status: r.7,
        down_payment: r.8,
        installment_count: r.10,
        created_on: r.11,
    })
}

pub async fn list_agreements(
    pool: &SqlitePool,
    p: &Principal,
    f: AgreementFilter,
) -> CoreResult<AgreementPage> {
    p.require(perm::INSTALLMENTS_VIEW)?;
    let like = f.search.as_deref().map(crate::catalogue::like_pattern);
    let raws: Vec<AgRaw> = sqlx::query_as(&format!(
        "{AG_SELECT} WHERE (?1 IS NULL OR a.number LIKE ?1 OR c.name LIKE ?1 OR c.phone LIKE ?1 OR s.number LIKE ?1) AND (?2 IS NULL OR a.status = ?2) AND (?3 IS NULL OR a.customer_id = ?3)
         ORDER BY a.number DESC LIMIT 1000"
    ))
    .bind(&like)
    .bind(&f.status)
    .bind(&f.customer_id)
    .fetch_all(pool)
    .await?;
    let mut conn = pool.acquire().await?;
    let grace = setting_i64(&mut conn, "installment.grace_days").await?;
    let today = time::today_dhaka();
    let mut rows = vec![];
    for r in raws {
        let row = build_row(&mut conn, r, today, grace).await?;
        if !f.overdue_only || row.overdue_amount > 0 {
            rows.push(row);
        }
    }
    let overdue_total = rows.iter().map(|r| r.overdue_amount).sum();
    let outstanding_total = rows.iter().map(|r| r.outstanding).sum();
    let total = rows.len() as i64;
    let off = f.offset.unwrap_or(0).max(0) as usize;
    let lim = f.limit.unwrap_or(100).clamp(1, 500) as usize;
    let rows = rows.into_iter().skip(off).take(lim).collect();
    Ok(AgreementPage {
        total,
        overdue_total,
        outstanding_total,
        rows,
    })
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ChargeRow {
    pub label: String,
    pub amount: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CollectionRow {
    pub id: String,
    pub receipt_no: String,
    pub kind: String,
    pub amount: i64,
    pub method: String,
    pub paid_on: String,
    pub reference: String,
    pub note: String,
    pub reverses_id: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FollowupRow {
    pub at: String,
    pub note: String,
    pub promise_date: Option<String>,
    pub actor: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RescheduleRow {
    pub at: String,
    pub reason: String,
    pub outstanding: i64,
    pub old_count: i64,
    pub new_count: i64,
}

#[derive(Debug, Serialize)]
pub struct AgreementDetail {
    pub agreement: AgreementRow,
    pub cash_price: i64,
    pub price_total: i64,
    pub principal: i64,
    pub markup_bp: i64,
    pub markup: i64,
    pub fees: Vec<ChargeRow>,
    pub late_fees_total: i64,
    pub rebates: i64,
    pub frequency: String,
    pub first_due_date: String,
    pub recognition_policy: String,
    pub earned_markup: i64,
    pub unearned_markup: i64,
    pub notes: String,
    pub items: Vec<ItemState>,
    pub collections: Vec<CollectionRow>,
    pub reschedules: Vec<RescheduleRow>,
    pub followups: Vec<FollowupRow>,
    pub grace_days: i64,
}

pub async fn get_agreement(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
) -> CoreResult<AgreementDetail> {
    p.require(perm::INSTALLMENTS_VIEW)?;
    let aid = id.to_string();
    let raw: AgRaw = sqlx::query_as(&format!("{AG_SELECT} WHERE a.id = ?"))
        .bind(&aid)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("installment agreement".into()))?;
    let mut conn = pool.acquire().await?;
    let grace = setting_i64(&mut conn, "installment.grace_days").await?;
    let today = time::today_dhaka();
    let agreement = build_row(&mut conn, raw, today, grace).await?;
    let (cash_price, price_total, principal, markup_bp, markup, frequency, first_due, policy, notes, fees_total): (i64, i64, i64, i64, i64, String, String, String, String, i64) =
        sqlx::query_as("SELECT cash_price, price_total, principal, markup_bp, markup, frequency, first_due_date, recognition_policy, notes, fees_total FROM installment_agreements WHERE id = ?").bind(&aid).fetch_one(&mut *conn).await?;
    let (t, lf, c, r) = totals(&mut conn, &aid).await?;
    let u = markup + fees_total;
    let earned = if policy == "on_agreement" {
        u - r
    } else {
        earned_markup(u, t, c, r)
    };
    let unearned_left = u - earned - if policy == "on_agreement" { 0 } else { r };
    Ok(AgreementDetail {
        agreement,
        cash_price,
        price_total,
        principal,
        markup_bp,
        markup,
        fees: sqlx::query_as("SELECT label, amount FROM installment_charges WHERE agreement_id = ? ORDER BY label").bind(&aid).fetch_all(&mut *conn).await?,
        late_fees_total: lf,
        rebates: r,
        frequency,
        first_due_date: first_due,
        recognition_policy: policy,
        earned_markup: earned,
        unearned_markup: unearned_left,
        notes,
        items: item_states(&mut conn, &aid, today, grace).await?,
        collections: sqlx::query_as("SELECT id, receipt_no, kind, amount, method, paid_on, reference, note, reverses_id FROM installment_collections WHERE agreement_id = ? ORDER BY rowid").bind(&aid).fetch_all(&mut *conn).await?,
        reschedules: sqlx::query_as("SELECT at, reason, outstanding, old_count, new_count FROM installment_reschedules WHERE agreement_id = ? ORDER BY at").bind(&aid).fetch_all(&mut *conn).await?,
        followups: sqlx::query_as("SELECT f.at, f.note, f.promise_date, u.username AS actor FROM installment_followups f LEFT JOIN users u ON u.id = f.actor_id WHERE f.agreement_id = ? ORDER BY f.at DESC").bind(&aid).fetch_all(&mut *conn).await?,
        grace_days: grace,
    })
}

/// Agreements whose unpaid installments are past due (the follow-up worklist), most overdue first.
pub async fn overdue_worklist(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<AgreementRow>> {
    let mut page = list_agreements(
        pool,
        p,
        AgreementFilter {
            overdue_only: true,
            status: Some("active".into()),
            limit: Some(500),
            ..Default::default()
        },
    )
    .await?;
    page.rows.sort_by(|a, b| {
        b.max_days_overdue
            .cmp(&a.max_days_overdue)
            .then(b.overdue_amount.cmp(&a.overdue_amount))
    });
    Ok(page.rows)
}

/// Sum of what the schedule says is still owed (used by reconciliation).
pub(crate) async fn expected_receivable(pool: &SqlitePool) -> CoreResult<(i64, i64)> {
    let mut conn = pool.acquire().await?;
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM installment_agreements")
        .fetch_all(&mut *conn)
        .await?;
    let (mut receivable, mut unearned) = (0i64, 0i64);
    for id in ids {
        receivable += outstanding(&mut conn, &id).await?;
        let (u, policy): (i64, String) = sqlx::query_as("SELECT markup + fees_total, recognition_policy FROM installment_agreements WHERE id = ?").bind(&id).fetch_one(&mut *conn).await?;
        let (t, _f, c, r) = totals(&mut conn, &id).await?;
        unearned += if policy == "on_agreement" {
            0
        } else {
            u - earned_markup(u, t, c, r) - r
        };
    }
    Ok((receivable, unearned))
}

/// Schedule invariant: the rows' effective obligations always add up to total payable + late fees.
pub(crate) async fn schedule_mismatches(pool: &SqlitePool) -> CoreResult<Vec<(String, i64, i64)>> {
    let mut conn = pool.acquire().await?;
    let ids: Vec<(String, String)> =
        sqlx::query_as("SELECT id, number FROM installment_agreements")
            .fetch_all(&mut *conn)
            .await?;
    let mut out = vec![];
    for (id, number) in ids {
        let states = item_states(&mut conn, &id, time::today_dhaka(), 0).await?;
        let sum: i64 = states.iter().map(|s| s.effective).sum();
        let (t, f, _, _) = totals(&mut conn, &id).await?;
        if sum != t + f {
            out.push((number, t + f, sum));
        }
    }
    Ok(out)
}
