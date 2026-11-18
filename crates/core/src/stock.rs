//! Stock ledger primitives: immutable movements, cached balances, serial lifecycle, reconciliation.
//! Everything here takes the caller's transaction so stock and its source document commit together.

use crate::authz::{perm, Principal};
use crate::{time, CoreError, CoreResult, Id};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Available,
    Reserved,
    Damaged,
    Warranty,
    PendingInspection,
}

impl Bucket {
    pub const ALL: [Bucket; 5] = [
        Bucket::Available,
        Bucket::Reserved,
        Bucket::Damaged,
        Bucket::Warranty,
        Bucket::PendingInspection,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Available => "available",
            Bucket::Reserved => "reserved",
            Bucket::Damaged => "damaged",
            Bucket::Warranty => "warranty",
            Bucket::PendingInspection => "pending_inspection",
        }
    }

    pub fn parse(s: &str) -> CoreResult<Bucket> {
        Bucket::ALL
            .into_iter()
            .find(|b| b.as_str() == s)
            .ok_or_else(|| CoreError::Validation(format!("unknown stock bucket: {s}")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SerialStatus {
    Available,
    Reserved,
    Sold,
    ReturnedPendingInspection,
    Damaged,
    WarrantyService,
    WrittenOff,
    ReturnedToSupplier,
}

impl SerialStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SerialStatus::Available => "available",
            SerialStatus::Reserved => "reserved",
            SerialStatus::Sold => "sold",
            SerialStatus::ReturnedPendingInspection => "returned_pending_inspection",
            SerialStatus::Damaged => "damaged",
            SerialStatus::WarrantyService => "warranty_service",
            SerialStatus::WrittenOff => "written_off",
            SerialStatus::ReturnedToSupplier => "returned_to_supplier",
        }
    }

    pub fn parse(s: &str) -> CoreResult<SerialStatus> {
        use SerialStatus::*;
        [
            Available,
            Reserved,
            Sold,
            ReturnedPendingInspection,
            Damaged,
            WarrantyService,
            WrittenOff,
            ReturnedToSupplier,
        ]
        .into_iter()
        .find(|x| x.as_str() == s)
        .ok_or_else(|| CoreError::Validation(format!("unknown serial status: {s}")))
    }

    /// Quantity bucket a unit in this status is counted in (`None` = not on hand).
    pub fn bucket(self) -> Option<Bucket> {
        match self {
            SerialStatus::Available => Some(Bucket::Available),
            SerialStatus::Reserved => Some(Bucket::Reserved),
            SerialStatus::Damaged => Some(Bucket::Damaged),
            SerialStatus::WarrantyService => Some(Bucket::Warranty),
            SerialStatus::ReturnedPendingInspection => Some(Bucket::PendingInspection),
            SerialStatus::Sold | SerialStatus::WrittenOff | SerialStatus::ReturnedToSupplier => {
                None
            }
        }
    }
}

/// The complete lifecycle graph.
pub fn transition_allowed(from: SerialStatus, to: SerialStatus) -> bool {
    use SerialStatus::*;
    matches!(
        (from, to),
        (
            Available,
            Reserved | Damaged | WarrantyService | Sold | WrittenOff | ReturnedToSupplier
        ) | (Reserved, Available | Sold)
            | (Sold, ReturnedPendingInspection)
            | (ReturnedPendingInspection, Available | Damaged | WrittenOff)
            | (
                Damaged,
                Available | WrittenOff | ReturnedToSupplier | WarrantyService
            )
            | (WarrantyService, Available | Damaged | WrittenOff)
    )
}

/// Transitions a user may request by hand. Sale/return/inspection moves belong to those workflows.
pub fn manual_transition_allowed(from: SerialStatus, to: SerialStatus) -> bool {
    use SerialStatus::*;
    transition_allowed(from, to)
        && ![from, to]
            .iter()
            .any(|s| matches!(s, Sold | ReturnedPendingInspection | ReturnedToSupplier))
}

pub struct MovementIn<'a> {
    pub variant_id: Id,
    pub bucket: Bucket,
    pub qty_delta: i64,
    pub cost_total: i64,
    pub serial_id: Option<Id>,
    pub doc_type: &'a str,
    pub doc_id: &'a str,
    pub reason: &'a str,
    pub actor: Option<Id>,
    pub allow_negative: bool,
}

/// Appends a movement and updates the cached balance atomically (in the caller's transaction).
pub async fn post_movement(conn: &mut SqliteConnection, m: &MovementIn<'_>) -> CoreResult<i64> {
    if m.qty_delta == 0 {
        return Err(CoreError::Validation(
            "movement quantity cannot be zero".into(),
        ));
    }
    if m.reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "every stock movement needs a reason".into(),
        ));
    }
    let v = m.variant_id.to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO stock_balances (variant_id, bucket, qty, value) VALUES (?, ?, 0, 0)",
    )
    .bind(&v)
    .bind(m.bucket.as_str())
    .execute(&mut *conn)
    .await?;
    let qty: i64 =
        sqlx::query_scalar("SELECT qty FROM stock_balances WHERE variant_id = ? AND bucket = ?")
            .bind(&v)
            .bind(m.bucket.as_str())
            .fetch_one(&mut *conn)
            .await?;
    let new_qty = qty.checked_add(m.qty_delta).ok_or(CoreError::Overflow)?;
    if new_qty < 0 && !m.allow_negative {
        return Err(CoreError::Conflict(format!(
            "insufficient stock in '{}': have {qty}, need {}",
            m.bucket.as_str(),
            -m.qty_delta
        )));
    }
    sqlx::query("UPDATE stock_balances SET qty = qty + ?, value = value + ? WHERE variant_id = ? AND bucket = ?")
        .bind(m.qty_delta)
        .bind(m.cost_total)
        .bind(&v)
        .bind(m.bucket.as_str())
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO stock_movements (id, at, variant_id, bucket, qty_delta, cost_total, serial_id, doc_type, doc_id, reason, actor_id, balance_after)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Id::new().to_string())
    .bind(time::now_utc_string())
    .bind(&v)
    .bind(m.bucket.as_str())
    .bind(m.qty_delta)
    .bind(m.cost_total)
    .bind(m.serial_id.map(|s| s.to_string()))
    .bind(m.doc_type)
    .bind(m.doc_id)
    .bind(m.reason.trim())
    .bind(m.actor.map(|a| a.to_string()))
    .bind(new_qty)
    .execute(&mut *conn)
    .await?;
    Ok(new_qty)
}

/// Cost (positive paisa) to remove `take` units from a bucket at weighted-average cost.
/// Removing the whole bucket removes its whole value, so no rounding residue is ever left behind.
pub async fn avg_cost_out(
    conn: &mut SqliteConnection,
    variant: Id,
    bucket: Bucket,
    take: i64,
) -> CoreResult<i64> {
    let row: Option<(i64, i64)> =
        sqlx::query_as("SELECT qty, value FROM stock_balances WHERE variant_id = ? AND bucket = ?")
            .bind(variant.to_string())
            .bind(bucket.as_str())
            .fetch_optional(&mut *conn)
            .await?;
    let (qty, value) = row.unwrap_or((0, 0));
    if qty <= 0 || take <= 0 {
        return Ok(0);
    }
    if take >= qty {
        return Ok(value);
    }
    let num = (value as i128) * (take as i128);
    let q = (num.abs() * 2 + qty as i128) / (qty as i128 * 2);
    i64::try_from(q * if num < 0 { -1 } else { 1 }).map_err(|_| CoreError::Overflow)
}

pub async fn on_hand(conn: &mut SqliteConnection, variant: Id, bucket: Bucket) -> CoreResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT qty FROM stock_balances WHERE variant_id = ? AND bucket = ?")
            .bind(variant.to_string())
            .bind(bucket.as_str())
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(0),
    )
}

/// Moves a serial through its lifecycle, keeping quantity buckets and the event log in step.
pub async fn transition_serial(
    conn: &mut SqliteConnection,
    serial_id: Id,
    to: SerialStatus,
    doc_type: &str,
    doc_id: &str,
    reason: &str,
    actor: Option<Id>,
) -> CoreResult<()> {
    let row: Option<(String, String, i64, String)> = sqlx::query_as(
        "SELECT variant_id, status, cost, serial_no FROM product_serials WHERE id = ?",
    )
    .bind(serial_id.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    let (variant, status, cost, serial_no) =
        row.ok_or_else(|| CoreError::NotFound("serial".into()))?;
    let from = SerialStatus::parse(&status)?;
    if !transition_allowed(from, to) {
        return Err(CoreError::Conflict(format!(
            "serial {serial_no}: cannot go from '{}' to '{}'",
            from.as_str(),
            to.as_str()
        )));
    }
    let variant = Id::parse(&variant)?;
    if let Some(b) = from.bucket() {
        post_movement(
            conn,
            &MovementIn {
                variant_id: variant,
                bucket: b,
                qty_delta: -1,
                cost_total: -cost,
                serial_id: Some(serial_id),
                doc_type,
                doc_id,
                reason,
                actor,
                allow_negative: false,
            },
        )
        .await?;
    }
    if let Some(b) = to.bucket() {
        post_movement(
            conn,
            &MovementIn {
                variant_id: variant,
                bucket: b,
                qty_delta: 1,
                cost_total: cost,
                serial_id: Some(serial_id),
                doc_type,
                doc_id,
                reason,
                actor,
                allow_negative: false,
            },
        )
        .await?;
    }
    sqlx::query("UPDATE product_serials SET status = ? WHERE id = ?")
        .bind(to.as_str())
        .bind(serial_id.to_string())
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO serial_events (id, serial_id, at, from_status, to_status, doc_type, doc_id, reason, actor_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Id::new().to_string())
    .bind(serial_id.to_string())
    .bind(time::now_utc_string())
    .bind(from.as_str())
    .bind(to.as_str())
    .bind(doc_type)
    .bind(doc_id)
    .bind(reason.trim())
    .bind(actor.map(|a| a.to_string()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// A rejected return: the unit goes back to the customer (pending-inspection -> sold).
/// This is the one transition outside the normal graph; it is only reachable from `reject_return`.
pub async fn release_to_customer(
    conn: &mut SqliteConnection,
    serial_id: Id,
    doc_type: &str,
    doc_id: &str,
    reason: &str,
    actor: Option<Id>,
) -> CoreResult<()> {
    let row: Option<(String, String, i64)> =
        sqlx::query_as("SELECT variant_id, status, cost FROM product_serials WHERE id = ?")
            .bind(serial_id.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    let (variant, status, cost) = row.ok_or_else(|| CoreError::NotFound("serial".into()))?;
    if status != "returned_pending_inspection" {
        return Err(CoreError::Conflict(format!(
            "serial is {status}, not awaiting inspection"
        )));
    }
    post_movement(
        conn,
        &MovementIn {
            variant_id: Id::parse(&variant)?,
            bucket: Bucket::PendingInspection,
            qty_delta: -1,
            cost_total: -cost,
            serial_id: Some(serial_id),
            doc_type,
            doc_id,
            reason,
            actor,
            allow_negative: false,
        },
    )
    .await?;
    sqlx::query("UPDATE product_serials SET status = 'sold' WHERE id = ?")
        .bind(serial_id.to_string())
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO serial_events (id, serial_id, at, from_status, to_status, doc_type, doc_id, reason, actor_id) VALUES (?, ?, ?, 'returned_pending_inspection', 'sold', ?, ?, ?, ?)")
        .bind(Id::new().to_string())
        .bind(serial_id.to_string())
        .bind(time::now_utc_string())
        .bind(doc_type)
        .bind(doc_id)
        .bind(reason.trim())
        .bind(actor.map(|a| a.to_string()))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Creates a serial already in `Available` (receiving / opening stock) with its +1 movement.
#[allow(clippy::too_many_arguments)]
pub async fn receive_serial(
    conn: &mut SqliteConnection,
    variant: Id,
    serial_no: &str,
    cost: i64,
    purchase_item_id: Option<&str>,
    doc_type: &str,
    doc_id: &str,
    reason: &str,
    actor: Option<Id>,
) -> CoreResult<Id> {
    let sn = serial_no.trim();
    if sn.is_empty() || sn.chars().any(|c| c.is_control()) || sn.len() > 64 {
        return Err(CoreError::Validation(format!(
            "invalid serial number: '{serial_no}'"
        )));
    }
    let id = Id::new();
    let now = time::now_utc_string();
    sqlx::query(
        "INSERT INTO product_serials (id, variant_id, serial_no, status, cost, purchase_item_id, received_at)
         VALUES (?, ?, ?, 'available', ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(variant.to_string())
    .bind(sn)
    .bind(cost)
    .bind(purchase_item_id)
    .bind(&now)
    .execute(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            CoreError::Conflict(format!("serial number '{sn}' already exists for this product"))
        }
        other => CoreError::Database(other),
    })?;
    post_movement(
        conn,
        &MovementIn {
            variant_id: variant,
            bucket: Bucket::Available,
            qty_delta: 1,
            cost_total: cost,
            serial_id: Some(id),
            doc_type,
            doc_id,
            reason,
            actor,
            allow_negative: false,
        },
    )
    .await?;
    sqlx::query(
        "INSERT INTO serial_events (id, serial_id, at, from_status, to_status, doc_type, doc_id, reason, actor_id)
         VALUES (?, ?, ?, NULL, 'available', ?, ?, ?, ?)",
    )
    .bind(Id::new().to_string())
    .bind(id.to_string())
    .bind(&now)
    .bind(doc_type)
    .bind(doc_id)
    .bind(reason.trim())
    .bind(actor.map(|a| a.to_string()))
    .execute(&mut *conn)
    .await?;
    Ok(id)
}

#[derive(Debug, Serialize)]
pub struct Discrepancy {
    pub variant_id: String,
    pub sku: String,
    pub bucket: String,
    pub check: String,
    pub expected: i64,
    pub actual: i64,
}

/// Read-only integrity check: ledger vs cached balances, and serial counts/costs vs quantity buckets.
/// An empty result means stock is fully reconciled.
pub async fn reconcile(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<Discrepancy>> {
    p.require(perm::INVENTORY_VIEW)?;
    let mut out = Vec::new();
    // 1. ledger vs cache (both directions)
    let rows: Vec<(String, String, String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT x.variant_id, v.sku, x.bucket, x.q, x.c, COALESCE(b.qty, 0), COALESCE(b.value, 0)
         FROM (SELECT variant_id, bucket, SUM(qty_delta) q, SUM(cost_total) c FROM stock_movements GROUP BY variant_id, bucket) x
         JOIN product_variants v ON v.id = x.variant_id
         LEFT JOIN stock_balances b ON b.variant_id = x.variant_id AND b.bucket = x.bucket
         UNION ALL
         SELECT b.variant_id, v.sku, b.bucket, 0, 0, b.qty, b.value FROM stock_balances b
         JOIN product_variants v ON v.id = b.variant_id
         WHERE NOT EXISTS (SELECT 1 FROM stock_movements m WHERE m.variant_id = b.variant_id AND m.bucket = b.bucket)",
    )
    .fetch_all(pool)
    .await?;
    for (variant_id, sku, bucket, lq, lc, bq, bv) in rows {
        if lq != bq {
            out.push(Discrepancy {
                variant_id: variant_id.clone(),
                sku: sku.clone(),
                bucket: bucket.clone(),
                check: "ledger_qty_vs_cached_qty".into(),
                expected: lq,
                actual: bq,
            });
        }
        if lc != bv {
            out.push(Discrepancy {
                variant_id,
                sku,
                bucket,
                check: "ledger_value_vs_cached_value".into(),
                expected: lc,
                actual: bv,
            });
        }
    }
    // 2. serialized units vs quantity buckets
    let serialized: Vec<(String, String)> = sqlx::query_as(
        "SELECT v.id, v.sku FROM product_variants v JOIN products p ON p.id = v.product_id WHERE p.is_serialized = 1",
    )
    .fetch_all(pool)
    .await?;
    for (vid, sku) in serialized {
        for b in Bucket::ALL {
            let status = [
                SerialStatus::Available,
                SerialStatus::Reserved,
                SerialStatus::Damaged,
                SerialStatus::WarrantyService,
                SerialStatus::ReturnedPendingInspection,
            ]
            .into_iter()
            .find(|s| s.bucket() == Some(b))
            .expect("mapped");
            let (n, c): (i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(cost), 0) FROM product_serials WHERE variant_id = ? AND status = ?")
                .bind(&vid)
                .bind(status.as_str())
                .fetch_one(pool)
                .await?;
            let (bq, bv): (i64, i64) = sqlx::query_as(
                "SELECT qty, value FROM stock_balances WHERE variant_id = ? AND bucket = ?",
            )
            .bind(&vid)
            .bind(b.as_str())
            .fetch_optional(pool)
            .await?
            .unwrap_or((0, 0));
            if n != bq {
                out.push(Discrepancy {
                    variant_id: vid.clone(),
                    sku: sku.clone(),
                    bucket: b.as_str().into(),
                    check: "serial_count_vs_quantity".into(),
                    expected: n,
                    actual: bq,
                });
            }
            if c != bv {
                out.push(Discrepancy {
                    variant_id: vid.clone(),
                    sku: sku.clone(),
                    bucket: b.as_str().into(),
                    check: "serial_cost_vs_value".into(),
                    expected: c,
                    actual: bv,
                });
            }
        }
    }
    Ok(out)
}
