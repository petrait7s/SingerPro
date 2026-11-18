//! Inventory queries and manual stock operations (opening stock, adjustments, bucket moves, serial status).
//! Every operation is a numbered `stock_adjustments` document + immutable ledger movements + audit.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::stock::{self, Bucket, MovementIn, SerialStatus};
use crate::{sequence, time, CoreError, CoreResult, Id};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

// ---------------------------------------------------------------- queries

#[derive(Debug, Serialize)]
pub struct StockRow {
    pub variant_id: String,
    pub product_id: String,
    pub label: String,
    pub brand: String,
    pub category: String,
    pub sku: String,
    pub barcode: Option<String>,
    pub is_serialized: bool,
    pub min_stock: i64,
    pub available: i64,
    pub reserved: i64,
    pub damaged: i64,
    pub warranty: i64,
    pub pending_inspection: i64,
    pub value: i64,
    pub avg_cost: i64,
    pub low_stock: bool,
}

#[derive(Debug, Serialize)]
pub struct StockPage {
    pub total: i64,
    pub total_value: i64,
    pub rows: Vec<StockRow>,
}

#[derive(Debug, Default, Deserialize)]
pub struct StockFilter {
    pub search: Option<String>,
    pub low_only: bool,
    pub in_stock_only: bool,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

const BUCKET_SUM: &str =
    "COALESCE((SELECT qty FROM stock_balances WHERE variant_id = v.id AND bucket = '{b}'), 0)";

fn bsum(b: &str) -> String {
    BUCKET_SUM.replace("{b}", b)
}

pub async fn stock_levels(
    pool: &SqlitePool,
    p: &Principal,
    f: StockFilter,
) -> CoreResult<StockPage> {
    p.require(perm::INVENTORY_VIEW)?;
    let like = f.search.as_deref().map(crate::catalogue::like_pattern);
    let (av, rs, dm, wr, pi) = (
        bsum("available"),
        bsum("reserved"),
        bsum("damaged"),
        bsum("warranty"),
        bsum("pending_inspection"),
    );
    let value = "COALESCE((SELECT SUM(value) FROM stock_balances WHERE variant_id = v.id), 0)";
    let prod_av = "COALESCE((SELECT SUM(sb.qty) FROM stock_balances sb JOIN product_variants pv ON pv.id = sb.variant_id WHERE pv.product_id = p.id AND sb.bucket = 'available'), 0)";
    let base = format!(
        "FROM product_variants v JOIN products p ON p.id = v.product_id JOIN brands b ON b.id = p.brand_id JOIN categories c ON c.id = p.category_id
         WHERE (?1 IS NULL OR p.name LIKE ?1 OR p.model LIKE ?1 OR b.name LIKE ?1 OR v.sku LIKE ?1 OR v.barcode LIKE ?1)
           AND (?2 = 0 OR ({av} + {rs} + {dm} + {wr} + {pi}) > 0)
           AND (?3 = 0 OR (p.min_stock > 0 AND {prod_av} < p.min_stock))"
    );
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) {base}"))
        .bind(&like)
        .bind(f.in_stock_only as i64)
        .bind(f.low_only as i64)
        .fetch_one(pool)
        .await?;
    let total_value: i64 = sqlx::query_scalar(&format!("SELECT COALESCE(SUM({value}), 0) {base}"))
        .bind(&like)
        .bind(f.in_stock_only as i64)
        .bind(f.low_only as i64)
        .fetch_one(pool)
        .await?;
    type Raw = (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    );
    let rows: Vec<Raw> = sqlx::query_as(&format!(
        "SELECT v.id, p.id,
            TRIM(p.name || CASE WHEN p.model <> '' THEN ' ' || p.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END || CASE WHEN v.capacity <> '' THEN ' · ' || v.capacity ELSE '' END || CASE WHEN v.size <> '' THEN ' · ' || v.size ELSE '' END),
            b.name, c.name, v.sku, v.barcode, p.is_serialized, p.min_stock, {av}, {rs}, {dm}, {wr}, {pi}, {value}, {prod_av}
         {base} ORDER BY p.name COLLATE NOCASE, v.sku LIMIT ?4 OFFSET ?5"
    ))
    .bind(&like)
    .bind(f.in_stock_only as i64)
    .bind(f.low_only as i64)
    .bind(f.limit.unwrap_or(100).clamp(1, 500))
    .bind(f.offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await?;
    let rows = rows
        .into_iter()
        .map(|r| {
            let on_hand = r.9 + r.10 + r.11 + r.12 + r.13;
            StockRow {
                variant_id: r.0,
                product_id: r.1,
                label: r.2,
                brand: r.3,
                category: r.4,
                sku: r.5,
                barcode: r.6,
                is_serialized: r.7 == 1,
                min_stock: r.8,
                available: r.9,
                reserved: r.10,
                damaged: r.11,
                warranty: r.12,
                pending_inspection: r.13,
                value: r.14,
                avg_cost: if on_hand > 0 {
                    (r.14 + on_hand / 2) / on_hand
                } else {
                    0
                },
                low_stock: r.8 > 0 && r.15 < r.8,
            }
        })
        .collect();
    Ok(StockPage {
        total,
        total_value,
        rows,
    })
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MovementRow {
    pub id: String,
    pub at: String,
    pub bucket: String,
    pub qty_delta: i64,
    pub cost_total: i64,
    pub balance_after: i64,
    pub doc_type: String,
    pub doc_id: String,
    pub reason: String,
    pub serial_no: Option<String>,
    pub actor: Option<String>,
}

pub async fn movements(
    pool: &SqlitePool,
    p: &Principal,
    variant_id: Id,
    limit: i64,
    offset: i64,
) -> CoreResult<Vec<MovementRow>> {
    p.require(perm::INVENTORY_VIEW)?;
    Ok(sqlx::query_as(
        "SELECT m.id, m.at, m.bucket, m.qty_delta, m.cost_total, m.balance_after, m.doc_type, m.doc_id, m.reason, s.serial_no, u.username AS actor
         FROM stock_movements m LEFT JOIN product_serials s ON s.id = m.serial_id LEFT JOIN users u ON u.id = m.actor_id
         WHERE m.variant_id = ? ORDER BY m.at DESC, m.id DESC LIMIT ? OFFSET ?",
    )
    .bind(variant_id.to_string())
    .bind(limit.clamp(1, 500))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await?)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SerialRow {
    pub id: String,
    pub variant_id: String,
    pub serial_no: String,
    pub status: String,
    pub cost: i64,
    pub label: String,
    pub sku: String,
    pub received_at: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct SerialFilter {
    pub variant_id: Option<String>,
    pub search: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn serials(
    pool: &SqlitePool,
    p: &Principal,
    f: SerialFilter,
) -> CoreResult<Vec<SerialRow>> {
    p.require(perm::INVENTORY_VIEW)?;
    if let Some(s) = &f.status {
        SerialStatus::parse(s)?;
    }
    let like = f.search.as_deref().map(crate::catalogue::like_pattern);
    Ok(sqlx::query_as(
        "SELECT s.id, s.variant_id, s.serial_no, s.status, s.cost,
            TRIM(p.name || CASE WHEN p.model <> '' THEN ' ' || p.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END) AS label, v.sku, s.received_at
         FROM product_serials s JOIN product_variants v ON v.id = s.variant_id JOIN products p ON p.id = v.product_id
         WHERE (?1 IS NULL OR s.variant_id = ?1) AND (?2 IS NULL OR s.serial_no LIKE ?2) AND (?3 IS NULL OR s.status = ?3)
         ORDER BY s.received_at DESC, s.serial_no LIMIT ?4 OFFSET ?5",
    )
    .bind(&f.variant_id)
    .bind(like)
    .bind(&f.status)
    .bind(f.limit.unwrap_or(100).clamp(1, 500))
    .bind(f.offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await?)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SerialEventRow {
    pub at: String,
    pub from_status: Option<String>,
    pub to_status: String,
    pub doc_type: String,
    pub doc_id: String,
    pub reason: String,
    pub actor: Option<String>,
}

pub async fn serial_history(
    pool: &SqlitePool,
    p: &Principal,
    serial_id: Id,
) -> CoreResult<Vec<SerialEventRow>> {
    p.require(perm::INVENTORY_VIEW)?;
    Ok(sqlx::query_as(
        "SELECT e.at, e.from_status, e.to_status, e.doc_type, e.doc_id, e.reason, u.username AS actor
         FROM serial_events e LEFT JOIN users u ON u.id = e.actor_id WHERE e.serial_id = ? ORDER BY e.at, e.id",
    )
    .bind(serial_id.to_string())
    .fetch_all(pool)
    .await?)
}

// ---------------------------------------------------------------- operations

struct VariantInfo {
    serialized: bool,
    sku: String,
}

async fn variant_info(conn: &mut SqliteConnection, id: &str) -> CoreResult<VariantInfo> {
    let r: Option<(i64, String, i64, i64)> = sqlx::query_as("SELECT p.is_serialized, v.sku, v.is_active, p.is_active FROM product_variants v JOIN products p ON p.id = v.product_id WHERE v.id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    let (s, sku, va, pa) = r.ok_or_else(|| CoreError::NotFound("product variant".into()))?;
    if va == 0 || pa == 0 {
        return Err(CoreError::Validation(format!("{sku} is inactive")));
    }
    Ok(VariantInfo {
        serialized: s == 1,
        sku,
    })
}

async fn new_adjustment(
    conn: &mut SqliteConnection,
    p: &Principal,
    kind: &str,
    variant: &str,
    summary: &str,
    reason: &str,
) -> CoreResult<(Id, String)> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 300 {
        return Err(CoreError::Validation(
            "a reason (max 300 characters) is required".into(),
        ));
    }
    sequence::ensure(&mut *conn, "adjustment", "ADJ-", 6).await?;
    let number = sequence::next(&mut *conn, "adjustment").await?;
    let id = Id::new();
    sqlx::query("INSERT INTO stock_adjustments (id, number, kind, variant_id, summary, reason, at, actor_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&number)
        .bind(kind)
        .bind(variant)
        .bind(summary)
        .bind(reason)
        .bind(time::now_utc_string())
        .bind(p.user_id.to_string())
        .execute(&mut *conn)
        .await?;
    Ok((id, number))
}

#[derive(Debug, Deserialize)]
pub struct OpeningStockIn {
    pub variant_id: String,
    /// Units for non-serialized products (ignored for serialized: one unit per serial number).
    pub qty: i64,
    pub unit_cost: i64,
    pub serials: Vec<String>,
    pub reason: String,
}

/// Opening balance for a variant that has no stock history yet.
pub async fn record_opening_stock(
    pool: &SqlitePool,
    p: &Principal,
    o: OpeningStockIn,
) -> CoreResult<Id> {
    p.require(perm::INVENTORY_ADJUST)?;
    if o.unit_cost < 0 {
        return Err(CoreError::Validation("unit cost cannot be negative".into()));
    }
    let mut tx = pool.begin().await?;
    let vi = variant_info(&mut tx, &o.variant_id).await?;
    let prior: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stock_movements WHERE variant_id = ?")
            .bind(&o.variant_id)
            .fetch_one(&mut *tx)
            .await?;
    if prior > 0 {
        return Err(CoreError::Conflict("opening stock can only be recorded before any stock movement; use an adjustment or a purchase".into()));
    }
    let variant = Id::parse(&o.variant_id)?;
    let qty = if vi.serialized {
        o.serials.len() as i64
    } else {
        o.qty
    };
    if qty <= 0 {
        return Err(CoreError::Validation(
            "enter a quantity (or serial numbers) greater than zero".into(),
        ));
    }
    let (id, number) = new_adjustment(
        &mut tx,
        p,
        "opening",
        &o.variant_id,
        &format!("Opening stock {qty} x {}", vi.sku),
        &o.reason,
    )
    .await?;
    if vi.serialized {
        let mut uniq = std::collections::HashSet::new();
        for sn in &o.serials {
            if !uniq.insert(sn.trim().to_lowercase()) {
                return Err(CoreError::Validation("duplicate serial numbers".into()));
            }
            stock::receive_serial(
                &mut tx,
                variant,
                sn,
                o.unit_cost,
                None,
                "stock_adjustment",
                &id.to_string(),
                &format!("Opening stock ({number})"),
                Some(p.user_id),
            )
            .await?;
        }
    } else {
        if !o.serials.is_empty() {
            return Err(CoreError::Validation(
                "this product is not serial-tracked".into(),
            ));
        }
        let cost = crate::Money::from_paisa(o.unit_cost)
            .checked_mul_qty(qty)?
            .paisa();
        stock::post_movement(
            &mut tx,
            &MovementIn {
                variant_id: variant,
                bucket: Bucket::Available,
                qty_delta: qty,
                cost_total: cost,
                serial_id: None,
                doc_type: "stock_adjustment",
                doc_id: &id.to_string(),
                reason: &format!("Opening stock ({number})"),
                actor: Some(p.user_id),
                allow_negative: false,
            },
        )
        .await?;
    }
    crate::accounting::post_stock_adjustment(&mut tx, &id.to_string()).await?;
    audit::log(&mut tx, p, "stock.opening", "stock_adjustment", &id.to_string(), serde_json::json!({"number": number, "variant": o.variant_id, "qty": qty, "unit_cost": o.unit_cost})).await?;
    tx.commit().await?;
    Ok(id)
}

#[derive(Debug, Deserialize)]
pub struct AdjustIn {
    pub variant_id: String,
    pub bucket: String,
    /// Signed. Positive needs `unit_cost`; negative is valued at weighted-average cost.
    pub qty_delta: i64,
    pub unit_cost: Option<i64>,
    pub reason: String,
}

/// Count correction for NON-serialized products. Serialized units change via serial status instead.
pub async fn adjust_quantity(pool: &SqlitePool, p: &Principal, a: AdjustIn) -> CoreResult<Id> {
    p.require(perm::INVENTORY_ADJUST)?;
    if a.qty_delta == 0 {
        return Err(CoreError::Validation(
            "adjustment quantity cannot be zero".into(),
        ));
    }
    let bucket = Bucket::parse(&a.bucket)?;
    let mut tx = pool.begin().await?;
    let vi = variant_info(&mut tx, &a.variant_id).await?;
    if vi.serialized {
        return Err(CoreError::Validation(
            "serial-tracked units are adjusted one by one via serial status, not by quantity"
                .into(),
        ));
    }
    let variant = Id::parse(&a.variant_id)?;
    let cost = if a.qty_delta > 0 {
        let uc = a.unit_cost.ok_or_else(|| {
            CoreError::Validation("a unit cost is required when adding stock".into())
        })?;
        if uc < 0 {
            return Err(CoreError::Validation("unit cost cannot be negative".into()));
        }
        crate::Money::from_paisa(uc)
            .checked_mul_qty(a.qty_delta)?
            .paisa()
    } else {
        -stock::avg_cost_out(&mut tx, variant, bucket, -a.qty_delta).await?
    };
    let kind = if a.qty_delta < 0 {
        "write_off"
    } else {
        "adjustment"
    };
    let (id, number) = new_adjustment(
        &mut tx,
        p,
        kind,
        &a.variant_id,
        &format!(
            "{:+} {} in {} ({})",
            a.qty_delta,
            vi.sku,
            bucket.as_str(),
            a.reason.trim()
        ),
        &a.reason,
    )
    .await?;
    stock::post_movement(
        &mut tx,
        &MovementIn {
            variant_id: variant,
            bucket,
            qty_delta: a.qty_delta,
            cost_total: cost,
            serial_id: None,
            doc_type: "stock_adjustment",
            doc_id: &id.to_string(),
            reason: a.reason.trim(),
            actor: Some(p.user_id),
            allow_negative: false,
        },
    )
    .await?;
    crate::accounting::post_stock_adjustment(&mut tx, &id.to_string()).await?;
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "stock.adjusted", entity_type: "stock_adjustment", entity_id: Some(&id.to_string()), detail: Some(serde_json::json!({"number": number, "variant": a.variant_id, "bucket": bucket.as_str(), "qty_delta": a.qty_delta, "cost": cost})), reason: Some(a.reason.trim()) }).await?;
    tx.commit().await?;
    Ok(id)
}

#[derive(Debug, Deserialize)]
pub struct MoveIn {
    pub variant_id: String,
    pub from_bucket: String,
    pub to_bucket: String,
    pub qty: i64,
    pub reason: String,
}

/// Re-classifies non-serialized units between buckets (e.g. available -> damaged) at average cost.
pub async fn move_quantity(pool: &SqlitePool, p: &Principal, m: MoveIn) -> CoreResult<Id> {
    p.require(perm::INVENTORY_ADJUST)?;
    let (from, to) = (Bucket::parse(&m.from_bucket)?, Bucket::parse(&m.to_bucket)?);
    if from == to || m.qty <= 0 {
        return Err(CoreError::Validation(
            "choose two different buckets and a positive quantity".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    let vi = variant_info(&mut tx, &m.variant_id).await?;
    if vi.serialized {
        return Err(CoreError::Validation(
            "serial-tracked units move via serial status".into(),
        ));
    }
    let variant = Id::parse(&m.variant_id)?;
    let cost = stock::avg_cost_out(&mut tx, variant, from, m.qty).await?;
    let (id, number) = new_adjustment(
        &mut tx,
        p,
        "transfer",
        &m.variant_id,
        &format!(
            "{} x {} {} -> {}",
            m.qty,
            vi.sku,
            from.as_str(),
            to.as_str()
        ),
        &m.reason,
    )
    .await?;
    let reason = m.reason.trim();
    stock::post_movement(
        &mut tx,
        &MovementIn {
            variant_id: variant,
            bucket: from,
            qty_delta: -m.qty,
            cost_total: -cost,
            serial_id: None,
            doc_type: "stock_adjustment",
            doc_id: &id.to_string(),
            reason,
            actor: Some(p.user_id),
            allow_negative: false,
        },
    )
    .await?;
    stock::post_movement(
        &mut tx,
        &MovementIn {
            variant_id: variant,
            bucket: to,
            qty_delta: m.qty,
            cost_total: cost,
            serial_id: None,
            doc_type: "stock_adjustment",
            doc_id: &id.to_string(),
            reason,
            actor: Some(p.user_id),
            allow_negative: false,
        },
    )
    .await?;
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "stock.moved", entity_type: "stock_adjustment", entity_id: Some(&id.to_string()), detail: Some(serde_json::json!({"number": number, "from": from.as_str(), "to": to.as_str(), "qty": m.qty})), reason: Some(reason) }).await?;
    tx.commit().await?;
    Ok(id)
}

/// Manual serial status change (damage, repair, warranty service, write-off). Sale / return /
/// inspection transitions are reserved for their own workflows.
pub async fn change_serial_status(
    pool: &SqlitePool,
    p: &Principal,
    serial_id: Id,
    to: &str,
    reason: &str,
) -> CoreResult<Id> {
    p.require(perm::INVENTORY_ADJUST)?;
    let to = SerialStatus::parse(to)?;
    let mut tx = pool.begin().await?;
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT variant_id, status, serial_no FROM product_serials WHERE id = ?")
            .bind(serial_id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    let (variant, status, serial_no) = row.ok_or_else(|| CoreError::NotFound("serial".into()))?;
    let from = SerialStatus::parse(&status)?;
    if !stock::manual_transition_allowed(from, to) {
        return Err(CoreError::Conflict(format!(
            "serial {serial_no}: '{}' -> '{}' is not a manual transition",
            from.as_str(),
            to.as_str()
        )));
    }
    let (id, number) = new_adjustment(
        &mut tx,
        p,
        "serial_status",
        &variant,
        &format!("{serial_no}: {} -> {}", from.as_str(), to.as_str()),
        reason,
    )
    .await?;
    stock::transition_serial(
        &mut tx,
        serial_id,
        to,
        "stock_adjustment",
        &id.to_string(),
        reason,
        Some(p.user_id),
    )
    .await?;
    crate::accounting::post_stock_adjustment(&mut tx, &id.to_string()).await?;
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "serial.status_changed", entity_type: "serial", entity_id: Some(&serial_id.to_string()), detail: Some(serde_json::json!({"number": number, "serial_no": serial_no, "from": from.as_str(), "to": to.as_str()})), reason: Some(reason.trim()) }).await?;
    tx.commit().await?;
    Ok(id)
}
