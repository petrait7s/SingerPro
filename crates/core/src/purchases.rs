//! Purchasing: drafts -> confirmed invoices -> (partial) receiving into stock -> payments/returns.
//!
//! Policy (documented in PROJECT_DECISIONS.md):
//! * The supplier payable (subtotal - discount) is recognised when the purchase is CONFIRMED.
//! * Landed costs (freight etc.) and the discount are allocated across items by line value and become
//!   part of inventory cost; landed costs are NOT added to the supplier payable (they are paid to others).
//! * Each unit's cost is a deterministic slice of its line's total cost (remainder on the last unit),
//!   so received-unit costs always add up exactly to the line total.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::catalogue::map_unique;
use crate::stock::{self, Bucket, MovementIn, SerialStatus};
use crate::suppliers::post_ledger;
use crate::{sequence, time, CoreError, CoreResult, Id, Money};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;

#[derive(Debug, Clone, Deserialize)]
pub struct LineIn {
    pub variant_id: String,
    pub qty: i64,
    pub unit_cost: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CostIn {
    pub label: String,
    pub amount: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PurchaseIn {
    pub supplier_id: String,
    pub supplier_invoice_no: Option<String>,
    pub order_date: String,
    pub discount: i64,
    pub notes: String,
    pub items: Vec<LineIn>,
    pub costs: Vec<CostIn>,
}

fn parse_date(label: &str, s: &str) -> CoreResult<String> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map(|d| d.to_string())
        .map_err(|_| CoreError::Validation(format!("{label} must be a date like 2026-01-31")))
}

struct Totals {
    subtotal: i64,
    landed: i64,
}

fn validate(pi: &PurchaseIn) -> CoreResult<(Totals, Option<String>, String)> {
    if pi.items.is_empty() {
        return Err(CoreError::Validation("add at least one item".into()));
    }
    let mut subtotal = Money::ZERO;
    let mut seen = HashSet::new();
    for l in &pi.items {
        if l.qty <= 0 {
            return Err(CoreError::Validation("quantity must be positive".into()));
        }
        if l.unit_cost < 0 {
            return Err(CoreError::Validation("unit cost cannot be negative".into()));
        }
        if !seen.insert(l.variant_id.clone()) {
            return Err(CoreError::Validation(
                "each product variant may appear on only one line".into(),
            ));
        }
        subtotal = subtotal.checked_add(Money::from_paisa(l.unit_cost).checked_mul_qty(l.qty)?)?;
    }
    let mut landed = Money::ZERO;
    for c in &pi.costs {
        if c.amount <= 0 || c.label.trim().is_empty() || c.label.chars().count() > 60 {
            return Err(CoreError::Validation(
                "each landed cost needs a label (<= 60 chars) and a positive amount".into(),
            ));
        }
        landed = landed.checked_add(Money::from_paisa(c.amount))?;
    }
    if pi.discount < 0 || pi.discount > subtotal.paisa() {
        return Err(CoreError::Validation(
            "discount must be between 0 and the subtotal".into(),
        ));
    }
    let inv = pi
        .supplier_invoice_no
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(i) = inv {
        if i.chars().count() > 60 || i.chars().any(|c| c.is_control()) {
            return Err(CoreError::Validation(
                "supplier invoice number: at most 60 characters".into(),
            ));
        }
    }
    if pi.notes.chars().count() > 1000 {
        return Err(CoreError::Validation(
            "notes: at most 1000 characters".into(),
        ));
    }
    let date = parse_date("order date", &pi.order_date)?;
    Ok((
        Totals {
            subtotal: subtotal.paisa(),
            landed: landed.paisa(),
        },
        inv.map(str::to_string),
        date,
    ))
}

async fn write_lines(
    conn: &mut SqliteConnection,
    purchase_id: &str,
    pi: &PurchaseIn,
) -> CoreResult<()> {
    for l in &pi.items {
        let ok: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_variants v JOIN products p ON p.id = v.product_id WHERE v.id = ? AND v.is_active = 1 AND p.is_active = 1")
            .bind(&l.variant_id)
            .fetch_one(&mut *conn)
            .await?;
        if ok == 0 {
            return Err(CoreError::Validation(
                "an item refers to an unknown or inactive product variant".into(),
            ));
        }
        sqlx::query("INSERT INTO purchase_items (id, purchase_id, variant_id, qty, unit_cost) VALUES (?, ?, ?, ?, ?)")
            .bind(Id::new().to_string())
            .bind(purchase_id)
            .bind(&l.variant_id)
            .bind(l.qty)
            .bind(l.unit_cost)
            .execute(&mut *conn)
            .await?;
    }
    for c in &pi.costs {
        sqlx::query(
            "INSERT INTO purchase_costs (id, purchase_id, label, amount) VALUES (?, ?, ?, ?)",
        )
        .bind(Id::new().to_string())
        .bind(purchase_id)
        .bind(c.label.trim())
        .bind(c.amount)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn require_active_supplier(conn: &mut SqliteConnection, id: &str) -> CoreResult<()> {
    let a: Option<i64> = sqlx::query_scalar("SELECT is_active FROM suppliers WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    match a {
        None => Err(CoreError::Validation("unknown supplier".into())),
        Some(0) => Err(CoreError::Validation("supplier is inactive".into())),
        Some(_) => Ok(()),
    }
}

pub async fn create_draft(pool: &SqlitePool, p: &Principal, pi: PurchaseIn) -> CoreResult<Id> {
    p.require(perm::PURCHASES_MANAGE)?;
    let (t, inv, date) = validate(&pi)?;
    let id = Id::new();
    let now = time::now_utc_string();
    let mut tx = pool.begin().await?;
    require_active_supplier(&mut tx, &pi.supplier_id).await?;
    sequence::ensure(&mut tx, "purchase", "PO-", 6).await?;
    let number = sequence::next(&mut tx, "purchase").await?;
    sqlx::query(
        "INSERT INTO purchases (id, number, supplier_id, supplier_invoice_no, order_date, status, discount, subtotal, landed_total, grand_total, notes, created_by, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'draft', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&number)
    .bind(&pi.supplier_id)
    .bind(&inv)
    .bind(&date)
    .bind(pi.discount)
    .bind(t.subtotal)
    .bind(t.landed)
    .bind(t.subtotal - pi.discount)
    .bind(pi.notes.trim())
    .bind(p.user_id.to_string())
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| map_unique(e, "a purchase with this supplier invoice number"))?;
    write_lines(&mut tx, &id.to_string(), &pi).await?;
    audit::log(
        &mut tx,
        p,
        "purchase.drafted",
        "purchase",
        &id.to_string(),
        serde_json::json!({"number": number, "grand_total": t.subtotal - pi.discount}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn status_of(conn: &mut SqliteConnection, id: &str) -> CoreResult<(String, String, String)> {
    let r: Option<(String, String, String)> =
        sqlx::query_as("SELECT status, supplier_id, number FROM purchases WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    r.ok_or_else(|| CoreError::NotFound("purchase".into()))
}

pub async fn update_draft(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    pi: PurchaseIn,
) -> CoreResult<()> {
    p.require(perm::PURCHASES_MANAGE)?;
    let (t, inv, date) = validate(&pi)?;
    let mut tx = pool.begin().await?;
    let (status, _, number) = status_of(&mut tx, &id.to_string()).await?;
    if status != "draft" {
        return Err(CoreError::Conflict(
            "only draft purchases can be edited".into(),
        ));
    }
    require_active_supplier(&mut tx, &pi.supplier_id).await?;
    sqlx::query("DELETE FROM purchase_items WHERE purchase_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM purchase_costs WHERE purchase_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE purchases SET supplier_id=?, supplier_invoice_no=?, order_date=?, discount=?, subtotal=?, landed_total=?, grand_total=?, notes=?, updated_at=? WHERE id=?")
        .bind(&pi.supplier_id)
        .bind(&inv)
        .bind(&date)
        .bind(pi.discount)
        .bind(t.subtotal)
        .bind(t.landed)
        .bind(t.subtotal - pi.discount)
        .bind(pi.notes.trim())
        .bind(time::now_utc_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, "a purchase with this supplier invoice number"))?;
    write_lines(&mut tx, &id.to_string(), &pi).await?;
    audit::log(
        &mut tx,
        p,
        "purchase.draft_updated",
        "purchase",
        &id.to_string(),
        serde_json::json!({"number": number, "grand_total": t.subtotal - pi.discount}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Locks in cost allocation and books the supplier payable. After this the purchase is immutable.
pub async fn confirm(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<()> {
    p.require(perm::PURCHASES_MANAGE)?;
    let pid = id.to_string();
    let mut tx = pool.begin().await?;
    let (status, supplier, number) = status_of(&mut tx, &pid).await?;
    if status != "draft" {
        return Err(CoreError::Conflict(format!("purchase is already {status}")));
    }
    require_active_supplier(&mut tx, &supplier).await?;
    let (discount, landed, grand): (i64, i64, i64) =
        sqlx::query_as("SELECT discount, landed_total, grand_total FROM purchases WHERE id = ?")
            .bind(&pid)
            .fetch_one(&mut *tx)
            .await?;
    let items: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT id, qty, unit_cost FROM purchase_items WHERE purchase_id = ? ORDER BY id",
    )
    .bind(&pid)
    .fetch_all(&mut *tx)
    .await?;
    if items.is_empty() {
        return Err(CoreError::Validation(
            "a purchase needs at least one item".into(),
        ));
    }
    let weights: Vec<i64> = {
        let w: Vec<i64> = items.iter().map(|(_, q, c)| q * c).collect();
        if w.iter().sum::<i64>() == 0 {
            items.iter().map(|(_, q, _)| *q).collect()
        } else {
            w
        }
    };
    let adj = Money::from_paisa(landed - discount).allocate_by_weights(&weights)?;
    for ((item_id, _, _), a) in items.iter().zip(adj) {
        sqlx::query("UPDATE purchase_items SET landed_alloc = ? WHERE id = ?")
            .bind(a.paisa())
            .bind(item_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE purchases SET status='confirmed', confirmed_at=?, updated_at=? WHERE id=?")
        .bind(time::now_utc_string())
        .bind(time::now_utc_string())
        .bind(&pid)
        .execute(&mut *tx)
        .await?;
    if grand > 0 {
        post_ledger(
            &mut tx,
            &supplier,
            "purchase",
            grand,
            "purchase",
            &pid,
            &number,
            Some(p.user_id),
        )
        .await?;
    }
    crate::accounting::post_purchase_confirmed(&mut tx, &pid).await?;
    audit::log(&mut tx, p, "purchase.confirmed", "purchase", &pid, serde_json::json!({"number": number, "payable": grand, "landed_total": landed, "discount": discount})).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn cancel(pool: &SqlitePool, p: &Principal, id: Id, reason: &str) -> CoreResult<()> {
    p.require(perm::PURCHASES_MANAGE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to cancel a purchase".into(),
        ));
    }
    let pid = id.to_string();
    let mut tx = pool.begin().await?;
    let (status, supplier, number) = status_of(&mut tx, &pid).await?;
    match status.as_str() {
        "cancelled" => return Err(CoreError::Conflict("purchase is already cancelled".into())),
        "draft" => {}
        _ => {
            let received: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(received_qty), 0) FROM purchase_items WHERE purchase_id = ?",
            )
            .bind(&pid)
            .fetch_one(&mut *tx)
            .await?;
            if received > 0 {
                return Err(CoreError::Conflict(
                    "goods were already received; use a purchase return instead".into(),
                ));
            }
            let paid: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount), 0) FROM supplier_payments WHERE purchase_id = ?",
            )
            .bind(&pid)
            .fetch_one(&mut *tx)
            .await?;
            if paid != 0 {
                return Err(CoreError::Conflict(
                    "payments exist for this purchase; reverse them first".into(),
                ));
            }
            let grand: i64 = sqlx::query_scalar("SELECT grand_total FROM purchases WHERE id = ?")
                .bind(&pid)
                .fetch_one(&mut *tx)
                .await?;
            if grand > 0 {
                post_ledger(
                    &mut tx,
                    &supplier,
                    "purchase_cancel",
                    -grand,
                    "purchase",
                    &pid,
                    &number,
                    Some(p.user_id),
                )
                .await?;
            }
        }
    }
    sqlx::query("UPDATE purchases SET status='cancelled', cancelled_at=?, cancel_reason=?, updated_at=? WHERE id=?")
        .bind(time::now_utc_string())
        .bind(reason.trim())
        .bind(time::now_utc_string())
        .bind(&pid)
        .execute(&mut *tx)
        .await?;
    crate::accounting::post_purchase_cancelled(&mut tx, &pid).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "purchase.cancelled",
            entity_type: "purchase",
            entity_id: Some(&pid),
            detail: Some(serde_json::json!({"number": number, "was": status})),
            reason: Some(reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------- receiving

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiveLine {
    pub item_id: String,
    pub qty: i64,
    /// Required (exactly `qty` unique numbers) for serial-tracked products; must be empty otherwise.
    pub serials: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReceiveIn {
    pub received_on: String,
    pub note: String,
    pub lines: Vec<ReceiveLine>,
}

pub async fn receive(pool: &SqlitePool, p: &Principal, id: Id, r: ReceiveIn) -> CoreResult<Id> {
    p.require(perm::PURCHASES_MANAGE)?;
    let pid = id.to_string();
    let received_on = parse_date("received date", &r.received_on)?;
    if r.lines.is_empty() || r.lines.iter().all(|l| l.qty == 0) {
        return Err(CoreError::Validation("enter a quantity to receive".into()));
    }
    let mut tx = pool.begin().await?;
    let (status, _, number) = status_of(&mut tx, &pid).await?;
    if status != "confirmed" {
        return Err(CoreError::Conflict(format!(
            "only confirmed purchases can be received (this one is {status})"
        )));
    }
    let receipt_id = Id::new();
    sqlx::query("INSERT INTO purchase_receipts (id, purchase_id, received_on, note, actor_id, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(receipt_id.to_string())
        .bind(&pid)
        .bind(&received_on)
        .bind(r.note.trim())
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await?;
    let mut seen_items = HashSet::new();
    let mut summary = vec![];
    for l in r.lines.iter().filter(|l| l.qty != 0) {
        if l.qty < 0 {
            return Err(CoreError::Validation("quantity cannot be negative".into()));
        }
        if !seen_items.insert(l.item_id.clone()) {
            return Err(CoreError::Validation(
                "an item appears twice in this receipt".into(),
            ));
        }
        let it: Option<(String, String, i64, i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT i.variant_id, i.id, i.qty, i.unit_cost, i.landed_alloc, i.received_qty, pr.is_serialized
             FROM purchase_items i JOIN product_variants v ON v.id = i.variant_id JOIN products pr ON pr.id = v.product_id
             WHERE i.id = ? AND i.purchase_id = ?",
        )
        .bind(&l.item_id)
        .bind(&pid)
        .fetch_optional(&mut *tx)
        .await?;
        let (variant, item_id, ordered, unit_cost, alloc, got, serialized) = it
            .ok_or_else(|| CoreError::Validation("item does not belong to this purchase".into()))?;
        if got + l.qty > ordered {
            return Err(CoreError::Conflict(format!(
                "cannot receive {} - only {} still outstanding",
                l.qty,
                ordered - got
            )));
        }
        if serialized == 1 {
            if l.serials.len() as i64 != l.qty {
                return Err(CoreError::Validation(format!(
                    "enter exactly {} serial number(s) for this serial-tracked item",
                    l.qty
                )));
            }
            let uniq: HashSet<String> = l.serials.iter().map(|s| s.trim().to_lowercase()).collect();
            if uniq.len() != l.serials.len() {
                return Err(CoreError::Validation(
                    "duplicate serial numbers in this receipt".into(),
                ));
            }
        } else if !l.serials.is_empty() {
            return Err(CoreError::Validation(
                "this product is not serial-tracked; leave serial numbers empty".into(),
            ));
        }
        // deterministic per-unit cost slices; remainder lands on the last ordered unit
        let total = Money::from_paisa(unit_cost)
            .checked_mul_qty(ordered)?
            .checked_add(Money::from_paisa(alloc))?;
        let slices = total.split_even_remainder_last(ordered as u32)?;
        let range = &slices[got as usize..(got + l.qty) as usize];
        let variant_id = Id::parse(&variant)?;
        let reason = format!("Received on {number}");
        let mut cost_sum = 0i64;
        if serialized == 1 {
            for (sn, c) in l.serials.iter().zip(range) {
                stock::receive_serial(
                    &mut tx,
                    variant_id,
                    sn,
                    c.paisa(),
                    Some(&item_id),
                    "purchase_receipt",
                    &receipt_id.to_string(),
                    &reason,
                    Some(p.user_id),
                )
                .await?;
                cost_sum += c.paisa();
            }
        } else {
            cost_sum = range.iter().map(|m| m.paisa()).sum();
            stock::post_movement(
                &mut tx,
                &MovementIn {
                    variant_id,
                    bucket: Bucket::Available,
                    qty_delta: l.qty,
                    cost_total: cost_sum,
                    serial_id: None,
                    doc_type: "purchase_receipt",
                    doc_id: &receipt_id.to_string(),
                    reason: &reason,
                    actor: Some(p.user_id),
                    allow_negative: false,
                },
            )
            .await?;
        }
        sqlx::query("UPDATE purchase_items SET received_qty = received_qty + ? WHERE id = ?")
            .bind(l.qty)
            .bind(&item_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO purchase_receipt_items (id, receipt_id, purchase_item_id, qty, cost_total) VALUES (?, ?, ?, ?, ?)")
            .bind(Id::new().to_string())
            .bind(receipt_id.to_string())
            .bind(&item_id)
            .bind(l.qty)
            .bind(cost_sum)
            .execute(&mut *tx)
            .await?;
        summary.push(serde_json::json!({"item": item_id, "qty": l.qty, "cost": cost_sum}));
    }
    crate::accounting::post_purchase_receipt(&mut tx, &receipt_id.to_string()).await?;
    audit::log(
        &mut tx,
        p,
        "purchase.received",
        "purchase",
        &pid,
        serde_json::json!({"number": number, "receipt": receipt_id.to_string(), "lines": summary}),
    )
    .await?;
    tx.commit().await?;
    Ok(receipt_id)
}

// ---------------------------------------------------------------- returns to supplier

#[derive(Debug, Clone, Deserialize)]
pub struct ReturnLine {
    pub item_id: String,
    pub qty: i64,
    pub serials: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReturnIn {
    pub returned_on: String,
    pub reason: String,
    pub lines: Vec<ReturnLine>,
}

/// Sends received goods back. Credit = returned units x original unit cost (landed costs are not refunded).
pub async fn return_to_supplier(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    r: ReturnIn,
) -> CoreResult<Id> {
    p.require(perm::PURCHASES_RETURN)?;
    if r.reason.trim().is_empty() {
        return Err(CoreError::Validation("a reason is required".into()));
    }
    let returned_on = parse_date("return date", &r.returned_on)?;
    if r.lines.is_empty() {
        return Err(CoreError::Validation("select items to return".into()));
    }
    let pid = id.to_string();
    let mut tx = pool.begin().await?;
    let (status, supplier, number) = status_of(&mut tx, &pid).await?;
    if status != "confirmed" {
        return Err(CoreError::Conflict(
            "only confirmed purchases can have returns".into(),
        ));
    }
    sequence::ensure(&mut tx, "purchase_return", "PRN-", 6).await?;
    let rnum = sequence::next(&mut tx, "purchase_return").await?;
    let rid = Id::new();
    let mut credit_total = 0i64;
    let mut lines_json = vec![];
    // header first (credit_total patched after lines) -- both inside the same transaction
    sqlx::query("INSERT INTO purchase_returns (id, number, purchase_id, returned_on, reason, credit_total, actor_id, created_at) VALUES (?, ?, ?, ?, ?, 0, ?, ?)")
        .bind(rid.to_string())
        .bind(&rnum)
        .bind(&pid)
        .bind(&returned_on)
        .bind(r.reason.trim())
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await?;
    let mut seen = HashSet::new();
    for l in &r.lines {
        if l.qty <= 0 || !seen.insert(l.item_id.clone()) {
            return Err(CoreError::Validation(
                "each returned item needs a positive quantity and may appear once".into(),
            ));
        }
        let it: Option<(String, i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT i.variant_id, i.unit_cost, i.received_qty - i.returned_qty, pr.is_serialized, 0 FROM purchase_items i
             JOIN product_variants v ON v.id = i.variant_id JOIN products pr ON pr.id = v.product_id WHERE i.id = ? AND i.purchase_id = ?",
        )
        .bind(&l.item_id)
        .bind(&pid)
        .fetch_optional(&mut *tx)
        .await?;
        let (variant, unit_cost, returnable, serialized, _) = it
            .ok_or_else(|| CoreError::Validation("item does not belong to this purchase".into()))?;
        if l.qty > returnable {
            return Err(CoreError::Conflict(format!(
                "only {returnable} unit(s) can still be returned for this item"
            )));
        }
        let variant_id = Id::parse(&variant)?;
        let reason = format!("Returned to supplier ({rnum})");
        let mut cost_total = 0i64;
        if serialized == 1 {
            if l.serials.len() as i64 != l.qty {
                return Err(CoreError::Validation(format!(
                    "select exactly {} serial number(s) to return",
                    l.qty
                )));
            }
            for sn in &l.serials {
                let row: Option<(String, String, i64)> = sqlx::query_as("SELECT id, status, cost FROM product_serials WHERE variant_id = ? AND serial_no = ? AND purchase_item_id = ?")
                    .bind(&variant)
                    .bind(sn.trim())
                    .bind(&l.item_id)
                    .fetch_optional(&mut *tx)
                    .await?;
                let (sid, st, cost) = row.ok_or_else(|| {
                    CoreError::Validation(format!(
                        "serial '{sn}' was not received on this purchase item"
                    ))
                })?;
                if !matches!(
                    SerialStatus::parse(&st)?,
                    SerialStatus::Available | SerialStatus::Damaged
                ) {
                    return Err(CoreError::Conflict(format!(
                        "serial '{sn}' is {st}; only available or damaged units can be returned"
                    )));
                }
                stock::transition_serial(
                    &mut tx,
                    Id::parse(&sid)?,
                    SerialStatus::ReturnedToSupplier,
                    "purchase_return",
                    &rid.to_string(),
                    &reason,
                    Some(p.user_id),
                )
                .await?;
                cost_total += cost;
            }
        } else {
            if !l.serials.is_empty() {
                return Err(CoreError::Validation(
                    "this product is not serial-tracked".into(),
                ));
            }
            cost_total = stock::avg_cost_out(&mut tx, variant_id, Bucket::Available, l.qty).await?;
            stock::post_movement(
                &mut tx,
                &MovementIn {
                    variant_id,
                    bucket: Bucket::Available,
                    qty_delta: -l.qty,
                    cost_total: -cost_total,
                    serial_id: None,
                    doc_type: "purchase_return",
                    doc_id: &rid.to_string(),
                    reason: &reason,
                    actor: Some(p.user_id),
                    allow_negative: false,
                },
            )
            .await?;
        }
        let credit = Money::from_paisa(unit_cost).checked_mul_qty(l.qty)?.paisa();
        credit_total += credit;
        sqlx::query("UPDATE purchase_items SET returned_qty = returned_qty + ? WHERE id = ?")
            .bind(l.qty)
            .bind(&l.item_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO purchase_return_items (id, return_id, purchase_item_id, qty, credit, cost_total) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(Id::new().to_string())
            .bind(rid.to_string())
            .bind(&l.item_id)
            .bind(l.qty)
            .bind(credit)
            .bind(cost_total)
            .execute(&mut *tx)
            .await?;
        lines_json.push(serde_json::json!({"item": l.item_id, "qty": l.qty, "credit": credit, "cost": cost_total}));
    }
    // purchase_returns rows are not append-only-protected; patch the computed credit once.
    sqlx::query("UPDATE purchase_returns SET credit_total = ? WHERE id = ?")
        .bind(credit_total)
        .bind(rid.to_string())
        .execute(&mut *tx)
        .await?;
    if credit_total > 0 {
        post_ledger(
            &mut tx,
            &supplier,
            "return",
            -credit_total,
            "purchase_return",
            &rid.to_string(),
            &rnum,
            Some(p.user_id),
        )
        .await?;
    }
    crate::accounting::post_purchase_return(&mut tx, &rid.to_string()).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry { actor_id: Some(p.user_id), action: "purchase.returned", entity_type: "purchase", entity_id: Some(&pid), detail: Some(serde_json::json!({"number": number, "return": rnum, "credit": credit_total, "lines": lines_json})), reason: Some(r.reason.trim()) },
    )
    .await?;
    tx.commit().await?;
    Ok(rid)
}

// ---------------------------------------------------------------- supplier payments

#[derive(Debug, Clone, Deserialize)]
pub struct PaymentIn {
    pub supplier_id: String,
    pub purchase_id: Option<String>,
    pub amount: i64,
    pub method: String,
    pub paid_on: String,
    pub reference: String,
    pub note: String,
    /// Idempotency key generated when the form opens; re-submitting the same key returns the original payment.
    pub client_key: String,
}

async fn purchase_due(conn: &mut SqliteConnection, pid: &str) -> CoreResult<i64> {
    let (grand, paid, credits): (i64, i64, i64) = sqlx::query_as(
        "SELECT grand_total, COALESCE((SELECT SUM(amount) FROM supplier_payments WHERE purchase_id = purchases.id), 0),
                COALESCE((SELECT SUM(credit_total) FROM purchase_returns WHERE purchase_id = purchases.id), 0)
         FROM purchases WHERE id = ?",
    )
    .bind(pid)
    .fetch_one(&mut *conn)
    .await?;
    Ok(grand - paid - credits)
}

pub async fn pay_supplier(pool: &SqlitePool, p: &Principal, pay: PaymentIn) -> CoreResult<Id> {
    p.require(perm::PURCHASES_PAY)?;
    if pay.amount <= 0 {
        return Err(CoreError::Validation(
            "payment amount must be positive".into(),
        ));
    }
    if !["cash", "bank", "mobile_wallet", "cheque", "other"].contains(&pay.method.as_str()) {
        return Err(CoreError::Validation("unknown payment method".into()));
    }
    let key = pay.client_key.trim();
    if key.is_empty() || key.len() > 64 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    let paid_on = parse_date("payment date", &pay.paid_on)?;
    let mut tx = pool.begin().await?;
    if let Some(existing) =
        sqlx::query_scalar::<_, String>("SELECT id FROM supplier_payments WHERE client_key = ?")
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?
    {
        return Id::parse(&existing); // duplicate submit: already recorded, do not post twice
    }
    let sup: Option<i64> = sqlx::query_scalar("SELECT is_active FROM suppliers WHERE id = ?")
        .bind(&pay.supplier_id)
        .fetch_optional(&mut *tx)
        .await?;
    if sup.is_none() {
        return Err(CoreError::Validation("unknown supplier".into()));
    }
    let mut note = pay.note.trim().to_string();
    if let Some(po) = &pay.purchase_id {
        let row: Option<(String, String, String)> =
            sqlx::query_as("SELECT supplier_id, status, number FROM purchases WHERE id = ?")
                .bind(po)
                .fetch_optional(&mut *tx)
                .await?;
        let (s, st, num) = row.ok_or_else(|| CoreError::Validation("unknown purchase".into()))?;
        if s != pay.supplier_id || st != "confirmed" {
            return Err(CoreError::Validation(
                "payment must target a confirmed purchase of this supplier".into(),
            ));
        }
        let due = purchase_due(&mut tx, po).await?;
        if pay.amount > due {
            return Err(CoreError::Validation(format!(
                "payment exceeds the amount due on {num} ({})",
                Money::from_paisa(due)
            )));
        }
        if note.is_empty() {
            note = num;
        }
    }
    let id = Id::new();
    sqlx::query("INSERT INTO supplier_payments (id, supplier_id, purchase_id, amount, method, paid_on, reference, note, client_key, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&pay.supplier_id)
        .bind(&pay.purchase_id)
        .bind(pay.amount)
        .bind(&pay.method)
        .bind(&paid_on)
        .bind(pay.reference.trim())
        .bind(&note)
        .bind(key)
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await?;
    post_ledger(
        &mut tx,
        &pay.supplier_id,
        "payment",
        -pay.amount,
        "supplier_payment",
        &id.to_string(),
        &note,
        Some(p.user_id),
    )
    .await?;
    crate::accounting::post_supplier_payment(&mut tx, &id.to_string()).await?;
    audit::log(&mut tx, p, "supplier.payment", "supplier_payment", &id.to_string(), serde_json::json!({"supplier": pay.supplier_id, "purchase": pay.purchase_id, "amount": pay.amount, "method": pay.method})).await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn reverse_payment(
    pool: &SqlitePool,
    p: &Principal,
    payment_id: Id,
    reason: &str,
) -> CoreResult<Id> {
    p.require(perm::PURCHASES_REVERSE_PAYMENT)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to reverse a payment".into(),
        ));
    }
    let oid = payment_id.to_string();
    let mut tx = pool.begin().await?;
    type PayRow = (String, Option<String>, i64, String, Option<String>);
    let row: Option<PayRow> =
        sqlx::query_as("SELECT supplier_id, purchase_id, amount, method, reverses_id FROM supplier_payments WHERE id = ?").bind(&oid).fetch_optional(&mut *tx).await?;
    let (supplier, purchase, amount, method, reverses) =
        row.ok_or_else(|| CoreError::NotFound("payment".into()))?;
    if reverses.is_some() || amount <= 0 {
        return Err(CoreError::Conflict("a reversal cannot be reversed".into()));
    }
    let id = Id::new();
    sqlx::query("INSERT INTO supplier_payments (id, supplier_id, purchase_id, amount, method, paid_on, reference, note, reverses_id, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, '', ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&supplier)
        .bind(&purchase)
        .bind(-amount)
        .bind(&method)
        .bind(time::today_dhaka().to_string())
        .bind(reason.trim())
        .bind(&oid)
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, "reversal of this payment"))?;
    post_ledger(
        &mut tx,
        &supplier,
        "payment_reversal",
        amount,
        "supplier_payment",
        &id.to_string(),
        reason.trim(),
        Some(p.user_id),
    )
    .await?;
    crate::accounting::post_supplier_payment(&mut tx, &id.to_string()).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "supplier.payment_reversed",
            entity_type: "supplier_payment",
            entity_id: Some(&oid),
            detail: Some(serde_json::json!({"amount": amount, "reversal": id.to_string()})),
            reason: Some(reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

// ---------------------------------------------------------------- queries

#[derive(Debug, Serialize)]
pub struct PurchaseRow {
    pub id: String,
    pub number: String,
    pub supplier_id: String,
    pub supplier: String,
    pub supplier_invoice_no: Option<String>,
    pub order_date: String,
    pub status: String,
    pub grand_total: i64,
    pub landed_total: i64,
    pub paid: i64,
    pub credits: i64,
    pub due: i64,
    pub ordered_qty: i64,
    pub received_qty: i64,
}

#[derive(Debug, Serialize)]
pub struct PurchasePage {
    pub total: i64,
    pub rows: Vec<PurchaseRow>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PurchaseFilter {
    pub status: Option<String>,
    pub supplier_id: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

const PURCHASE_COLS: &str = "p.id, p.number, p.supplier_id, s.name, p.supplier_invoice_no, p.order_date, p.status, p.grand_total, p.landed_total,
    COALESCE((SELECT SUM(amount) FROM supplier_payments WHERE purchase_id = p.id), 0),
    COALESCE((SELECT SUM(credit_total) FROM purchase_returns WHERE purchase_id = p.id), 0),
    COALESCE((SELECT SUM(qty) FROM purchase_items WHERE purchase_id = p.id), 0),
    COALESCE((SELECT SUM(received_qty) FROM purchase_items WHERE purchase_id = p.id), 0)";

type PRaw = (
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
);

fn prow(r: PRaw) -> PurchaseRow {
    let due = if r.6 == "confirmed" {
        r.7 - r.9 - r.10
    } else {
        0
    };
    PurchaseRow {
        id: r.0,
        number: r.1,
        supplier_id: r.2,
        supplier: r.3,
        supplier_invoice_no: r.4,
        order_date: r.5,
        status: r.6,
        grand_total: r.7,
        landed_total: r.8,
        paid: r.9,
        credits: r.10,
        due,
        ordered_qty: r.11,
        received_qty: r.12,
    }
}

pub async fn list_purchases(
    pool: &SqlitePool,
    p: &Principal,
    f: PurchaseFilter,
) -> CoreResult<PurchasePage> {
    p.require(perm::PURCHASES_VIEW)?;
    let like = f.search.as_deref().map(crate::catalogue::like_pattern);
    let w = "WHERE (?1 IS NULL OR p.status = ?1) AND (?2 IS NULL OR p.supplier_id = ?2) AND (?3 IS NULL OR p.number LIKE ?3 OR p.supplier_invoice_no LIKE ?3 OR s.name LIKE ?3)";
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM purchases p JOIN suppliers s ON s.id = p.supplier_id {w}"
    ))
    .bind(&f.status)
    .bind(&f.supplier_id)
    .bind(&like)
    .fetch_one(pool)
    .await?;
    let rows: Vec<PRaw> = sqlx::query_as(&format!("SELECT {PURCHASE_COLS} FROM purchases p JOIN suppliers s ON s.id = p.supplier_id {w} ORDER BY p.order_date DESC, p.number DESC LIMIT ?4 OFFSET ?5"))
        .bind(&f.status)
        .bind(&f.supplier_id)
        .bind(&like)
        .bind(f.limit.unwrap_or(50).clamp(1, 200))
        .bind(f.offset.unwrap_or(0).max(0))
        .fetch_all(pool)
        .await?;
    Ok(PurchasePage {
        total,
        rows: rows.into_iter().map(prow).collect(),
    })
}

#[derive(Debug, Serialize)]
pub struct ItemRow {
    pub id: String,
    pub variant_id: String,
    pub label: String,
    pub sku: String,
    pub is_serialized: bool,
    pub qty: i64,
    pub unit_cost: i64,
    pub landed_alloc: i64,
    pub received_qty: i64,
    pub returned_qty: i64,
    pub line_total: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CostRow {
    pub label: String,
    pub amount: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PaymentRow {
    pub id: String,
    pub amount: i64,
    pub method: String,
    pub paid_on: String,
    pub reference: String,
    pub note: String,
    pub reverses_id: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ReceiptRow {
    pub id: String,
    pub received_on: String,
    pub note: String,
    pub qty: i64,
    pub cost_total: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ReturnRow {
    pub id: String,
    pub number: String,
    pub returned_on: String,
    pub reason: String,
    pub credit_total: i64,
}

#[derive(Debug, Serialize)]
pub struct PurchaseDetail {
    pub purchase: PurchaseRow,
    pub notes: String,
    pub discount: i64,
    pub subtotal: i64,
    pub cancel_reason: Option<String>,
    pub items: Vec<ItemRow>,
    pub costs: Vec<CostRow>,
    pub payments: Vec<PaymentRow>,
    pub receipts: Vec<ReceiptRow>,
    pub returns: Vec<ReturnRow>,
}

pub async fn get_purchase(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<PurchaseDetail> {
    p.require(perm::PURCHASES_VIEW)?;
    let pid = id.to_string();
    let r: PRaw = sqlx::query_as(&format!("SELECT {PURCHASE_COLS} FROM purchases p JOIN suppliers s ON s.id = p.supplier_id WHERE p.id = ?"))
        .bind(&pid)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("purchase".into()))?;
    let (notes, discount, subtotal, cancel_reason): (String, i64, i64, Option<String>) =
        sqlx::query_as(
            "SELECT notes, discount, subtotal, cancel_reason FROM purchases WHERE id = ?",
        )
        .bind(&pid)
        .fetch_one(pool)
        .await?;
    type IRaw = (
        String,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    );
    let items: Vec<IRaw> = sqlx::query_as(
        "SELECT i.id, i.variant_id,
            TRIM(b.name || ' ' || pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END || CASE WHEN v.capacity <> '' THEN ' · ' || v.capacity ELSE '' END),
            v.sku, pr.is_serialized, i.qty, i.unit_cost, i.landed_alloc, i.received_qty, i.returned_qty, i.qty * i.unit_cost + i.landed_alloc
         FROM purchase_items i JOIN product_variants v ON v.id = i.variant_id JOIN products pr ON pr.id = v.product_id JOIN brands b ON b.id = pr.brand_id
         WHERE i.purchase_id = ? ORDER BY v.sku",
    )
    .bind(&pid)
    .fetch_all(pool)
    .await?;
    Ok(PurchaseDetail {
        purchase: prow(r),
        notes,
        discount,
        subtotal,
        cancel_reason,
        items: items
            .into_iter()
            .map(|i| ItemRow { id: i.0, variant_id: i.1, label: i.2, sku: i.3, is_serialized: i.4 == 1, qty: i.5, unit_cost: i.6, landed_alloc: i.7, received_qty: i.8, returned_qty: i.9, line_total: i.10 })
            .collect(),
        costs: sqlx::query_as("SELECT label, amount FROM purchase_costs WHERE purchase_id = ? ORDER BY label").bind(&pid).fetch_all(pool).await?,
        payments: sqlx::query_as("SELECT id, amount, method, paid_on, reference, note, reverses_id FROM supplier_payments WHERE purchase_id = ? ORDER BY created_at, id").bind(&pid).fetch_all(pool).await?,
        receipts: sqlx::query_as(
            "SELECT r.id, r.received_on, r.note, COALESCE(SUM(ri.qty), 0) AS qty, COALESCE(SUM(ri.cost_total), 0) AS cost_total
             FROM purchase_receipts r LEFT JOIN purchase_receipt_items ri ON ri.receipt_id = r.id WHERE r.purchase_id = ? GROUP BY r.id ORDER BY r.created_at",
        )
        .bind(&pid)
        .fetch_all(pool)
        .await?,
        returns: sqlx::query_as("SELECT id, number, returned_on, reason, credit_total FROM purchase_returns WHERE purchase_id = ? ORDER BY created_at").bind(&pid).fetch_all(pool).await?,
    })
}
