//! Sales returns with mandatory inspection.
//!
//! Flow: `record_return` (returns.process) takes the goods back into the *pending inspection* bucket only -
//! nothing becomes sellable and no refund happens. `approve_return` (returns.approve) inspects each unit
//! (restock / damaged / write-off), issues the credit note and any refund. `reject_return` hands the goods back.
//! Returned serial units can never reach `available` without passing through this approval.

use crate::accounting::{self, code, cr, dr, method_account, PostIn};
use crate::audit;
use crate::authz::{perm, Principal};
use crate::customers;
use crate::stock::{self, Bucket, MovementIn, SerialStatus};
use crate::{sequence, time, CoreError, CoreResult, Id, Money};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize)]
pub struct ReturnLineIn {
    pub sale_item_id: String,
    pub qty: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordReturnIn {
    pub sale_id: String,
    pub reason: String,
    pub lines: Vec<ReturnLineIn>,
    pub client_key: String,
}

fn proportional(total: i64, qty: i64, of: i64) -> CoreResult<i64> {
    // total * qty / of, rounded half away from zero, exact integer math
    let num = (total as i128) * (qty as i128);
    let q = (num.abs() * 2 + of as i128) / (of as i128 * 2);
    i64::try_from(q * if num < 0 { -1 } else { 1 }).map_err(|_| CoreError::Overflow)
}

pub async fn record_return(pool: &SqlitePool, p: &Principal, r: RecordReturnIn) -> CoreResult<Id> {
    p.require(perm::RETURNS_PROCESS)?;
    let key = r.client_key.trim();
    if key.is_empty() || key.len() > 64 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    if r.reason.trim().is_empty() || r.reason.chars().count() > 300 {
        return Err(CoreError::Validation(
            "a reason (max 300 characters) is required".into(),
        ));
    }
    if r.lines.is_empty() {
        return Err(CoreError::Validation(
            "select at least one item to return".into(),
        ));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(existing) =
        sqlx::query_scalar::<_, String>("SELECT id FROM sale_returns WHERE client_key = ?")
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?
    {
        return Id::parse(&existing);
    }
    let sale: Option<(String, Option<String>, String)> =
        sqlx::query_as("SELECT mode, customer_id, number FROM sales WHERE id = ?")
            .bind(&r.sale_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (mode, customer, sale_number) = sale.ok_or_else(|| CoreError::NotFound("sale".into()))?;
    if mode == "installment" {
        return Err(CoreError::Conflict("returns of installment sales (agreement cancellation) are not supported in this release".into()));
    }
    sequence::ensure(&mut tx, "return", "RET-", 6).await?;
    let number = sequence::next(&mut tx, "return").await?;
    let rid = Id::new().to_string();
    sqlx::query("INSERT INTO sale_returns (id, number, sale_id, customer_id, return_date, reason, client_key, requested_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&rid)
        .bind(&number)
        .bind(&r.sale_id)
        .bind(&customer)
        .bind(time::today_dhaka().to_string())
        .bind(r.reason.trim())
        .bind(key)
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await?;
    let mut seen = HashSet::new();
    let mut summary = vec![];
    for l in &r.lines {
        if l.qty <= 0 || !seen.insert(l.sale_item_id.clone()) {
            return Err(CoreError::Validation(
                "each returned line needs a positive quantity and may appear once".into(),
            ));
        }
        type ItemRaw = (String, Option<String>, i64, i64, i64, i64);
        let it: Option<ItemRaw> = sqlx::query_as("SELECT variant_id, serial_id, qty, line_total, tax, cost_total FROM sale_items WHERE id = ? AND sale_id = ?").bind(&l.sale_item_id).bind(&r.sale_id).fetch_optional(&mut *tx).await?;
        let (variant, serial, item_qty, line_total, tax, cost) =
            it.ok_or_else(|| CoreError::Validation("an item does not belong to this sale".into()))?;
        let (prev_qty, prev_value, prev_tax, prev_cost): (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(ri.qty), 0), COALESCE(SUM(ri.value), 0), COALESCE(SUM(ri.tax), 0), COALESCE(SUM(ri.cost_total), 0) FROM sale_return_items ri JOIN sale_returns sr ON sr.id = ri.return_id WHERE ri.sale_item_id = ? AND sr.status IN ('requested', 'completed')",
        )
        .bind(&l.sale_item_id)
        .fetch_one(&mut *tx)
        .await?;
        let left = item_qty - prev_qty;
        if l.qty > left {
            return Err(CoreError::Conflict(format!(
                "only {left} unit(s) of this line can still be returned"
            )));
        }
        // the last units take the exact remainder, so returned values always add up to the invoice line
        let (value, vtax, vcost) = if l.qty == left {
            (line_total - prev_value, tax - prev_tax, cost - prev_cost)
        } else {
            (
                proportional(line_total, l.qty, item_qty)?,
                proportional(tax, l.qty, item_qty)?,
                proportional(cost, l.qty, item_qty)?,
            )
        };
        let variant_id = Id::parse(&variant)?;
        let reason = format!("Returned on {number} (awaiting inspection)");
        let serial_id = match &serial {
            Some(sid) => {
                if l.qty != 1 {
                    return Err(CoreError::Validation(
                        "a serial-tracked unit is returned one at a time".into(),
                    ));
                }
                let status: String =
                    sqlx::query_scalar("SELECT status FROM product_serials WHERE id = ?")
                        .bind(sid)
                        .fetch_one(&mut *tx)
                        .await?;
                if status != "sold" {
                    return Err(CoreError::Conflict(format!(
                        "this unit is {} and cannot be returned",
                        status.replace('_', " ")
                    )));
                }
                stock::transition_serial(
                    &mut tx,
                    Id::parse(sid)?,
                    SerialStatus::ReturnedPendingInspection,
                    "sale_return",
                    &rid,
                    &reason,
                    Some(p.user_id),
                )
                .await?;
                Some(sid.clone())
            }
            None => {
                stock::post_movement(
                    &mut tx,
                    &MovementIn {
                        variant_id,
                        bucket: Bucket::PendingInspection,
                        qty_delta: l.qty,
                        cost_total: vcost,
                        serial_id: None,
                        doc_type: "sale_return",
                        doc_id: &rid,
                        reason: &reason,
                        actor: Some(p.user_id),
                        allow_negative: false,
                    },
                )
                .await?;
                None
            }
        };
        sqlx::query("INSERT INTO sale_return_items (id, return_id, sale_item_id, variant_id, serial_id, qty, value, tax, cost_total) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(Id::new().to_string())
            .bind(&rid)
            .bind(&l.sale_item_id)
            .bind(&variant)
            .bind(&serial_id)
            .bind(l.qty)
            .bind(value)
            .bind(vtax)
            .bind(vcost)
            .execute(&mut *tx)
            .await?;
        summary
            .push(serde_json::json!({"sale_item": l.sale_item_id, "qty": l.qty, "value": value}));
    }
    post_goods_in(&mut tx, &rid).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "return.requested",
            entity_type: "sale_return",
            entity_id: Some(&rid),
            detail: Some(
                serde_json::json!({"number": number, "sale": sale_number, "lines": summary}),
            ),
            reason: Some(r.reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Id::parse(&rid)
}

#[derive(Debug, Clone, Deserialize)]
pub struct DecisionIn {
    pub item_id: String,
    /// restock | damaged | write_off
    pub outcome: String,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApproveIn {
    pub decisions: Vec<DecisionIn>,
    /// Cash handed back now (0 = credit only). Limited to what the customer has overpaid after this return.
    pub refund_amount: i64,
    pub refund_method: String,
    pub note: String,
    pub client_key: String,
}

#[derive(Debug, Serialize)]
pub struct ApproveResult {
    pub value: i64,
    pub refund_receipt: Option<String>,
}

async fn return_totals(
    conn: &mut SqliteConnection,
    sale_id: &str,
    upto_return: &str,
) -> CoreResult<(i64, i64, i64)> {
    // (grand_total, paid net of refunds, value of already-completed returns excluding `upto_return`)
    let (grand,): (i64,) = sqlx::query_as("SELECT grand_total FROM sales WHERE id = ?")
        .bind(sale_id)
        .fetch_one(&mut *conn)
        .await?;
    let paid: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT SUM(amount) FROM sale_payments WHERE sale_id = ?1), 0) - COALESCE((SELECT SUM(amount) FROM sale_refunds WHERE sale_id = ?1), 0)").bind(sale_id).fetch_one(&mut *conn).await?;
    let returned: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(ri.value), 0) FROM sale_return_items ri JOIN sale_returns r ON r.id = ri.return_id WHERE r.sale_id = ? AND r.status = 'completed' AND r.id <> ?").bind(sale_id).bind(upto_return).fetch_one(&mut *conn).await?;
    Ok((grand, paid, returned))
}

pub async fn approve_return(
    pool: &SqlitePool,
    p: &Principal,
    return_id: Id,
    a: ApproveIn,
) -> CoreResult<ApproveResult> {
    p.require(perm::RETURNS_APPROVE)?;
    let rid = return_id.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let head: Option<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT number, sale_id, customer_id, status FROM sale_returns WHERE id = ?",
    )
    .bind(&rid)
    .fetch_optional(&mut *tx)
    .await?;
    let (number, sale_id, customer, status) =
        head.ok_or_else(|| CoreError::NotFound("return".into()))?;
    if status != "requested" {
        return Err(CoreError::Conflict(format!(
            "this return is already {status}"
        )));
    }
    type ItemRaw = (String, String, Option<String>, i64, i64, i64, i64);
    let items: Vec<ItemRaw> = sqlx::query_as("SELECT id, variant_id, serial_id, qty, value, tax, cost_total FROM sale_return_items WHERE return_id = ? ORDER BY rowid").bind(&rid).fetch_all(&mut *tx).await?;
    let ids: HashSet<&str> = items.iter().map(|i| i.0.as_str()).collect();
    let decided: HashSet<&str> = a.decisions.iter().map(|d| d.item_id.as_str()).collect();
    if ids != decided || a.decisions.len() != items.len() {
        return Err(CoreError::Validation(
            "inspect every returned item: choose an outcome for each".into(),
        ));
    }
    if a.refund_amount < 0 {
        return Err(CoreError::Validation("refund cannot be negative".into()));
    }
    let methods = ["cash", "bank", "mobile_wallet", "cheque", "other"];
    if a.refund_amount > 0 && !methods.contains(&a.refund_method.as_str()) {
        return Err(CoreError::Validation(
            "choose how the refund is paid".into(),
        ));
    }
    let key = a.client_key.trim();
    if key.is_empty() || key.len() > 64 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    if a.refund_amount > 0 && a.refund_method == "cash" {
        crate::cash::require_open_if_configured(&mut tx).await?;
    }
    let total_value: i64 = items.iter().map(|i| i.4).sum();
    let (grand, paid_net, prior_returned) = return_totals(&mut tx, &sale_id, &rid).await?;
    let remaining_invoice = grand - prior_returned - total_value;
    let max_refund = (paid_net - remaining_invoice).max(0);
    if a.refund_amount > max_refund {
        return Err(CoreError::Validation(format!(
            "refund exceeds what the customer has overpaid after this return ({})",
            Money::from_paisa(max_refund)
        )));
    }
    if customer.is_none() && a.refund_amount != max_refund {
        return Err(CoreError::Validation(format!(
            "this sale has no customer account to hold a credit, so the full amount ({}) must be refunded",
            Money::from_paisa(max_refund)
        )));
    }
    // inspection outcomes move stock out of the pending bucket
    let mut written_off_cost = 0i64;
    let mut outcomes = vec![];
    for it in &items {
        let d = a
            .decisions
            .iter()
            .find(|d| d.item_id == it.0)
            .expect("checked");
        let variant = Id::parse(&it.1)?;
        let reason = format!("Inspection of {number}: {}", d.outcome);
        let (target_serial, target_bucket) = match d.outcome.as_str() {
            "restock" => (Some(SerialStatus::Available), Some(Bucket::Available)),
            "damaged" => (Some(SerialStatus::Damaged), Some(Bucket::Damaged)),
            "write_off" => (Some(SerialStatus::WrittenOff), None),
            _ => {
                return Err(CoreError::Validation(
                    "outcome must be restock, damaged or write_off".into(),
                ))
            }
        };
        if d.outcome == "write_off" && d.note.trim().is_empty() {
            return Err(CoreError::Validation(
                "a note is required to write a unit off".into(),
            ));
        }
        if let Some(sid) = &it.2 {
            stock::transition_serial(
                &mut tx,
                Id::parse(sid)?,
                target_serial.expect("set"),
                "sale_return",
                &rid,
                &reason,
                Some(p.user_id),
            )
            .await?;
        } else {
            stock::post_movement(
                &mut tx,
                &MovementIn {
                    variant_id: variant,
                    bucket: Bucket::PendingInspection,
                    qty_delta: -it.3,
                    cost_total: -it.6,
                    serial_id: None,
                    doc_type: "sale_return",
                    doc_id: &rid,
                    reason: &reason,
                    actor: Some(p.user_id),
                    allow_negative: false,
                },
            )
            .await?;
            if let Some(b) = target_bucket {
                stock::post_movement(
                    &mut tx,
                    &MovementIn {
                        variant_id: variant,
                        bucket: b,
                        qty_delta: it.3,
                        cost_total: it.6,
                        serial_id: None,
                        doc_type: "sale_return",
                        doc_id: &rid,
                        reason: &reason,
                        actor: Some(p.user_id),
                        allow_negative: false,
                    },
                )
                .await?;
            }
        }
        if d.outcome == "write_off" {
            written_off_cost += it.6;
        }
        sqlx::query("UPDATE sale_return_items SET outcome = ?, note = ? WHERE id = ?")
            .bind(&d.outcome)
            .bind(d.note.trim())
            .bind(&it.0)
            .execute(&mut *tx)
            .await?;
        outcomes.push(serde_json::json!({"item": it.0, "outcome": d.outcome}));
    }
    sqlx::query("UPDATE sale_returns SET status = 'completed', decision_note = ?, decided_by = ?, decided_at = ? WHERE id = ?").bind(a.note.trim()).bind(p.user_id.to_string()).bind(time::now_utc_string()).bind(&rid).execute(&mut *tx).await?;
    if let Some(cid) = &customer {
        customers::post_ledger(
            &mut tx,
            cid,
            "return",
            -total_value,
            "sale_return",
            &rid,
            &format!("{number}: credit note"),
            Some(p.user_id),
        )
        .await?;
    }
    post_credit_note(&mut tx, &rid).await?;
    if written_off_cost > 0 {
        post_write_off(&mut tx, &rid).await?;
    }
    let mut refund_receipt = None;
    if a.refund_amount > 0 {
        sequence::ensure(&mut tx, "receipt", "RCT-", 6).await?;
        let rno = sequence::next(&mut tx, "receipt").await?;
        let fid = Id::new().to_string();
        sqlx::query("INSERT INTO sale_refunds (id, return_id, sale_id, receipt_no, amount, method, paid_on, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&fid)
            .bind(&rid)
            .bind(&sale_id)
            .bind(&rno)
            .bind(a.refund_amount)
            .bind(&a.refund_method)
            .bind(time::today_dhaka().to_string())
            .bind(p.user_id.to_string())
            .bind(time::now_utc_string())
            .execute(&mut *tx)
            .await?;
        if let Some(cid) = &customer {
            customers::post_ledger(
                &mut tx,
                cid,
                "adjustment",
                a.refund_amount,
                "sale_refund",
                &fid,
                &format!("{rno}: refund for {number}"),
                Some(p.user_id),
            )
            .await?;
        }
        post_refund(&mut tx, &fid).await?;
        refund_receipt = Some(rno);
    }
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "return.approved", entity_type: "sale_return", entity_id: Some(&rid), detail: Some(serde_json::json!({"number": number, "value": total_value, "refund": a.refund_amount, "method": a.refund_method, "outcomes": outcomes})), reason: Some(a.note.trim()).filter(|s| !s.is_empty()) }).await?;
    tx.commit().await?;
    Ok(ApproveResult {
        value: total_value,
        refund_receipt,
    })
}

pub async fn reject_return(
    pool: &SqlitePool,
    p: &Principal,
    return_id: Id,
    reason: &str,
) -> CoreResult<()> {
    p.require(perm::RETURNS_APPROVE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to reject a return".into(),
        ));
    }
    let rid = return_id.to_string();
    let mut tx = pool.begin().await?;
    let head: Option<(String, String)> =
        sqlx::query_as("SELECT number, status FROM sale_returns WHERE id = ?")
            .bind(&rid)
            .fetch_optional(&mut *tx)
            .await?;
    let (number, status) = head.ok_or_else(|| CoreError::NotFound("return".into()))?;
    if status != "requested" {
        return Err(CoreError::Conflict(format!(
            "this return is already {status}"
        )));
    }
    let items: Vec<(String, Option<String>, i64, i64)> = sqlx::query_as(
        "SELECT variant_id, serial_id, qty, cost_total FROM sale_return_items WHERE return_id = ?",
    )
    .bind(&rid)
    .fetch_all(&mut *tx)
    .await?;
    for (variant, serial, qty, cost) in items {
        let why = format!("Return {number} rejected: goods handed back");
        match serial {
            Some(sid) => {
                stock::release_to_customer(
                    &mut tx,
                    Id::parse(&sid)?,
                    "sale_return",
                    &rid,
                    &why,
                    Some(p.user_id),
                )
                .await?
            }
            None => {
                stock::post_movement(
                    &mut tx,
                    &MovementIn {
                        variant_id: Id::parse(&variant)?,
                        bucket: Bucket::PendingInspection,
                        qty_delta: -qty,
                        cost_total: -cost,
                        serial_id: None,
                        doc_type: "sale_return",
                        doc_id: &rid,
                        reason: &why,
                        actor: Some(p.user_id),
                        allow_negative: false,
                    },
                )
                .await?;
            }
        }
    }
    sqlx::query("UPDATE sale_returns SET status = 'rejected', decision_note = ?, decided_by = ?, decided_at = ? WHERE id = ?").bind(reason.trim()).bind(p.user_id.to_string()).bind(time::now_utc_string()).bind(&rid).execute(&mut *tx).await?;
    post_goods_out(&mut tx, &rid).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "return.rejected",
            entity_type: "sale_return",
            entity_id: Some(&rid),
            detail: Some(serde_json::json!({"number": number})),
            reason: Some(reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------- posting

pub(crate) async fn post_goods_in(conn: &mut SqliteConnection, return_id: &str) -> CoreResult<()> {
    let r: Option<(String, String, i64)> = sqlx::query_as("SELECT r.number, r.return_date, COALESCE((SELECT SUM(cost_total) FROM sale_return_items WHERE return_id = r.id), 0) FROM sale_returns r WHERE r.id = ?").bind(return_id).fetch_optional(&mut *conn).await?;
    let Some((number, date, cost)) = r else {
        return Ok(());
    };
    let key = format!("sale_return_goods:{return_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &format!("Goods returned {number}"),
            source_type: "sale_return",
            source_id: return_id,
            lines: vec![
                dr(code::INVENTORY, cost, "Goods back in stock"),
                cr(code::COGS, cost, "Cost of sale reversed"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_goods_out(conn: &mut SqliteConnection, return_id: &str) -> CoreResult<()> {
    let r: Option<(String, i64, Option<String>)> = sqlx::query_as("SELECT r.number, COALESCE((SELECT SUM(cost_total) FROM sale_return_items WHERE return_id = r.id), 0), r.decided_at FROM sale_returns r WHERE r.id = ? AND r.status = 'rejected'").bind(return_id).fetch_optional(&mut *conn).await?;
    let Some((number, cost, Some(at))) = r else {
        return Ok(());
    };
    let key = format!("sale_return_rejected:{return_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &accounting::dhaka_date(&at),
            memo: &format!("Return rejected {number}"),
            source_type: "sale_return",
            source_id: return_id,
            lines: vec![
                dr(code::COGS, cost, "Cost of sale restored"),
                cr(code::INVENTORY, cost, "Goods handed back"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_credit_note(
    conn: &mut SqliteConnection,
    return_id: &str,
) -> CoreResult<()> {
    let r: Option<(String, i64, i64, Option<String>)> = sqlx::query_as("SELECT r.number, COALESCE((SELECT SUM(value) FROM sale_return_items WHERE return_id = r.id), 0), COALESCE((SELECT SUM(tax) FROM sale_return_items WHERE return_id = r.id), 0), r.decided_at FROM sale_returns r WHERE r.id = ? AND r.status = 'completed'").bind(return_id).fetch_optional(&mut *conn).await?;
    let Some((number, value, tax, Some(at))) = r else {
        return Ok(());
    };
    let key = format!("sale_return_credit:{return_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &accounting::dhaka_date(&at),
            memo: &format!("Credit note {number}"),
            source_type: "sale_return",
            source_id: return_id,
            lines: vec![
                dr(code::SALES_RETURNS, value - tax, "Sales returned"),
                dr(code::TAX_PAYABLE, tax, "Tax reversed"),
                cr(code::AR, value, "Credit to customer"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_write_off(conn: &mut SqliteConnection, return_id: &str) -> CoreResult<()> {
    let r: Option<(String, i64, Option<String>)> = sqlx::query_as("SELECT r.number, COALESCE((SELECT SUM(cost_total) FROM sale_return_items WHERE return_id = r.id AND outcome = 'write_off'), 0), r.decided_at FROM sale_returns r WHERE r.id = ? AND r.status = 'completed'").bind(return_id).fetch_optional(&mut *conn).await?;
    let Some((number, cost, Some(at))) = r else {
        return Ok(());
    };
    let key = format!("sale_return_writeoff:{return_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &accounting::dhaka_date(&at),
            memo: &format!("Returned goods written off {number}"),
            source_type: "sale_return",
            source_id: return_id,
            lines: vec![
                dr(code::INVENTORY_ADJUSTMENTS, cost, "Write-off"),
                cr(code::INVENTORY, cost, "Stock written off"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_refund(conn: &mut SqliteConnection, refund_id: &str) -> CoreResult<()> {
    let r: Option<(i64, String, String, String)> =
        sqlx::query_as("SELECT amount, method, paid_on, receipt_no FROM sale_refunds WHERE id = ?")
            .bind(refund_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((amount, method, date, rno)) = r else {
        return Ok(());
    };
    let key = format!("sale_refund:{refund_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &format!("Refund {rno}"),
            source_type: "sale_refund",
            source_id: refund_id,
            lines: vec![
                dr(code::AR, amount, "Refund to customer"),
                cr(method_account(&method), amount, "Money out"),
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
    for (sql, which) in [
        ("SELECT id FROM sale_returns WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_return_goods:' || sale_returns.id)", 0),
        ("SELECT id FROM sale_returns WHERE status = 'completed' AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_return_credit:' || sale_returns.id)", 1),
        ("SELECT id FROM sale_returns WHERE status = 'completed' AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_return_writeoff:' || sale_returns.id) AND EXISTS (SELECT 1 FROM sale_return_items WHERE return_id = sale_returns.id AND outcome = 'write_off')", 2),
        ("SELECT id FROM sale_returns WHERE status = 'rejected' AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_return_rejected:' || sale_returns.id)", 3),
        ("SELECT id FROM sale_refunds WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_refund:' || sale_refunds.id)", 4),
    ] {
        let ids: Vec<String> = sqlx::query_scalar(sql).fetch_all(pool).await?;
        for id in ids {
            let mut tx = pool.begin().await?;
            match which {
                0 => post_goods_in(&mut tx, &id).await?,
                1 => post_credit_note(&mut tx, &id).await?,
                2 => post_write_off(&mut tx, &id).await?,
                3 => post_goods_out(&mut tx, &id).await?,
                _ => post_refund(&mut tx, &id).await?,
            }
            tx.commit().await?;
            n += 1;
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------- queries

#[derive(Debug, Serialize)]
pub struct ReturnRow {
    pub id: String,
    pub number: String,
    pub sale_id: String,
    pub sale_number: String,
    pub customer: Option<String>,
    pub status: String,
    pub return_date: String,
    pub reason: String,
    pub value: i64,
}

#[derive(Debug, Serialize)]
pub struct ReturnItemRow {
    pub id: String,
    pub label: String,
    pub sku: String,
    pub serial_no: Option<String>,
    pub qty: i64,
    pub value: i64,
    pub outcome: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReturnDetail {
    pub row: ReturnRow,
    pub decision_note: Option<String>,
    pub items: Vec<ReturnItemRow>,
    pub refunds: Vec<(String, i64, String)>,
}

fn can_see(p: &Principal) -> CoreResult<()> {
    if p.has(perm::RETURNS_PROCESS) || p.has(perm::RETURNS_APPROVE) {
        Ok(())
    } else {
        Err(CoreError::Forbidden(perm::RETURNS_PROCESS.into()))
    }
}

type RetRaw = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    i64,
);
const RET_SELECT: &str = "SELECT r.id, r.number, r.sale_id, s.number, c.name, r.status, r.return_date, r.reason, COALESCE((SELECT SUM(value) FROM sale_return_items WHERE return_id = r.id), 0)
    FROM sale_returns r JOIN sales s ON s.id = r.sale_id LEFT JOIN customers c ON c.id = r.customer_id";

fn rrow(r: RetRaw) -> ReturnRow {
    ReturnRow {
        id: r.0,
        number: r.1,
        sale_id: r.2,
        sale_number: r.3,
        customer: r.4,
        status: r.5,
        return_date: r.6,
        reason: r.7,
        value: r.8,
    }
}

pub async fn list_returns(
    pool: &SqlitePool,
    p: &Principal,
    status: Option<&str>,
    search: Option<&str>,
) -> CoreResult<Vec<ReturnRow>> {
    can_see(p)?;
    let like = search.map(crate::catalogue::like_pattern);
    let rows: Vec<RetRaw> = sqlx::query_as(&format!("{RET_SELECT} WHERE (?1 IS NULL OR r.status = ?1) AND (?2 IS NULL OR r.number LIKE ?2 OR s.number LIKE ?2 OR c.name LIKE ?2) ORDER BY r.created_at DESC LIMIT 300")).bind(status).bind(like).fetch_all(pool).await?;
    Ok(rows.into_iter().map(rrow).collect())
}

pub async fn get_return(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<ReturnDetail> {
    can_see(p)?;
    let rid = id.to_string();
    let r: RetRaw = sqlx::query_as(&format!("{RET_SELECT} WHERE r.id = ?"))
        .bind(&rid)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("return".into()))?;
    let note: Option<String> =
        sqlx::query_scalar("SELECT decision_note FROM sale_returns WHERE id = ?")
            .bind(&rid)
            .fetch_one(pool)
            .await?;
    type ItemRaw = (
        String,
        String,
        String,
        Option<String>,
        i64,
        i64,
        Option<String>,
        Option<String>,
    );
    let items: Vec<ItemRaw> = sqlx::query_as(
        "SELECT ri.id, TRIM(pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END), v.sku, s.serial_no, ri.qty, ri.value, ri.outcome, ri.note
         FROM sale_return_items ri JOIN product_variants v ON v.id = ri.variant_id JOIN products pr ON pr.id = v.product_id LEFT JOIN product_serials s ON s.id = ri.serial_id WHERE ri.return_id = ? ORDER BY ri.rowid",
    )
    .bind(&rid)
    .fetch_all(pool)
    .await?;
    let refunds: Vec<(String, i64, String)> = sqlx::query_as("SELECT receipt_no, amount, method FROM sale_refunds WHERE return_id = ? ORDER BY created_at").bind(&rid).fetch_all(pool).await?;
    Ok(ReturnDetail {
        row: rrow(r),
        decision_note: note,
        items: items
            .into_iter()
            .map(|i| ReturnItemRow {
                id: i.0,
                label: i.1,
                sku: i.2,
                serial_no: i.3,
                qty: i.4,
                value: i.5,
                outcome: i.6,
                note: i.7,
            })
            .collect(),
        refunds,
    })
}

#[derive(Debug, Serialize)]
pub struct ReturnableItem {
    pub sale_item_id: String,
    pub label: String,
    pub sku: String,
    pub serial_no: Option<String>,
    pub serial_status: Option<String>,
    pub sold_qty: i64,
    pub returnable_qty: i64,
    pub unit_value: i64,
}

/// Lines of a sale that can still be returned (lookup by invoice number or serial number).
pub async fn returnable_items(
    pool: &SqlitePool,
    p: &Principal,
    sale_id: Id,
) -> CoreResult<Vec<ReturnableItem>> {
    can_see(p)?;
    type Raw = (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
        i64,
    );
    let rows: Vec<Raw> = sqlx::query_as(
        "SELECT i.id, TRIM(pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END), v.sku, s.serial_no, s.status, i.qty,
            i.qty - COALESCE((SELECT SUM(ri.qty) FROM sale_return_items ri JOIN sale_returns r ON r.id = ri.return_id WHERE ri.sale_item_id = i.id AND r.status IN ('requested', 'completed')), 0), i.line_total / i.qty
         FROM sale_items i JOIN product_variants v ON v.id = i.variant_id JOIN products pr ON pr.id = v.product_id LEFT JOIN product_serials s ON s.id = i.serial_id WHERE i.sale_id = ? ORDER BY i.rowid",
    )
    .bind(sale_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ReturnableItem {
            sale_item_id: r.0,
            label: r.1,
            sku: r.2,
            serial_no: r.3,
            serial_status: r.4,
            sold_qty: r.5,
            returnable_qty: r.6,
            unit_value: r.7,
        })
        .collect())
}

/// Find the invoice a serial number was sold on.
pub async fn find_sale_by_serial(
    pool: &SqlitePool,
    p: &Principal,
    serial_no: &str,
) -> CoreResult<Option<(String, String)>> {
    can_see(p)?;
    Ok(sqlx::query_as("SELECT s.id, s.number FROM sale_items i JOIN product_serials ps ON ps.id = i.serial_id JOIN sales s ON s.id = i.sale_id WHERE ps.serial_no = ? ORDER BY s.created_at DESC LIMIT 1").bind(serial_no.trim()).fetch_optional(pool).await?)
}
