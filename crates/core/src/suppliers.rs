//! Supplier profiles and the supplier ledger (append-only; positive = we owe the supplier).

use crate::audit;
use crate::authz::{perm, Principal};
use crate::catalogue::map_unique;
use crate::{time, CoreError, CoreResult, Id};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Clone, Deserialize)]
pub struct SupplierIn {
    pub name: String,
    pub contact_person: String,
    pub phone: String,
    pub alt_phone: String,
    pub email: String,
    pub address: String,
    pub notes: String,
}

#[derive(Debug, Serialize)]
pub struct SupplierRow {
    pub id: String,
    pub name: String,
    pub contact_person: String,
    pub phone: String,
    pub alt_phone: String,
    pub email: String,
    pub address: String,
    pub notes: String,
    pub is_active: bool,
    /// Amount we currently owe (ledger sum). Negative = supplier owes us / advance paid.
    pub balance: i64,
}

fn clean(label: &str, v: &str, max: usize, required: bool) -> CoreResult<String> {
    let t = v.trim();
    if required && t.is_empty() {
        return Err(CoreError::Validation(format!("{label} is required")));
    }
    if t.chars().count() > max || t.chars().any(|c| c.is_control() && c != '\n') {
        return Err(CoreError::Validation(format!(
            "{label}: at most {max} characters"
        )));
    }
    Ok(t.to_string())
}

pub async fn create_supplier(pool: &SqlitePool, p: &Principal, s: SupplierIn) -> CoreResult<Id> {
    p.require(perm::SUPPLIERS_MANAGE)?;
    let name = clean("supplier name", &s.name, 100, true)?;
    let id = Id::new();
    let now = time::now_utc_string();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO suppliers (id, name, contact_person, phone, alt_phone, email, address, notes, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&name)
    .bind(clean("contact person", &s.contact_person, 100, false)?)
    .bind(clean("phone", &s.phone, 40, false)?)
    .bind(clean("alternative phone", &s.alt_phone, 40, false)?)
    .bind(clean("email", &s.email, 100, false)?)
    .bind(clean("address", &s.address, 300, false)?)
    .bind(clean("notes", &s.notes, 1000, false)?)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| map_unique(e, "supplier"))?;
    audit::log(
        &mut tx,
        p,
        "supplier.created",
        "supplier",
        &id.to_string(),
        serde_json::json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update_supplier(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    s: SupplierIn,
) -> CoreResult<()> {
    p.require(perm::SUPPLIERS_MANAGE)?;
    let name = clean("supplier name", &s.name, 100, true)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE suppliers SET name=?, contact_person=?, phone=?, alt_phone=?, email=?, address=?, notes=?, updated_at=? WHERE id=?")
        .bind(&name)
        .bind(clean("contact person", &s.contact_person, 100, false)?)
        .bind(clean("phone", &s.phone, 40, false)?)
        .bind(clean("alternative phone", &s.alt_phone, 40, false)?)
        .bind(clean("email", &s.email, 100, false)?)
        .bind(clean("address", &s.address, 300, false)?)
        .bind(clean("notes", &s.notes, 1000, false)?)
        .bind(time::now_utc_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, "supplier"))?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("supplier".into()));
    }
    audit::log(
        &mut tx,
        p,
        "supplier.updated",
        "supplier",
        &id.to_string(),
        serde_json::json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_supplier_active(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::SUPPLIERS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE suppliers SET is_active = ?, updated_at = ? WHERE id = ?")
        .bind(active as i64)
        .bind(time::now_utc_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("supplier".into()));
    }
    audit::log(
        &mut tx,
        p,
        if active {
            "supplier.activated"
        } else {
            "supplier.deactivated"
        },
        "supplier",
        &id.to_string(),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn list_suppliers(
    pool: &SqlitePool,
    p: &Principal,
    search: Option<&str>,
    include_inactive: bool,
) -> CoreResult<Vec<SupplierRow>> {
    p.require(perm::SUPPLIERS_VIEW)?;
    let like = search.map(|s| format!("%{}%", s.trim().replace(['%', '_'], "")));
    type Raw = (
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
    );
    let rows: Vec<Raw> = sqlx::query_as(
        "SELECT s.id, s.name, s.contact_person, s.phone, s.alt_phone, s.email, s.address, s.notes, s.is_active,
            COALESCE((SELECT SUM(amount) FROM supplier_ledger_entries WHERE supplier_id = s.id), 0)
         FROM suppliers s WHERE (?1 IS NULL OR s.name LIKE ?1 OR s.phone LIKE ?1 OR s.contact_person LIKE ?1) AND (?2 = 1 OR s.is_active = 1)
         ORDER BY s.name COLLATE NOCASE",
    )
    .bind(like)
    .bind(include_inactive as i64)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| SupplierRow {
            id: r.0,
            name: r.1,
            contact_person: r.2,
            phone: r.3,
            alt_phone: r.4,
            email: r.5,
            address: r.6,
            notes: r.7,
            is_active: r.8 == 1,
            balance: r.9,
        })
        .collect())
}

#[derive(Debug, Serialize)]
pub struct LedgerLine {
    pub at: String,
    pub kind: String,
    pub doc_type: String,
    pub doc_id: String,
    pub note: String,
    pub amount: i64,
    pub balance: i64,
}

#[derive(Debug, Serialize)]
pub struct Statement {
    pub supplier_id: String,
    pub supplier: String,
    pub lines: Vec<LedgerLine>,
    pub closing_balance: i64,
}

pub async fn statement(pool: &SqlitePool, p: &Principal, supplier_id: Id) -> CoreResult<Statement> {
    p.require(perm::SUPPLIERS_VIEW)?;
    let name: String = sqlx::query_scalar("SELECT name FROM suppliers WHERE id = ?")
        .bind(supplier_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("supplier".into()))?;
    let rows: Vec<(String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT at, kind, doc_type, doc_id, note, amount FROM supplier_ledger_entries WHERE supplier_id = ? ORDER BY at, id",
    )
    .bind(supplier_id.to_string())
    .fetch_all(pool)
    .await?;
    let mut bal = 0i64;
    let lines = rows
        .into_iter()
        .map(|(at, kind, doc_type, doc_id, note, amount)| {
            bal += amount;
            LedgerLine {
                at,
                kind,
                doc_type,
                doc_id,
                note,
                amount,
                balance: bal,
            }
        })
        .collect();
    Ok(Statement {
        supplier_id: supplier_id.to_string(),
        supplier: name,
        lines,
        closing_balance: bal,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn post_ledger(
    conn: &mut SqliteConnection,
    supplier_id: &str,
    kind: &str,
    amount: i64,
    doc_type: &str,
    doc_id: &str,
    note: &str,
    actor: Option<Id>,
) -> CoreResult<()> {
    sqlx::query("INSERT INTO supplier_ledger_entries (id, supplier_id, at, kind, amount, doc_type, doc_id, note, actor_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(Id::new().to_string())
        .bind(supplier_id)
        .bind(time::now_utc_string())
        .bind(kind)
        .bind(amount)
        .bind(doc_type)
        .bind(doc_id)
        .bind(note)
        .bind(actor.map(|a| a.to_string()))
        .execute(&mut *conn)
        .await
        .map_err(|e| map_unique(e, "ledger entry for this document"))?;
    Ok(())
}

pub async fn balance(pool: &SqlitePool, supplier_id: Id) -> CoreResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0) FROM supplier_ledger_entries WHERE supplier_id = ?",
    )
    .bind(supplier_id.to_string())
    .fetch_one(pool)
    .await?)
}
