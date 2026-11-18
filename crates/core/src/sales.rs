//! Sales (retail now; wholesale credit mechanics included; installments arrive in P4).
//!
//! One atomic transaction per sale: stock out (at cost), customer ledger, receipts, journals and audit
//! all commit together or not at all. A per-form `client_key` makes duplicate submits harmless.

use crate::accounting::{self, code, cr, dr, method_account, PostIn};
use crate::audit;
use crate::authz::{perm, Principal};
use crate::customers;
use crate::stock::{self, Bucket, MovementIn, SerialStatus};
use crate::{sequence, time, CoreError, CoreResult, Id, Money};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize)]
pub struct SaleLineIn {
    pub variant_id: String,
    pub qty: i64,
    /// Discount for the whole line in paisa (needs `sales.discount`).
    pub discount: i64,
    /// Exactly `qty` serial ids for serial-tracked products; empty otherwise.
    pub serial_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayLineIn {
    pub method: String,
    pub amount: i64,
    pub reference: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaleIn {
    /// "retail" or "wholesale"
    pub mode: String,
    pub customer_id: Option<String>,
    pub sale_date: Option<String>,
    pub items: Vec<SaleLineIn>,
    pub payments: Vec<PayLineIn>,
    pub notes: String,
    pub client_key: String,
    /// Sell beyond the credit limit (needs `sales.credit_override`).
    pub credit_override: bool,
    /// Required when `mode` is "installment": the financing terms (payments are then the down payment).
    #[serde(default)]
    pub installment: Option<crate::installments::InstallmentIn>,
}

#[derive(Debug, Serialize)]
pub struct SaleResult {
    pub id: String,
    pub number: String,
    pub grand_total: i64,
    pub paid: i64,
    pub due: i64,
    pub receipts: Vec<String>,
}

struct Tax {
    enabled: bool,
    rate_bp: i64,
    inclusive: bool,
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

async fn load_tax(conn: &mut SqliteConnection) -> CoreResult<Tax> {
    Ok(Tax {
        enabled: setting(conn, "tax.enabled").await? == "true",
        rate_bp: setting(conn, "tax.default_rate_bp")
            .await?
            .parse()
            .unwrap_or(0),
        inclusive: setting(conn, "tax.prices_include_tax").await? == "true",
    })
}

/// (tax, line_total) for a net-of-discount amount.
fn line_tax(net: i64, t: &Tax) -> CoreResult<(i64, i64)> {
    if !t.enabled || t.rate_bp == 0 {
        return Ok((0, net));
    }
    if t.inclusive {
        // tax portion of a tax-inclusive price: net * r / (1 + r)
        let num = (net as i128) * (t.rate_bp as i128);
        let den = 10_000i128 + t.rate_bp as i128;
        let q = (num * 2 + den) / (den * 2);
        let tax = i64::try_from(q).map_err(|_| CoreError::Overflow)?;
        Ok((tax, net))
    } else {
        let tax = Money::from_paisa(net).percent_bp(t.rate_bp)?.paisa();
        Ok((tax, net.checked_add(tax).ok_or(CoreError::Overflow)?))
    }
}

fn existing_result_query() -> &'static str {
    "SELECT s.id, s.number, s.grand_total, COALESCE((SELECT SUM(amount) FROM sale_payments WHERE sale_id = s.id), 0) FROM sales s WHERE s.client_key = ?"
}

struct Priced {
    variant_id: String,
    qty: i64,
    unit_price: i64,
    discount: i64,
    tax: i64,
    line_total: i64,
    serialized: bool,
    serial_ids: Vec<String>,
    sku: String,
    purchase_cost: i64,
    /// Retail list price (the cash price of the goods) for installment disclosure.
    retail: i64,
}

async fn price_items(tx: &mut SqliteConnection, si: &SaleIn, tax: &Tax) -> CoreResult<Vec<Priced>> {
    let mut priced: Vec<Priced> = vec![];
    let mut seen_variants = HashSet::new();
    let mut seen_serials = HashSet::new();
    for l in &si.items {
        if l.qty <= 0 {
            return Err(CoreError::Validation("quantity must be positive".into()));
        }
        if l.discount < 0 {
            return Err(CoreError::Validation("discount cannot be negative".into()));
        }
        if !seen_variants.insert(l.variant_id.clone()) {
            return Err(CoreError::Validation(
                "each product variant may appear on only one line".into(),
            ));
        }
        type VariantRaw = (String, i64, i64, i64, i64, i64, Option<i64>);
        let v: Option<VariantRaw> = sqlx::query_as("SELECT v.sku, v.retail_price, v.wholesale_price, v.purchase_cost, p.is_serialized, v.is_active * p.is_active, v.installment_price FROM product_variants v JOIN products p ON p.id = v.product_id WHERE v.id = ?")
            .bind(&l.variant_id)
            .fetch_optional(&mut *tx)
            .await?;
        let (sku, retail, wholesale, pcost, serialized, active, inst_price) =
            v.ok_or_else(|| CoreError::Validation("unknown product variant".into()))?;
        if active != 1 {
            return Err(CoreError::Validation(format!("{sku} is inactive")));
        }
        let unit_price = match si.mode.as_str() {
            "wholesale" => {
                crate::pricing::wholesale_price(
                    tx,
                    si.customer_id.as_deref(),
                    &l.variant_id,
                    wholesale,
                )
                .await?
            }
            "installment" => inst_price.unwrap_or(retail),
            _ => retail,
        };
        let gross = Money::from_paisa(unit_price)
            .checked_mul_qty(l.qty)?
            .paisa();
        if l.discount > gross {
            return Err(CoreError::Validation(format!(
                "discount on {sku} exceeds the line amount"
            )));
        }
        if serialized == 1 {
            if l.serial_ids.len() as i64 != l.qty {
                return Err(CoreError::Validation(format!(
                    "select exactly {} serial number(s) for {sku}",
                    l.qty
                )));
            }
            for s in &l.serial_ids {
                if !seen_serials.insert(s.clone()) {
                    return Err(CoreError::Validation(
                        "a serial number was selected twice".into(),
                    ));
                }
            }
        } else if !l.serial_ids.is_empty() {
            return Err(CoreError::Validation(format!(
                "{sku} is not serial-tracked"
            )));
        }
        let net = gross - l.discount;
        let (tx_amt, line_total) = line_tax(net, tax)?;
        priced.push(Priced {
            variant_id: l.variant_id.clone(),
            qty: l.qty,
            unit_price,
            discount: l.discount,
            tax: tx_amt,
            line_total,
            serialized: serialized == 1,
            serial_ids: l.serial_ids.clone(),
            sku,
            purchase_cost: pcost,
            retail,
        });
    }
    Ok(priced)
}

pub async fn create_sale(pool: &SqlitePool, p: &Principal, si: SaleIn) -> CoreResult<SaleResult> {
    match si.mode.as_str() {
        "retail" => p.require(perm::SALES_CREATE)?,
        "wholesale" => {
            p.require(perm::SALES_WHOLESALE)?;
        }
        "installment" => {
            p.require(perm::INSTALLMENTS_CREATE)?;
            if si.installment.is_none() {
                return Err(CoreError::Validation(
                    "installment sales need the financing terms".into(),
                ));
            }
        }
        _ => return Err(CoreError::Validation("unknown sale mode".into())),
    }
    let key = si.client_key.trim();
    if key.is_empty() || key.len() > 64 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    if si.items.is_empty() {
        return Err(CoreError::Validation("add at least one item".into()));
    }
    let sale_date = match &si.sale_date {
        Some(d) => NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")
            .map_err(|_| CoreError::Validation("sale date must be YYYY-MM-DD".into()))?
            .to_string(),
        None => time::today_dhaka().to_string(),
    };
    if si.items.iter().any(|l| l.discount > 0) {
        p.require(perm::SALES_DISCOUNT)?;
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(r) = sqlx::query_as::<_, (String, String, i64, i64)>(existing_result_query())
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?
    {
        let receipts: Vec<String> = sqlx::query_scalar(
            "SELECT receipt_no FROM sale_payments WHERE sale_id = ? ORDER BY created_at, id",
        )
        .bind(&r.0)
        .fetch_all(&mut *tx)
        .await?;
        return Ok(SaleResult {
            id: r.0,
            number: r.1,
            grand_total: r.2,
            paid: r.3,
            due: r.2 - r.3,
            receipts,
        });
    }
    // customer
    let customer = match &si.customer_id {
        Some(cid) => {
            let r: Option<(i64, i64, String)> =
                sqlx::query_as("SELECT is_active, credit_limit, name FROM customers WHERE id = ?")
                    .bind(cid)
                    .fetch_optional(&mut *tx)
                    .await?;
            let (active, limit, name) =
                r.ok_or_else(|| CoreError::Validation("unknown customer".into()))?;
            if active != 1 {
                return Err(CoreError::Validation("customer is inactive".into()));
            }
            Some((cid.clone(), limit, name))
        }
        None => None,
    };
    if si.mode != "retail" && customer.is_none() {
        return Err(CoreError::Validation(format!(
            "{} sales need a customer",
            si.mode
        )));
    }
    let tax = load_tax(&mut tx).await?;
    let allow_negative = setting(&mut tx, "stock.allow_negative").await? == "true";

    let priced = price_items(&mut tx, &si, &tax).await?;
    let subtotal: i64 = priced.iter().map(|x| x.unit_price * x.qty).sum();
    let discount_total: i64 = priced.iter().map(|x| x.discount).sum();
    let tax_total: i64 = priced.iter().map(|x| x.tax).sum();
    let grand_total: i64 = priced.iter().map(|x| x.line_total).sum();

    // payments
    let mut paid = 0i64;
    for pl in &si.payments {
        if pl.amount <= 0 {
            return Err(CoreError::Validation(
                "payment amounts must be positive".into(),
            ));
        }
        if !["cash", "bank", "mobile_wallet", "cheque", "other"].contains(&pl.method.as_str()) {
            return Err(CoreError::Validation("unknown payment method".into()));
        }
        paid = paid.checked_add(pl.amount).ok_or(CoreError::Overflow)?;
    }
    if paid > grand_total {
        return Err(CoreError::Validation(format!(
            "payments ({}) exceed the total ({}); enter the amount actually applied to this sale",
            Money::from_paisa(paid),
            Money::from_paisa(grand_total)
        )));
    }
    let due = grand_total - paid;
    if due > 0 {
        if si.mode == "retail" {
            return Err(CoreError::Validation(
                "retail sales must be paid in full".into(),
            ));
        }
        let (cid, limit, name) = customer.as_ref().expect("non-retail has customer");
        if si.mode == "wholesale"
            && setting(&mut tx, "sales.credit_limit_enforced").await? == "true"
        {
            let current = customers::balance(&mut tx, cid).await?;
            if current + due > *limit {
                if si.credit_override {
                    p.require(perm::SALES_CREDIT_OVERRIDE)?;
                } else {
                    return Err(CoreError::Conflict(format!(
                        "credit limit exceeded for {name}: owes {}, this sale adds {}, limit {} (a manager can override)",
                        Money::from_paisa(current),
                        Money::from_paisa(due),
                        Money::from_paisa(*limit)
                    )));
                }
            }
        }
    }

    // document
    sequence::ensure(&mut tx, "sale", "INV-", 6).await?;
    sequence::ensure(&mut tx, "receipt", "RCT-", 6).await?;
    let number = sequence::next(&mut tx, "sale").await?;
    let sale_id = Id::new();
    let sid = sale_id.to_string();
    sqlx::query("INSERT INTO sales (id, number, mode, customer_id, sale_date, subtotal, discount_total, tax_total, grand_total, notes, client_key, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&sid)
        .bind(&number)
        .bind(&si.mode)
        .bind(customer.as_ref().map(|c| c.0.clone()))
        .bind(&sale_date)
        .bind(subtotal)
        .bind(discount_total)
        .bind(tax_total)
        .bind(grand_total)
        .bind(si.notes.trim())
        .bind(key)
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await?;

    // stock out + sale items
    let mut items_json = vec![];
    for l in &priced {
        let variant = Id::parse(&l.variant_id)?;
        let reason = format!("Sold on {number}");
        let mut cost_total = 0i64;
        let mut serial_for_row: Vec<Option<String>> = vec![];
        if l.serialized {
            for sid_ in &l.serial_ids {
                let row: Option<(String, String, i64)> = sqlx::query_as(
                    "SELECT variant_id, status, cost FROM product_serials WHERE id = ?",
                )
                .bind(sid_)
                .fetch_optional(&mut *tx)
                .await?;
                let (v, st, cost) =
                    row.ok_or_else(|| CoreError::Validation("unknown serial number".into()))?;
                if v != l.variant_id {
                    return Err(CoreError::Validation(format!(
                        "a selected serial does not belong to {}",
                        l.sku
                    )));
                }
                if st != "available" {
                    return Err(CoreError::Conflict(format!(
                        "serial is {} and cannot be sold",
                        st.replace('_', " ")
                    )));
                }
                stock::transition_serial(
                    &mut tx,
                    Id::parse(sid_)?,
                    SerialStatus::Sold,
                    "sale",
                    &sid,
                    &reason,
                    Some(p.user_id),
                )
                .await?;
                cost_total += cost;
                serial_for_row.push(Some(sid_.clone()));
            }
        } else {
            let on_hand = stock::on_hand(&mut tx, variant, Bucket::Available).await?;
            if on_hand < l.qty && !allow_negative {
                return Err(CoreError::Conflict(format!(
                    "not enough stock of {}: {on_hand} available, {} requested",
                    l.sku, l.qty
                )));
            }
            let from_stock = on_hand.clamp(0, l.qty);
            let mut cost =
                stock::avg_cost_out(&mut tx, variant, Bucket::Available, from_stock).await?;
            if l.qty > from_stock {
                cost += Money::from_paisa(l.purchase_cost)
                    .checked_mul_qty(l.qty - from_stock)?
                    .paisa(); // negative stock valued at list cost
            }
            stock::post_movement(
                &mut tx,
                &MovementIn {
                    variant_id: variant,
                    bucket: Bucket::Available,
                    qty_delta: -l.qty,
                    cost_total: -cost,
                    serial_id: None,
                    doc_type: "sale",
                    doc_id: &sid,
                    reason: &reason,
                    actor: Some(p.user_id),
                    allow_negative,
                },
            )
            .await?;
            cost_total = cost;
            serial_for_row.push(None);
        }
        if l.serialized {
            // one row per unit so each serial is traceable to its invoice line
            for (i, serial) in serial_for_row.iter().enumerate() {
                let unit_cost: i64 =
                    sqlx::query_scalar("SELECT cost FROM product_serials WHERE id = ?")
                        .bind(serial.as_ref().expect("serial"))
                        .fetch_one(&mut *tx)
                        .await?;
                let (d, t, total) = split_unit(l.discount, l.tax, l.line_total, l.qty, i as i64);
                sqlx::query("INSERT INTO sale_items (id, sale_id, variant_id, serial_id, qty, unit_price, discount, tax, line_total, cost_total) VALUES (?, ?, ?, ?, 1, ?, ?, ?, ?, ?)")
                    .bind(Id::new().to_string())
                    .bind(&sid)
                    .bind(&l.variant_id)
                    .bind(serial)
                    .bind(l.unit_price)
                    .bind(d)
                    .bind(t)
                    .bind(total)
                    .bind(unit_cost)
                    .execute(&mut *tx)
                    .await?;
            }
        } else {
            sqlx::query("INSERT INTO sale_items (id, sale_id, variant_id, serial_id, qty, unit_price, discount, tax, line_total, cost_total) VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?)")
                .bind(Id::new().to_string())
                .bind(&sid)
                .bind(&l.variant_id)
                .bind(l.qty)
                .bind(l.unit_price)
                .bind(l.discount)
                .bind(l.tax)
                .bind(l.line_total)
                .bind(cost_total)
                .execute(&mut *tx)
                .await?;
        }
        items_json.push(serde_json::json!({"sku": l.sku, "qty": l.qty, "price": l.unit_price, "discount": l.discount}));
    }

    // customer ledger: the invoice, then each receipt
    if let Some((cid, _, _)) = &customer {
        customers::post_ledger(
            &mut tx,
            cid,
            "sale",
            grand_total,
            "sale",
            &sid,
            &number,
            Some(p.user_id),
        )
        .await?;
    }
    let mut receipts = vec![];
    for pl in &si.payments {
        let pid = Id::new();
        let rno = sequence::next(&mut tx, "receipt").await?;
        sqlx::query("INSERT INTO sale_payments (id, sale_id, receipt_no, method, amount, paid_on, reference, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(pid.to_string())
            .bind(&sid)
            .bind(&rno)
            .bind(&pl.method)
            .bind(pl.amount)
            .bind(&sale_date)
            .bind(pl.reference.trim())
            .bind(p.user_id.to_string())
            .bind(time::now_utc_string())
            .execute(&mut *tx)
            .await?;
        if let Some((cid, _, _)) = &customer {
            customers::post_ledger(
                &mut tx,
                cid,
                "payment",
                -pl.amount,
                "sale_payment",
                &pid.to_string(),
                &rno,
                Some(p.user_id),
            )
            .await?;
        }
        post_sale_payment(&mut tx, &pid.to_string()).await?;
        receipts.push(rno);
    }
    post_sale(&mut tx, &sid).await?;
    post_sale_cogs(&mut tx, &sid).await?;
    if si.mode == "installment" {
        let inst = si.installment.as_ref().expect("checked above");
        let cash_price: i64 = priced
            .iter()
            .map(|x| x.retail * x.qty - x.discount)
            .sum::<i64>()
            .max(0);
        let (cid, _, _) = customer.as_ref().expect("customer required");
        crate::installments::create_agreement(
            &mut tx,
            p,
            &crate::installments::AgreementCtx {
                sale_id: &sid,
                sale_number: &number,
                customer_id: cid,
                sale_date: &sale_date,
                cash_price,
                price_total: grand_total,
                down_payment: paid,
            },
            inst,
        )
        .await?;
    }
    audit::log(&mut tx, p, "sale.created", "sale", &sid, serde_json::json!({"number": number, "mode": si.mode, "grand_total": grand_total, "paid": paid, "due": due, "customer": customer.as_ref().map(|c| c.0.clone()), "items": items_json})).await?;
    if discount_total > 0 {
        audit::log(
            &mut tx,
            p,
            "sale.discount_applied",
            "sale",
            &sid,
            serde_json::json!({"number": number, "discount_total": discount_total}),
        )
        .await?;
    }
    if due > 0 && si.credit_override {
        audit::log(
            &mut tx,
            p,
            "sale.credit_override",
            "sale",
            &sid,
            serde_json::json!({"number": number, "due": due}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(SaleResult {
        id: sid,
        number,
        grand_total,
        paid,
        due,
        receipts,
    })
}

/// Splits line-level amounts across `qty` single-unit rows; the remainder lands on the last unit.
fn split_unit(discount: i64, tax: i64, total: i64, qty: i64, idx: i64) -> (i64, i64, i64) {
    let part = |v: i64| {
        if idx == qty - 1 {
            v - (v / qty) * (qty - 1)
        } else {
            v / qty
        }
    };
    (part(discount), part(tax), part(total))
}

// ---------------------------------------------------------------- receipts on account

#[derive(Debug, Deserialize)]
pub struct CollectIn {
    pub customer_id: String,
    /// Apply to one invoice; `None` = oldest invoices first.
    pub sale_id: Option<String>,
    pub amount: i64,
    pub method: String,
    pub reference: String,
    pub paid_on: Option<String>,
    pub client_key: String,
}

#[derive(Debug, Serialize)]
pub struct CollectResult {
    pub receipts: Vec<String>,
    pub allocations: Vec<(String, i64)>,
}

async fn sale_due(conn: &mut SqliteConnection, sale_id: &str) -> CoreResult<i64> {
    Ok(sqlx::query_scalar("SELECT CASE WHEN s.mode = 'installment' THEN 0 ELSE s.grand_total - COALESCE((SELECT SUM(amount) FROM sale_payments WHERE sale_id = s.id), 0) END FROM sales s WHERE s.id = ?").bind(sale_id).fetch_one(&mut *conn).await?)
}

/// Receives a customer payment and applies it to invoices (named invoice, else oldest first).
/// The allocation is shown to the user and recorded as one receipt per invoice.
pub async fn collect_payment(
    pool: &SqlitePool,
    p: &Principal,
    c: CollectIn,
) -> CoreResult<CollectResult> {
    p.require(perm::CUSTOMERS_COLLECT)?;
    if c.amount <= 0 {
        return Err(CoreError::Validation("amount must be positive".into()));
    }
    if !["cash", "bank", "mobile_wallet", "cheque", "other"].contains(&c.method.as_str()) {
        return Err(CoreError::Validation("unknown payment method".into()));
    }
    let key = c.client_key.trim();
    if key.is_empty() || key.len() > 60 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    let paid_on = match &c.paid_on {
        Some(d) => NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")
            .map_err(|_| CoreError::Validation("date must be YYYY-MM-DD".into()))?
            .to_string(),
        None => time::today_dhaka().to_string(),
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let prior: Vec<(String, String, i64)> = sqlx::query_as("SELECT receipt_no, sale_id, amount FROM sale_payments WHERE client_key LIKE ? ORDER BY receipt_no").bind(format!("{key}#%")).fetch_all(&mut *tx).await?;
    if !prior.is_empty() {
        return Ok(CollectResult {
            receipts: prior.iter().map(|r| r.0.clone()).collect(),
            allocations: prior.iter().map(|r| (r.1.clone(), r.2)).collect(),
        });
    }
    let cust: Option<i64> = sqlx::query_scalar("SELECT is_active FROM customers WHERE id = ?")
        .bind(&c.customer_id)
        .fetch_optional(&mut *tx)
        .await?;
    if cust.is_none() {
        return Err(CoreError::Validation("unknown customer".into()));
    }
    let targets: Vec<(String, String, i64)> = match &c.sale_id {
        Some(sid) => {
            let ok: Option<String> =
                sqlx::query_scalar("SELECT number FROM sales WHERE id = ? AND customer_id = ? AND mode <> 'installment'")
                    .bind(sid)
                    .bind(&c.customer_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let number = ok.ok_or_else(|| {
                CoreError::Validation("invoice does not belong to this customer".into())
            })?;
            vec![(sid.clone(), number, sale_due(&mut tx, sid).await?)]
        }
        None => {
            let ids: Vec<(String, String)> = sqlx::query_as(
                "SELECT id, number FROM sales WHERE customer_id = ? AND mode <> 'installment' ORDER BY sale_date, number",
            )
            .bind(&c.customer_id)
            .fetch_all(&mut *tx)
            .await?;
            let mut v = vec![];
            for (id, number) in ids {
                let due = sale_due(&mut tx, &id).await?;
                if due > 0 {
                    v.push((id, number, due));
                }
            }
            v
        }
    };
    let total_due: i64 = targets.iter().map(|t| t.2.max(0)).sum();
    if c.amount > total_due {
        return Err(CoreError::Validation(format!(
            "amount exceeds what is owed on the selected invoice(s) ({})",
            Money::from_paisa(total_due)
        )));
    }
    sequence::ensure(&mut tx, "receipt", "RCT-", 6).await?;
    let mut left = c.amount;
    let mut receipts = vec![];
    let mut allocations = vec![];
    for (i, (sid, number, due)) in targets.iter().enumerate() {
        if left == 0 {
            break;
        }
        let part = left.min(*due);
        if part <= 0 {
            continue;
        }
        let pid = Id::new();
        let rno = sequence::next(&mut tx, "receipt").await?;
        sqlx::query("INSERT INTO sale_payments (id, sale_id, receipt_no, method, amount, paid_on, reference, note, client_key, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(pid.to_string())
            .bind(sid)
            .bind(&rno)
            .bind(&c.method)
            .bind(part)
            .bind(&paid_on)
            .bind(c.reference.trim())
            .bind(format!("Payment received against {number}"))
            .bind(format!("{key}#{i}"))
            .bind(p.user_id.to_string())
            .bind(time::now_utc_string())
            .execute(&mut *tx)
            .await?;
        customers::post_ledger(
            &mut tx,
            &c.customer_id,
            "payment",
            -part,
            "sale_payment",
            &pid.to_string(),
            &rno,
            Some(p.user_id),
        )
        .await?;
        post_sale_payment(&mut tx, &pid.to_string()).await?;
        receipts.push(rno);
        allocations.push((sid.clone(), part));
        left -= part;
    }
    audit::log(&mut tx, p, "customer.payment_received", "customer", &c.customer_id, serde_json::json!({"amount": c.amount, "method": c.method, "receipts": receipts, "allocations": allocations})).await?;
    tx.commit().await?;
    Ok(CollectResult {
        receipts,
        allocations,
    })
}

pub async fn reverse_payment(
    pool: &SqlitePool,
    p: &Principal,
    payment_id: Id,
    reason: &str,
) -> CoreResult<Id> {
    p.require(perm::CUSTOMERS_REVERSE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to reverse a payment".into(),
        ));
    }
    let oid = payment_id.to_string();
    let mut tx = pool.begin().await?;
    type PayRaw = (String, String, i64, String, Option<String>, Option<String>);
    let row: Option<PayRaw> = sqlx::query_as("SELECT sale_id, receipt_no, amount, method, reverses_id, (SELECT customer_id FROM sales WHERE id = sale_payments.sale_id) FROM sale_payments WHERE id = ?").bind(&oid).fetch_optional(&mut *tx).await?;
    let (sale_id, receipt_no, amount, method, reverses, customer) =
        row.ok_or_else(|| CoreError::NotFound("payment".into()))?;
    let is_installment: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sales WHERE id = ? AND mode = 'installment'")
            .bind(&sale_id)
            .fetch_one(&mut *tx)
            .await?;
    if is_installment > 0 {
        return Err(CoreError::Conflict(
            "the down payment of an installment sale cannot be reversed here; cancel the agreement through a return".into(),
        ));
    }
    if reverses.is_some() || amount <= 0 {
        return Err(CoreError::Conflict("a reversal cannot be reversed".into()));
    }
    let id = Id::new();
    sequence::ensure(&mut tx, "receipt", "RCT-", 6).await?;
    let rno = sequence::next(&mut tx, "receipt").await?;
    sqlx::query("INSERT INTO sale_payments (id, sale_id, receipt_no, method, amount, paid_on, reference, note, reverses_id, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&sale_id)
        .bind(&rno)
        .bind(&method)
        .bind(-amount)
        .bind(time::today_dhaka().to_string())
        .bind(format!("Reversal of {receipt_no}: {}", reason.trim()))
        .bind(&oid)
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| crate::catalogue::map_unique(e, "reversal of this payment"))?;
    if let Some(cid) = &customer {
        customers::post_ledger(
            &mut tx,
            cid,
            "payment_reversal",
            amount,
            "sale_payment",
            &id.to_string(),
            &rno,
            Some(p.user_id),
        )
        .await?;
    }
    post_sale_payment(&mut tx, &id.to_string()).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "customer.payment_reversed",
            entity_type: "sale_payment",
            entity_id: Some(&oid),
            detail: Some(
                serde_json::json!({"receipt": receipt_no, "amount": amount, "reversal": rno}),
            ),
            reason: Some(reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

// ---------------------------------------------------------------- posting

pub(crate) async fn post_sale(conn: &mut SqliteConnection, sale_id: &str) -> CoreResult<()> {
    let r: Option<(String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT number, sale_date, grand_total, tax_total, discount_total FROM sales WHERE id = ?",
    )
    .bind(sale_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((number, date, grand, tax, disc)) = r else {
        return Ok(());
    };
    let key = format!("sale:{sale_id}");
    let memo = if disc > 0 {
        format!("Sale {number} (discount {})", Money::from_paisa(disc))
    } else {
        format!("Sale {number}")
    };
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &memo,
            source_type: "sale",
            source_id: sale_id,
            lines: vec![
                dr(code::AR, grand, "Invoice"),
                cr(code::SALES, grand - tax, "Net sales"),
                cr(code::TAX_PAYABLE, tax, "Tax collected"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_sale_cogs(conn: &mut SqliteConnection, sale_id: &str) -> CoreResult<()> {
    let r: Option<(String, String, i64)> = sqlx::query_as("SELECT s.number, s.sale_date, COALESCE((SELECT SUM(cost_total) FROM sale_items WHERE sale_id = s.id), 0) FROM sales s WHERE s.id = ?").bind(sale_id).fetch_optional(&mut *conn).await?;
    let Some((number, date, cost)) = r else {
        return Ok(());
    };
    let key = format!("sale_cogs:{sale_id}");
    let memo = format!("Cost of goods sold {number}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &memo,
            source_type: "sale_cogs",
            source_id: sale_id,
            lines: vec![
                dr(code::COGS, cost, "COGS"),
                cr(code::INVENTORY, cost, "Stock out"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_sale_payment(
    conn: &mut SqliteConnection,
    payment_id: &str,
) -> CoreResult<()> {
    let r: Option<(i64, String, String, String)> = sqlx::query_as(
        "SELECT amount, method, paid_on, receipt_no FROM sale_payments WHERE id = ?",
    )
    .bind(payment_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((amount, method, date, rno)) = r else {
        return Ok(());
    };
    let key = format!("sale_payment:{payment_id}");
    let acct = method_account(&method);
    let memo = if amount > 0 {
        format!("Receipt {rno}")
    } else {
        format!("Receipt reversal {rno}")
    };
    let lines = if amount > 0 {
        vec![
            dr(acct, amount, "Money in"),
            cr(code::AR, amount, "Settles invoice"),
        ]
    } else {
        vec![
            dr(code::AR, -amount, "Invoice reopened"),
            cr(acct, -amount, "Money returned"),
        ]
    };
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &memo,
            source_type: "sale_payment",
            source_id: payment_id,
            lines,
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
        ("SELECT id FROM sales WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale:' || sales.id)", 0),
        ("SELECT id FROM sales WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_cogs:' || sales.id)", 1),
        ("SELECT id FROM sale_payments WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_payment:' || sale_payments.id)", 2),
    ] {
        let ids: Vec<String> = sqlx::query_scalar(sql).fetch_all(pool).await?;
        for id in ids {
            let mut tx = pool.begin().await?;
            match which {
                0 => post_sale(&mut tx, &id).await?,
                1 => post_sale_cogs(&mut tx, &id).await?,
                _ => post_sale_payment(&mut tx, &id).await?,
            }
            tx.commit().await?;
            n += 1;
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------- queries

#[derive(Debug, Serialize)]
pub struct SaleRow {
    pub id: String,
    pub number: String,
    pub mode: String,
    pub sale_date: String,
    pub customer_id: Option<String>,
    pub customer: Option<String>,
    pub grand_total: i64,
    pub paid: i64,
    pub due: i64,
}

#[derive(Debug, Serialize)]
pub struct SalePage {
    pub total: i64,
    pub rows: Vec<SaleRow>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SaleFilter {
    pub search: Option<String>,
    pub mode: Option<String>,
    pub customer_id: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub due_only: bool,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_sales(pool: &SqlitePool, p: &Principal, f: SaleFilter) -> CoreResult<SalePage> {
    p.require(perm::SALES_VIEW)?;
    let like = f.search.as_deref().map(crate::catalogue::like_pattern);
    let w = "WHERE (?1 IS NULL OR s.number LIKE ?1 OR c.name LIKE ?1 OR c.phone LIKE ?1) AND (?2 IS NULL OR s.mode = ?2) AND (?3 IS NULL OR s.customer_id = ?3) AND (?4 IS NULL OR s.sale_date >= ?4) AND (?5 IS NULL OR s.sale_date <= ?5)
        AND (?6 = 0 OR (s.grand_total + COALESCE((SELECT a.markup + a.fees_total + COALESCE((SELECT SUM(amount) FROM installment_late_fees WHERE agreement_id = a.id), 0) FROM installment_agreements a WHERE a.sale_id = s.id), 0)) > (COALESCE((SELECT SUM(amount) FROM sale_payments WHERE sale_id = s.id), 0) + COALESCE((SELECT SUM(c.amount) FROM installment_collections c JOIN installment_agreements a ON a.id = c.agreement_id WHERE a.sale_id = s.id), 0)))";
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM sales s LEFT JOIN customers c ON c.id = s.customer_id {w}"
    ))
    .bind(&like)
    .bind(&f.mode)
    .bind(&f.customer_id)
    .bind(&f.from)
    .bind(&f.to)
    .bind(f.due_only as i64)
    .fetch_one(pool)
    .await?;
    type Raw = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
    );
    let rows: Vec<Raw> = sqlx::query_as(&format!("SELECT s.id, s.number, s.mode, s.sale_date, s.customer_id, c.name, (s.grand_total + COALESCE((SELECT a.markup + a.fees_total + COALESCE((SELECT SUM(amount) FROM installment_late_fees WHERE agreement_id = a.id), 0) FROM installment_agreements a WHERE a.sale_id = s.id), 0)), (COALESCE((SELECT SUM(amount) FROM sale_payments WHERE sale_id = s.id), 0) + COALESCE((SELECT SUM(c.amount) FROM installment_collections c JOIN installment_agreements a ON a.id = c.agreement_id WHERE a.sale_id = s.id), 0)) FROM sales s LEFT JOIN customers c ON c.id = s.customer_id {w} ORDER BY s.sale_date DESC, s.number DESC LIMIT ?7 OFFSET ?8"))
        .bind(&like)
        .bind(&f.mode)
        .bind(&f.customer_id)
        .bind(&f.from)
        .bind(&f.to)
        .bind(f.due_only as i64)
        .bind(f.limit.unwrap_or(50).clamp(1, 200))
        .bind(f.offset.unwrap_or(0).max(0))
        .fetch_all(pool)
        .await?;
    Ok(SalePage {
        total,
        rows: rows
            .into_iter()
            .map(|r| SaleRow {
                id: r.0,
                number: r.1,
                mode: r.2,
                sale_date: r.3,
                customer_id: r.4,
                customer: r.5,
                grand_total: r.6,
                paid: r.7,
                due: r.6 - r.7,
            })
            .collect(),
    })
}

#[derive(Debug, Serialize)]
pub struct SaleItemRow {
    pub label: String,
    pub sku: String,
    pub serial_no: Option<String>,
    pub qty: i64,
    pub unit_price: i64,
    pub discount: i64,
    pub tax: i64,
    pub line_total: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SalePaymentRow {
    pub id: String,
    pub receipt_no: String,
    pub method: String,
    pub amount: i64,
    pub paid_on: String,
    pub reference: String,
    pub note: String,
    pub reverses_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SaleDetail {
    pub sale: SaleRow,
    pub subtotal: i64,
    pub discount_total: i64,
    pub tax_total: i64,
    /// Installment markup, fees and late fees included in the contract total (0 for other sales).
    pub financing_charges: i64,
    pub notes: String,
    pub cashier: Option<String>,
    pub customer_phone: Option<String>,
    pub customer_address: Option<String>,
    pub items: Vec<SaleItemRow>,
    pub payments: Vec<SalePaymentRow>,
}

pub async fn get_sale(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<SaleDetail> {
    p.require(perm::SALES_VIEW)?;
    let sid = id.to_string();
    type Raw = (
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
        i64,
        i64,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let r: Raw = sqlx::query_as("SELECT s.id, s.number, s.mode, s.sale_date, s.customer_id, c.name, s.grand_total, s.subtotal, s.discount_total, s.tax_total, s.notes, (SELECT username FROM users WHERE id = s.created_by), c.phone, c.address FROM sales s LEFT JOIN customers c ON c.id = s.customer_id WHERE s.id = ?")
        .bind(&sid)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("sale".into()))?;
    // installment sales: the contract also includes markup, fees and late fees, and collections count as paid
    let charges: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT a.markup + a.fees_total + COALESCE((SELECT SUM(amount) FROM installment_late_fees WHERE agreement_id = a.id), 0) FROM installment_agreements a WHERE a.sale_id = ?1), 0)")
        .bind(&sid)
        .fetch_one(pool)
        .await?;
    let paid: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT SUM(amount) FROM sale_payments WHERE sale_id = ?1), 0) + COALESCE((SELECT SUM(c.amount) FROM installment_collections c JOIN installment_agreements a ON a.id = c.agreement_id WHERE a.sale_id = ?1), 0)")
        .bind(&sid)
        .fetch_one(pool)
        .await?;
    let contract_total = r.6 + charges;
    type ItemRaw = (String, String, Option<String>, i64, i64, i64, i64, i64);
    let items: Vec<ItemRaw> = sqlx::query_as(
        "SELECT TRIM(b.name || ' ' || pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END || CASE WHEN v.capacity <> '' THEN ' · ' || v.capacity ELSE '' END), v.sku, s.serial_no, i.qty, i.unit_price, i.discount, i.tax, i.line_total
         FROM sale_items i JOIN product_variants v ON v.id = i.variant_id JOIN products pr ON pr.id = v.product_id JOIN brands b ON b.id = pr.brand_id LEFT JOIN product_serials s ON s.id = i.serial_id WHERE i.sale_id = ? ORDER BY i.rowid",
    )
    .bind(&sid)
    .fetch_all(pool)
    .await?;
    Ok(SaleDetail {
        sale: SaleRow { id: r.0, number: r.1, mode: r.2, sale_date: r.3, customer_id: r.4, customer: r.5, grand_total: contract_total, paid, due: contract_total - paid },
        subtotal: r.7,
        discount_total: r.8,
        tax_total: r.9,
        financing_charges: charges,
        notes: r.10,
        cashier: r.11,
        customer_phone: r.12,
        customer_address: r.13,
        items: items.into_iter().map(|i| SaleItemRow { label: i.0, sku: i.1, serial_no: i.2, qty: i.3, unit_price: i.4, discount: i.5, tax: i.6, line_total: i.7 }).collect(),
        payments: sqlx::query_as("SELECT id, receipt_no, method, amount, paid_on, reference, note, reverses_id FROM sale_payments WHERE sale_id = ? ORDER BY created_at, id").bind(&sid).fetch_all(pool).await?,
    })
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SerialChoice {
    pub id: String,
    pub serial_no: String,
}

/// Serial numbers currently available to sell for a variant (POS picker).
pub async fn available_serials(
    pool: &SqlitePool,
    p: &Principal,
    variant_id: Id,
) -> CoreResult<Vec<SerialChoice>> {
    p.require(perm::SALES_CREATE)?;
    Ok(sqlx::query_as("SELECT id, serial_no FROM product_serials WHERE variant_id = ? AND status = 'available' ORDER BY received_at, serial_no").bind(variant_id.to_string()).fetch_all(pool).await?)
}

#[derive(Debug, Serialize)]
pub struct PreviewLine {
    pub variant_id: String,
    pub unit_price: i64,
    pub discount: i64,
    pub tax: i64,
    pub line_total: i64,
}

#[derive(Debug, Serialize)]
pub struct SalePreview {
    pub subtotal: i64,
    pub discount_total: i64,
    pub tax_total: i64,
    pub grand_total: i64,
    pub lines: Vec<PreviewLine>,
}

/// Exact totals (price tier, discount, tax) for the cart without touching stock or books.
/// Uses the same pricing code as `create_sale`, so what the cashier sees is what is posted.
pub async fn preview_sale(
    pool: &SqlitePool,
    p: &Principal,
    si: &SaleIn,
) -> CoreResult<SalePreview> {
    match si.mode.as_str() {
        "retail" => p.require(perm::SALES_CREATE)?,
        "wholesale" => p.require(perm::SALES_WHOLESALE)?,
        "installment" => p.require(perm::INSTALLMENTS_CREATE)?,
        _ => return Err(CoreError::Validation("unknown sale mode".into())),
    }
    if si.items.iter().any(|l| l.discount > 0) {
        p.require(perm::SALES_DISCOUNT)?;
    }
    let mut conn = pool.acquire().await?;
    let tax = load_tax(&mut conn).await?;
    let priced = price_items(&mut conn, si, &tax).await?;
    Ok(SalePreview {
        subtotal: priced.iter().map(|x| x.unit_price * x.qty).sum(),
        discount_total: priced.iter().map(|x| x.discount).sum(),
        tax_total: priced.iter().map(|x| x.tax).sum(),
        grand_total: priced.iter().map(|x| x.line_total).sum(),
        lines: priced
            .into_iter()
            .map(|x| PreviewLine {
                variant_id: x.variant_id,
                unit_price: x.unit_price,
                discount: x.discount,
                tax: x.tax,
                line_total: x.line_total,
            })
            .collect(),
    })
}
