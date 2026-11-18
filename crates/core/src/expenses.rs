//! Operating expenses with approval. Approved expenses post Dr expense account / Cr cash-bank-wallet.

use crate::accounting::{self, cr, dr, method_account, PostIn};
use crate::audit;
use crate::authz::{perm, Principal};
use crate::{sequence, time, CoreError, CoreResult, Id, Money};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

const DEFAULT_CATEGORIES: &[(&str, &str)] = &[
    ("Rent", "6000"),
    ("Salaries and wages", "6010"),
    ("Utilities", "6020"),
    ("Transport and delivery", "6030"),
    ("Marketing", "6040"),
    ("Repairs and maintenance", "6050"),
    ("Office and administration", "6060"),
    ("Bank charges", "6070"),
    ("Miscellaneous", "6900"),
];

pub async fn seed_categories(pool: &SqlitePool) -> CoreResult<()> {
    for (name, acct) in DEFAULT_CATEGORIES {
        sqlx::query("INSERT OR IGNORE INTO expense_categories (id, name, account_code, created_at) VALUES (?, ?, ?, ?)").bind(Id::new().to_string()).bind(name).bind(acct).bind(time::now_utc_string()).execute(pool).await?;
    }
    Ok(())
}

fn can_view(p: &Principal) -> CoreResult<()> {
    if p.has(perm::EXPENSES_CREATE) || p.has(perm::EXPENSES_APPROVE) || p.has(perm::ACCOUNTS_VIEW) {
        Ok(())
    } else {
        Err(CoreError::Forbidden(perm::EXPENSES_CREATE.into()))
    }
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub account_code: String,
    pub is_active: bool,
}

pub async fn list_categories(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<CategoryRow>> {
    can_view(p)?;
    Ok(sqlx::query_as(
        "SELECT id, name, account_code, is_active FROM expense_categories ORDER BY name",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn create_category(
    pool: &SqlitePool,
    p: &Principal,
    name: &str,
    account_code: &str,
) -> CoreResult<Id> {
    p.require(perm::EXPENSES_APPROVE)?;
    let name = name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err(CoreError::Validation(
            "category name must be 1-60 characters".into(),
        ));
    }
    let ok: Option<(String, i64)> =
        sqlx::query_as("SELECT type, is_active FROM accounts WHERE code = ?")
            .bind(account_code)
            .fetch_optional(pool)
            .await?;
    match ok {
        Some((t, 1)) if t == "expense" => {}
        _ => {
            return Err(CoreError::Validation(
                "choose an active expense account".into(),
            ))
        }
    }
    let id = Id::new();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO expense_categories (id, name, account_code, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(account_code)
    .bind(time::now_utc_string())
    .execute(&mut *tx)
    .await
    .map_err(|e| crate::catalogue::map_unique(e, "expense category"))?;
    audit::log(
        &mut tx,
        p,
        "expense.category_created",
        "expense_category",
        &id.to_string(),
        serde_json::json!({"name": name, "account": account_code}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn set_category_active(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::EXPENSES_APPROVE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE expense_categories SET is_active = ? WHERE id = ?")
        .bind(active as i64)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("category".into()));
    }
    audit::log(
        &mut tx,
        p,
        if active {
            "expense.category_activated"
        } else {
            "expense.category_deactivated"
        },
        "expense_category",
        &id.to_string(),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ExpenseIn {
    pub category_id: String,
    pub expense_date: String,
    pub amount: i64,
    pub method: String,
    pub payee: String,
    pub description: String,
    pub client_key: String,
}

/// Creating needs `expenses.create`. Users who also hold `expenses.approve` post immediately;
/// everyone else's expenses wait as `pending` for an approver.
pub async fn create_expense(pool: &SqlitePool, p: &Principal, e: ExpenseIn) -> CoreResult<Id> {
    p.require(perm::EXPENSES_CREATE)?;
    if e.amount <= 0 {
        return Err(CoreError::Validation("amount must be positive".into()));
    }
    if !["cash", "bank", "mobile_wallet", "cheque", "other"].contains(&e.method.as_str()) {
        return Err(CoreError::Validation("unknown payment method".into()));
    }
    let date = NaiveDate::parse_from_str(e.expense_date.trim(), "%Y-%m-%d")
        .map_err(|_| CoreError::Validation("date must be YYYY-MM-DD".into()))?;
    if date > time::today_dhaka() {
        return Err(CoreError::Validation(
            "an expense cannot be dated in the future".into(),
        ));
    }
    if e.payee.chars().count() > 100
        || e.description.chars().count() > 300
        || e.description.trim().is_empty()
    {
        return Err(CoreError::Validation(
            "a description (max 300 characters) is required".into(),
        ));
    }
    let key = e.client_key.trim();
    if key.is_empty() || key.len() > 64 {
        return Err(CoreError::Validation("missing submission key".into()));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if let Some(existing) =
        sqlx::query_scalar::<_, String>("SELECT id FROM expenses WHERE client_key = ?")
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?
    {
        return Id::parse(&existing);
    }
    let cat: Option<i64> =
        sqlx::query_scalar("SELECT is_active FROM expense_categories WHERE id = ?")
            .bind(&e.category_id)
            .fetch_optional(&mut *tx)
            .await?;
    match cat {
        None => return Err(CoreError::Validation("unknown category".into())),
        Some(0) => return Err(CoreError::Validation("category is inactive".into())),
        _ => {}
    }
    let auto = p.has(perm::EXPENSES_APPROVE);
    if auto && e.method == "cash" {
        crate::cash::require_open_if_configured(&mut tx).await?;
    }
    sequence::ensure(&mut tx, "expense", "EXP-", 6).await?;
    let number = sequence::next(&mut tx, "expense").await?;
    let id = Id::new();
    let now = time::now_utc_string();
    sqlx::query("INSERT INTO expenses (id, number, category_id, expense_date, amount, method, payee, description, status, client_key, created_by, created_at, decided_by, decided_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&number)
        .bind(&e.category_id)
        .bind(date.to_string())
        .bind(e.amount)
        .bind(&e.method)
        .bind(e.payee.trim())
        .bind(e.description.trim())
        .bind(if auto { "approved" } else { "pending" })
        .bind(key)
        .bind(p.user_id.to_string())
        .bind(&now)
        .bind(auto.then(|| p.user_id.to_string()))
        .bind(auto.then_some(now.clone()))
        .execute(&mut *tx)
        .await?;
    if auto {
        post_expense(&mut tx, &id.to_string()).await?;
    }
    audit::log(&mut tx, p, if auto { "expense.recorded" } else { "expense.submitted" }, "expense", &id.to_string(), serde_json::json!({"number": number, "amount": e.amount, "method": e.method, "category": e.category_id})).await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn approve_expense(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<()> {
    p.require(perm::EXPENSES_APPROVE)?;
    let eid = id.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT status, number, method FROM expenses WHERE id = ?")
            .bind(&eid)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, number, method) = row.ok_or_else(|| CoreError::NotFound("expense".into()))?;
    if status != "pending" {
        return Err(CoreError::Conflict(format!("expense is already {status}")));
    }
    if method == "cash" {
        crate::cash::require_open_if_configured(&mut tx).await?;
    }
    sqlx::query(
        "UPDATE expenses SET status = 'approved', decided_by = ?, decided_at = ? WHERE id = ?",
    )
    .bind(p.user_id.to_string())
    .bind(time::now_utc_string())
    .bind(&eid)
    .execute(&mut *tx)
    .await?;
    post_expense(&mut tx, &eid).await?;
    audit::log(
        &mut tx,
        p,
        "expense.approved",
        "expense",
        &eid,
        serde_json::json!({"number": number}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reject_expense(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    reason: &str,
) -> CoreResult<()> {
    p.require(perm::EXPENSES_APPROVE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation("a reason is required".into()));
    }
    let eid = id.to_string();
    let mut tx = pool.begin().await?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT status, number FROM expenses WHERE id = ?")
            .bind(&eid)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, number) = row.ok_or_else(|| CoreError::NotFound("expense".into()))?;
    if status != "pending" {
        return Err(CoreError::Conflict(format!("expense is already {status}")));
    }
    sqlx::query("UPDATE expenses SET status = 'rejected', decided_by = ?, decided_at = ?, decision_note = ? WHERE id = ?").bind(p.user_id.to_string()).bind(time::now_utc_string()).bind(reason.trim()).bind(&eid).execute(&mut *tx).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "expense.rejected",
            entity_type: "expense",
            entity_id: Some(&eid),
            detail: Some(serde_json::json!({"number": number})),
            reason: Some(reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Voiding an approved expense posts an equal and opposite journal; the record stays.
pub async fn void_expense(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    reason: &str,
) -> CoreResult<()> {
    p.require(perm::EXPENSES_APPROVE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to void an expense".into(),
        ));
    }
    let eid = id.to_string();
    let mut tx = pool.begin().await?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT status, number FROM expenses WHERE id = ?")
            .bind(&eid)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, number) = row.ok_or_else(|| CoreError::NotFound("expense".into()))?;
    if status != "approved" {
        return Err(CoreError::Conflict(format!(
            "only approved expenses can be voided (this one is {status})"
        )));
    }
    sqlx::query(
        "UPDATE expenses SET status = 'voided', voided_at = ?, void_reason = ? WHERE id = ?",
    )
    .bind(time::now_utc_string())
    .bind(reason.trim())
    .bind(&eid)
    .execute(&mut *tx)
    .await?;
    post_expense_void(&mut tx, &eid).await?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "expense.voided",
            entity_type: "expense",
            entity_id: Some(&eid),
            detail: Some(serde_json::json!({"number": number})),
            reason: Some(reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn post_expense(conn: &mut SqliteConnection, expense_id: &str) -> CoreResult<()> {
    let r: Option<(String, i64, String, String, String, String)> = sqlx::query_as("SELECT e.number, e.amount, e.method, e.expense_date, c.account_code, e.description FROM expenses e JOIN expense_categories c ON c.id = e.category_id WHERE e.id = ? AND e.status IN ('approved', 'voided')").bind(expense_id).fetch_optional(&mut *conn).await?;
    let Some((number, amount, method, date, acct, desc)) = r else {
        return Ok(());
    };
    let key = format!("expense:{expense_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &format!("Expense {number}: {desc}"),
            source_type: "expense",
            source_id: expense_id,
            lines: vec![
                dr(&acct, amount, &desc),
                cr(method_account(&method), amount, "Paid"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_expense_void(
    conn: &mut SqliteConnection,
    expense_id: &str,
) -> CoreResult<()> {
    let r: Option<(String, i64, String, String, Option<String>)> = sqlx::query_as("SELECT e.number, e.amount, e.method, c.account_code, e.voided_at FROM expenses e JOIN expense_categories c ON c.id = e.category_id WHERE e.id = ? AND e.status = 'voided'").bind(expense_id).fetch_optional(&mut *conn).await?;
    let Some((number, amount, method, acct, Some(at))) = r else {
        return Ok(());
    };
    let key = format!("expense_void:{expense_id}");
    accounting::post(
        conn,
        &PostIn {
            key: &key,
            date: &accounting::dhaka_date(&at),
            memo: &format!("Expense {number} voided"),
            source_type: "expense",
            source_id: expense_id,
            lines: vec![
                dr(method_account(&method), amount, "Money back"),
                cr(&acct, amount, "Expense reversed"),
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
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM expenses WHERE status IN ('approved', 'voided') AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'expense:' || expenses.id)").fetch_all(pool).await?;
    for id in ids {
        let mut tx = pool.begin().await?;
        post_expense(&mut tx, &id).await?;
        tx.commit().await?;
        n += 1;
    }
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM expenses WHERE status = 'voided' AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'expense_void:' || expenses.id)").fetch_all(pool).await?;
    for id in ids {
        let mut tx = pool.begin().await?;
        post_expense_void(&mut tx, &id).await?;
        tx.commit().await?;
        n += 1;
    }
    Ok(n)
}

#[derive(Debug, Serialize)]
pub struct ExpenseRow {
    pub id: String,
    pub number: String,
    pub expense_date: String,
    pub category: String,
    pub amount: i64,
    pub method: String,
    pub payee: String,
    pub description: String,
    pub status: String,
    pub created_by: Option<String>,
    pub decided_by: Option<String>,
    pub decision_note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ExpenseFilter {
    pub status: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub category_id: Option<String>,
    pub search: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExpensePage {
    pub rows: Vec<ExpenseRow>,
    /// Sum of approved expenses in the filtered set.
    pub approved_total: i64,
    pub pending_count: i64,
}

pub async fn list_expenses(
    pool: &SqlitePool,
    p: &Principal,
    f: ExpenseFilter,
) -> CoreResult<ExpensePage> {
    can_view(p)?;
    let like = f.search.as_deref().map(crate::catalogue::like_pattern);
    type Raw = (
        String,
        String,
        String,
        String,
        i64,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<Raw> = sqlx::query_as(
        "SELECT e.id, e.number, e.expense_date, c.name, e.amount, e.method, e.payee, e.description, e.status, (SELECT username FROM users WHERE id = e.created_by), (SELECT username FROM users WHERE id = e.decided_by), e.decision_note
         FROM expenses e JOIN expense_categories c ON c.id = e.category_id
         WHERE (?1 IS NULL OR e.status = ?1) AND (?2 IS NULL OR e.expense_date >= ?2) AND (?3 IS NULL OR e.expense_date <= ?3) AND (?4 IS NULL OR e.category_id = ?4) AND (?5 IS NULL OR e.description LIKE ?5 OR e.payee LIKE ?5 OR e.number LIKE ?5)
         ORDER BY e.expense_date DESC, e.number DESC LIMIT 500",
    )
    .bind(&f.status)
    .bind(&f.from)
    .bind(&f.to)
    .bind(&f.category_id)
    .bind(like)
    .fetch_all(pool)
    .await?;
    let approved_total = rows
        .iter()
        .filter(|r| r.8 == "approved")
        .try_fold(Money::ZERO, |a, r| a.checked_add(Money::from_paisa(r.4)))?
        .paisa();
    let pending_count = rows.iter().filter(|r| r.8 == "pending").count() as i64;
    let rows = rows
        .into_iter()
        .map(|r| ExpenseRow {
            id: r.0,
            number: r.1,
            expense_date: r.2,
            category: r.3,
            amount: r.4,
            method: r.5,
            payee: r.6,
            description: r.7,
            status: r.8,
            created_by: r.9,
            decided_by: r.10,
            decision_note: r.11,
        })
        .collect();
    Ok(ExpensePage {
        rows,
        approved_total,
        pending_count,
    })
}
