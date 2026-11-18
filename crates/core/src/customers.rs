//! Customer profiles, customer ledger (append-only; positive = customer owes us) and statements.

use crate::accounting::{self, code, cr, dr, PostIn};
use crate::audit;
use crate::authz::{perm, Principal};
use crate::{sequence, time, CoreError, CoreResult, Id};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Clone, Deserialize)]
pub struct CustomerIn {
    pub name: String,
    pub phone: String,
    pub alt_phone: String,
    pub address: String,
    /// retail | wholesale | dealer
    pub customer_type: String,
    pub credit_limit: i64,
    /// Only accepted when the shop has enabled `customers.collect_nid`.
    pub nid: String,
    pub notes: String,
}

#[derive(Debug, Serialize)]
pub struct CustomerRow {
    pub id: String,
    pub code: String,
    pub name: String,
    pub phone: String,
    pub alt_phone: String,
    pub address: String,
    pub customer_type: String,
    pub credit_limit: i64,
    pub nid: String,
    pub notes: String,
    pub is_active: bool,
    pub registered_on: String,
    /// What the customer currently owes us.
    pub balance: i64,
    pub price_tier_id: Option<String>,
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

async fn nid_enabled(conn: &mut SqliteConnection) -> CoreResult<bool> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'customers.collect_nid'",
    )
    .fetch_optional(&mut *conn)
    .await?
    .as_deref()
        == Some("true"))
}

async fn validate(
    conn: &mut SqliteConnection,
    c: &CustomerIn,
) -> CoreResult<(String, String, String, String, String, String)> {
    if !["retail", "wholesale", "dealer"].contains(&c.customer_type.as_str()) {
        return Err(CoreError::Validation(
            "customer type must be retail, wholesale or dealer".into(),
        ));
    }
    if c.credit_limit < 0 {
        return Err(CoreError::Validation(
            "credit limit cannot be negative".into(),
        ));
    }
    let nid = clean("national ID", &c.nid, 30, false)?;
    if !nid.is_empty() && !nid_enabled(conn).await? {
        return Err(CoreError::Validation("national ID collection is disabled; enable it in Settings only if you have a lawful need".into()));
    }
    Ok((
        clean("name", &c.name, 100, true)?,
        clean("phone", &c.phone, 40, false)?,
        clean("alternative phone", &c.alt_phone, 40, false)?,
        clean("address", &c.address, 300, false)?,
        clean("notes", &c.notes, 1000, false)?,
        nid,
    ))
}

pub async fn create_customer(pool: &SqlitePool, p: &Principal, c: CustomerIn) -> CoreResult<Id> {
    p.require(perm::CUSTOMERS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let (name, phone, alt, address, notes, nid) = validate(&mut tx, &c).await?;
    sequence::ensure(&mut tx, "customer", "CUS-", 5).await?;
    let code = sequence::next(&mut tx, "customer").await?;
    let id = Id::new();
    let now = time::now_utc_string();
    sqlx::query("INSERT INTO customers (id, code, name, phone, alt_phone, address, customer_type, credit_limit, nid, notes, registered_on, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&code)
        .bind(&name)
        .bind(&phone)
        .bind(&alt)
        .bind(&address)
        .bind(&c.customer_type)
        .bind(c.credit_limit)
        .bind(&nid)
        .bind(&notes)
        .bind(time::today_dhaka().to_string())
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    audit::log(
        &mut tx,
        p,
        "customer.created",
        "customer",
        &id.to_string(),
        serde_json::json!({"code": code, "name": name, "type": c.customer_type}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update_customer(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    c: CustomerIn,
) -> CoreResult<()> {
    p.require(perm::CUSTOMERS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let (name, phone, alt, address, notes, nid) = validate(&mut tx, &c).await?;
    let old: Option<(i64, String)> =
        sqlx::query_as("SELECT credit_limit, customer_type FROM customers WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    let (old_limit, old_type) = old.ok_or_else(|| CoreError::NotFound("customer".into()))?;
    // A credit limit is a financial control: changing it needs the credit-override permission.
    if old_limit != c.credit_limit && !p.has(perm::SALES_CREDIT_OVERRIDE) {
        return Err(CoreError::Forbidden(perm::SALES_CREDIT_OVERRIDE.into()));
    }
    sqlx::query("UPDATE customers SET name=?, phone=?, alt_phone=?, address=?, customer_type=?, credit_limit=?, nid=?, notes=?, updated_at=? WHERE id=?")
        .bind(&name)
        .bind(&phone)
        .bind(&alt)
        .bind(&address)
        .bind(&c.customer_type)
        .bind(c.credit_limit)
        .bind(&nid)
        .bind(&notes)
        .bind(time::now_utc_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    audit::log(&mut tx, p, if old_limit != c.credit_limit { "customer.credit_limit_changed" } else { "customer.updated" }, "customer", &id.to_string(), serde_json::json!({"credit_limit": {"before": old_limit, "after": c.credit_limit}, "type": {"before": old_type, "after": c.customer_type}})).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_customer_active(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::CUSTOMERS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE customers SET is_active = ?, updated_at = ? WHERE id = ?")
        .bind(active as i64)
        .bind(time::now_utc_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("customer".into()));
    }
    audit::log(
        &mut tx,
        p,
        if active {
            "customer.activated"
        } else {
            "customer.deactivated"
        },
        "customer",
        &id.to_string(),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

type Raw = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    i64,
    String,
    i64,
    Option<String>,
);
const SELECT: &str = "SELECT c.id, c.code, c.name, c.phone, c.alt_phone, c.address, c.customer_type, c.credit_limit, c.nid, c.notes, c.is_active, c.registered_on,
    COALESCE((SELECT SUM(amount) FROM customer_ledger_entries WHERE customer_id = c.id), 0), c.price_tier_id FROM customers c";

fn row(r: Raw, show_nid: bool) -> CustomerRow {
    CustomerRow {
        id: r.0,
        code: r.1,
        name: r.2,
        phone: r.3,
        alt_phone: r.4,
        address: r.5,
        customer_type: r.6,
        credit_limit: r.7,
        nid: if show_nid { r.8 } else { String::new() },
        notes: r.9,
        is_active: r.10 == 1,
        registered_on: r.11,
        balance: r.12,
        price_tier_id: r.13,
    }
}

/// The national ID is only returned to users who may manage customers.
pub async fn list_customers(
    pool: &SqlitePool,
    p: &Principal,
    search: Option<&str>,
    customer_type: Option<&str>,
    include_inactive: bool,
    with_balance_only: bool,
) -> CoreResult<Vec<CustomerRow>> {
    p.require(perm::CUSTOMERS_VIEW)?;
    let like = search.map(crate::catalogue::like_pattern);
    let rows: Vec<Raw> = sqlx::query_as(&format!(
        "{SELECT} WHERE (?1 IS NULL OR c.name LIKE ?1 OR c.phone LIKE ?1 OR c.alt_phone LIKE ?1 OR c.code LIKE ?1) AND (?2 IS NULL OR c.customer_type = ?2) AND (?3 = 1 OR c.is_active = 1)
           AND (?4 = 0 OR COALESCE((SELECT SUM(amount) FROM customer_ledger_entries WHERE customer_id = c.id), 0) > 0)
         ORDER BY c.name COLLATE NOCASE LIMIT 500"
    ))
    .bind(like)
    .bind(customer_type)
    .bind(include_inactive as i64)
    .bind(with_balance_only as i64)
    .fetch_all(pool)
    .await?;
    let show = p.has(perm::CUSTOMERS_MANAGE);
    Ok(rows.into_iter().map(|r| row(r, show)).collect())
}

pub async fn get_customer(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<CustomerRow> {
    p.require(perm::CUSTOMERS_VIEW)?;
    let r: Raw = sqlx::query_as(&format!("{SELECT} WHERE c.id = ?"))
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("customer".into()))?;
    Ok(row(r, p.has(perm::CUSTOMERS_MANAGE)))
}

#[derive(Debug, Serialize)]
pub struct StatementLine {
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
    pub customer: CustomerRow,
    pub lines: Vec<StatementLine>,
    pub closing_balance: i64,
}

pub async fn statement(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<Statement> {
    let customer = get_customer(pool, p, id).await?;
    let rows: Vec<(String, String, String, String, String, i64)> = sqlx::query_as("SELECT at, kind, doc_type, doc_id, note, amount FROM customer_ledger_entries WHERE customer_id = ? ORDER BY at, id").bind(id.to_string()).fetch_all(pool).await?;
    let mut bal = 0;
    let lines = rows
        .into_iter()
        .map(|(at, kind, doc_type, doc_id, note, amount)| {
            bal += amount;
            StatementLine {
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
        customer,
        lines,
        closing_balance: bal,
    })
}

pub async fn balance(conn: &mut SqliteConnection, customer_id: &str) -> CoreResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0) FROM customer_ledger_entries WHERE customer_id = ?",
    )
    .bind(customer_id)
    .fetch_one(&mut *conn)
    .await?)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn post_ledger(
    conn: &mut SqliteConnection,
    customer_id: &str,
    kind: &str,
    amount: i64,
    doc_type: &str,
    doc_id: &str,
    note: &str,
    actor: Option<Id>,
) -> CoreResult<()> {
    sqlx::query("INSERT INTO customer_ledger_entries (id, customer_id, at, kind, amount, doc_type, doc_id, note, actor_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(Id::new().to_string())
        .bind(customer_id)
        .bind(time::now_utc_string())
        .bind(kind)
        .bind(amount)
        .bind(doc_type)
        .bind(doc_id)
        .bind(note)
        .bind(actor.map(|a| a.to_string()))
        .execute(&mut *conn)
        .await
        .map_err(|e| crate::catalogue::map_unique(e, "customer ledger entry for this document"))?;
    Ok(())
}

/// Amount the customer already owed before the system was used. Once per customer.
pub async fn opening_balance(
    pool: &SqlitePool,
    p: &Principal,
    customer_id: Id,
    amount: i64,
    date: &str,
) -> CoreResult<()> {
    p.require(perm::ACCOUNTS_MANAGE)?;
    if amount <= 0 {
        return Err(CoreError::Validation(
            "opening balance must be positive".into(),
        ));
    }
    let cid = customer_id.to_string();
    let mut tx = pool.begin().await?;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers WHERE id = ?")
        .bind(&cid)
        .fetch_one(&mut *tx)
        .await?;
    if exists == 0 {
        return Err(CoreError::NotFound("customer".into()));
    }
    post_ledger(
        &mut tx,
        &cid,
        "opening",
        amount,
        "customer_opening",
        &cid,
        "Opening balance",
        Some(p.user_id),
    )
    .await?;
    let key = format!("customer_opening:{cid}");
    accounting::post(
        &mut tx,
        &PostIn {
            key: &key,
            date,
            memo: "Customer opening balance",
            source_type: "customer_opening",
            source_id: &key,
            lines: vec![
                dr(code::AR, amount, "Opening receivable"),
                cr(code::OPENING_EQUITY, amount, "Opening balance"),
            ],
            reverses: None,
            actor: Some(p.user_id),
        },
    )
    .await?;
    audit::log(
        &mut tx,
        p,
        "customer.opening_balance",
        "customer",
        &cid,
        serde_json::json!({"amount": amount}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
