//! Daily cash sessions: opening cash, expected cash computed from the ledgers, counted cash, variance,
//! and a locked close with an audit trail. Variance is booked to "Cash Over/Short" so the books equal the count.

use crate::accounting::{self, code, cr, dr, PostIn};
use crate::audit;
use crate::authz::{perm, Principal};
use crate::{sequence, time, CoreError, CoreResult, Id};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

/// When the shop enables `cash.require_session`, cash-moving operations need an open session.
pub(crate) async fn require_open_if_configured(conn: &mut SqliteConnection) -> CoreResult<()> {
    let on: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'cash.require_session'")
            .fetch_optional(&mut *conn)
            .await?;
    if on.as_deref() != Some("true") {
        return Ok(());
    }
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cash_sessions WHERE status = 'open'")
        .fetch_one(&mut *conn)
        .await?;
    if open == 0 {
        return Err(CoreError::Conflict(
            "open today's cash session before taking or paying cash".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct SessionRow {
    pub id: String,
    pub number: String,
    pub status: String,
    pub business_date: String,
    pub opening_cash: i64,
    pub opening_note: String,
    pub opened_by: Option<String>,
    pub opened_at: String,
    pub closed_by: Option<String>,
    pub closed_at: Option<String>,
    pub expected_cash: Option<i64>,
    pub counted_cash: Option<i64>,
    pub variance: Option<i64>,
    pub close_note: Option<String>,
}

type Raw = (
    String,
    String,
    String,
    String,
    i64,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);
const SELECT: &str = "SELECT s.id, s.number, s.status, s.business_date, s.opening_cash, s.opening_note, (SELECT username FROM users WHERE id = s.opened_by), s.opened_at, (SELECT username FROM users WHERE id = s.closed_by), s.closed_at, s.expected_cash, s.counted_cash, s.variance, s.close_note FROM cash_sessions s";

fn row(r: Raw) -> SessionRow {
    SessionRow {
        id: r.0,
        number: r.1,
        status: r.2,
        business_date: r.3,
        opening_cash: r.4,
        opening_note: r.5,
        opened_by: r.6,
        opened_at: r.7,
        closed_by: r.8,
        closed_at: r.9,
        expected_cash: r.10,
        counted_cash: r.11,
        variance: r.12,
        close_note: r.13,
    }
}

pub async fn open_session(
    pool: &SqlitePool,
    p: &Principal,
    opening_cash: i64,
    note: &str,
) -> CoreResult<Id> {
    p.require(perm::CASH_SESSION)?;
    if opening_cash < 0 {
        return Err(CoreError::Validation(
            "opening cash cannot be negative".into(),
        ));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cash_sessions WHERE status = 'open'")
        .fetch_one(&mut *tx)
        .await?;
    if open > 0 {
        return Err(CoreError::Conflict(
            "a cash session is already open; close it first".into(),
        ));
    }
    let prev: Option<i64> = sqlx::query_scalar("SELECT counted_cash FROM cash_sessions WHERE status = 'closed' ORDER BY closed_at DESC LIMIT 1").fetch_optional(&mut *tx).await?.flatten();
    if let Some(prev) = prev {
        if prev != opening_cash && note.trim().is_empty() {
            return Err(CoreError::Validation(format!(
                "opening cash differs from last close ({}); explain the difference in the note",
                crate::Money::from_paisa(prev)
            )));
        }
    }
    sequence::ensure(&mut tx, "cash_session", "CS-", 5).await?;
    let number = sequence::next(&mut tx, "cash_session").await?;
    let id = Id::new();
    sqlx::query("INSERT INTO cash_sessions (id, number, business_date, opening_cash, opening_note, opened_by, opened_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&number)
        .bind(time::today_dhaka().to_string())
        .bind(opening_cash)
        .bind(note.trim())
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await?;
    audit::log(
        &mut tx,
        p,
        "cash.session_opened",
        "cash_session",
        &id.to_string(),
        serde_json::json!({"number": number, "opening_cash": opening_cash, "previous_close": prev}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn current_session(pool: &SqlitePool, p: &Principal) -> CoreResult<Option<SessionRow>> {
    can_view(p)?;
    Ok(
        sqlx::query_as::<_, Raw>(&format!("{SELECT} WHERE s.status = 'open'"))
            .fetch_optional(pool)
            .await?
            .map(row),
    )
}

fn can_view(p: &Principal) -> CoreResult<()> {
    if p.has(perm::CASH_SESSION) || p.has(perm::ACCOUNTS_VIEW) {
        Ok(())
    } else {
        Err(CoreError::Forbidden(perm::CASH_SESSION.into()))
    }
}

pub async fn list_sessions(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<SessionRow>> {
    can_view(p)?;
    Ok(
        sqlx::query_as::<_, Raw>(&format!("{SELECT} ORDER BY s.opened_at DESC LIMIT 100"))
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(row)
            .collect(),
    )
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub session: SessionRow,
    pub sales_by_mode: Vec<(String, i64, i64)>,
    pub cash_sale_receipts: i64,
    pub cash_installment_receipts: i64,
    pub cash_refunds: i64,
    pub cash_expenses: i64,
    pub cash_supplier_payments: i64,
    pub expected_cash: i64,
    pub receipts_by_method: Vec<(String, i64)>,
    pub refunds_total: i64,
    pub expenses_total: i64,
}

async fn compute(conn: &mut SqliteConnection, s: &SessionRow) -> CoreResult<Summary> {
    let from = s.opened_at.clone();
    let to = s
        .closed_at
        .clone()
        .unwrap_or_else(|| "9999-12-31T00:00:00.000Z".into());
    async fn one(conn: &mut SqliteConnection, sql: &str, f: &str, t: &str) -> CoreResult<i64> {
        Ok(sqlx::query_scalar(sql)
            .bind(f)
            .bind(t)
            .fetch_one(&mut *conn)
            .await?)
    }
    let cash_sale_receipts = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM sale_payments WHERE method = 'cash' AND created_at >= ?1 AND created_at <= ?2", &from, &to).await?;
    let cash_installment_receipts = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM installment_collections WHERE kind = 'payment' AND method = 'cash' AND created_at >= ?1 AND created_at <= ?2", &from, &to).await?;
    let cash_refunds = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM sale_refunds WHERE method = 'cash' AND created_at >= ?1 AND created_at <= ?2", &from, &to).await?;
    let paid_out = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM expenses WHERE method = 'cash' AND status IN ('approved', 'voided') AND decided_at >= ?1 AND decided_at <= ?2", &from, &to).await?;
    let voided_back = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM expenses WHERE method = 'cash' AND status = 'voided' AND voided_at >= ?1 AND voided_at <= ?2", &from, &to).await?;
    let cash_expenses = paid_out - voided_back;
    let cash_supplier_payments = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM supplier_payments WHERE method = 'cash' AND created_at >= ?1 AND created_at <= ?2", &from, &to).await?;
    let sales_by_mode: Vec<(String, i64, i64)> = sqlx::query_as("SELECT mode, COUNT(*), COALESCE(SUM(grand_total), 0) FROM sales WHERE created_at >= ?1 AND created_at <= ?2 GROUP BY mode ORDER BY mode").bind(&from).bind(&to).fetch_all(&mut *conn).await?;
    let receipts_by_method: Vec<(String, i64)> = sqlx::query_as("SELECT method, SUM(amount) FROM (SELECT method, amount, created_at FROM sale_payments UNION ALL SELECT method, amount, created_at FROM installment_collections WHERE kind = 'payment') WHERE created_at >= ?1 AND created_at <= ?2 GROUP BY method ORDER BY method").bind(&from).bind(&to).fetch_all(&mut *conn).await?;
    let refunds_total = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM sale_refunds WHERE created_at >= ?1 AND created_at <= ?2", &from, &to).await?;
    let expenses_total = one(conn, "SELECT COALESCE(SUM(amount), 0) FROM expenses WHERE status IN ('approved', 'voided') AND decided_at >= ?1 AND decided_at <= ?2", &from, &to).await? - one(conn, "SELECT COALESCE(SUM(amount), 0) FROM expenses WHERE status = 'voided' AND voided_at >= ?1 AND voided_at <= ?2", &from, &to).await?;
    let expected_cash = s.opening_cash + cash_sale_receipts + cash_installment_receipts
        - cash_refunds
        - cash_expenses
        - cash_supplier_payments;
    Ok(Summary {
        session: s.clone(),
        sales_by_mode,
        cash_sale_receipts,
        cash_installment_receipts,
        cash_refunds,
        cash_expenses,
        cash_supplier_payments,
        expected_cash,
        receipts_by_method,
        refunds_total,
        expenses_total,
    })
}

pub async fn session_summary(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<Summary> {
    can_view(p)?;
    let s: Raw = sqlx::query_as(&format!("{SELECT} WHERE s.id = ?"))
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("cash session".into()))?;
    let mut conn = pool.acquire().await?;
    compute(&mut conn, &row(s)).await
}

/// Counts the drawer, records the variance and locks the session.
pub async fn close_session(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    counted_cash: i64,
    note: &str,
) -> CoreResult<Summary> {
    p.require(perm::CASH_SESSION)?;
    if counted_cash < 0 {
        return Err(CoreError::Validation(
            "counted cash cannot be negative".into(),
        ));
    }
    let sid = id.to_string();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let s: Raw = sqlx::query_as(&format!("{SELECT} WHERE s.id = ?"))
        .bind(&sid)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| CoreError::NotFound("cash session".into()))?;
    let session = row(s);
    if session.status != "open" {
        return Err(CoreError::Conflict(
            "this cash session is already closed".into(),
        ));
    }
    let now = time::now_utc_string();
    let mut probe = session.clone();
    probe.closed_at = Some(now.clone());
    let sum = compute(&mut tx, &probe).await?;
    let variance = counted_cash - sum.expected_cash;
    if variance != 0 && note.trim().is_empty() {
        return Err(CoreError::Validation(format!(
            "the count differs from the expected {} by {}; explain the difference in the note",
            crate::Money::from_paisa(sum.expected_cash),
            crate::Money::from_paisa(variance)
        )));
    }
    sqlx::query("UPDATE cash_sessions SET status = 'closed', closed_by = ?, closed_at = ?, expected_cash = ?, counted_cash = ?, variance = ?, close_note = ? WHERE id = ?")
        .bind(p.user_id.to_string())
        .bind(&now)
        .bind(sum.expected_cash)
        .bind(counted_cash)
        .bind(variance)
        .bind(note.trim())
        .bind(&sid)
        .execute(&mut *tx)
        .await?;
    if variance != 0 {
        let key = format!("cash_variance:{sid}");
        let memo = format!(
            "Cash {} on {}",
            if variance < 0 { "shortage" } else { "overage" },
            session.number
        );
        let lines = if variance < 0 {
            vec![
                dr(code::CASH_OVER_SHORT, -variance, "Shortage"),
                cr(code::CASH, -variance, "Cash corrected to count"),
            ]
        } else {
            vec![
                dr(code::CASH, variance, "Cash corrected to count"),
                cr(code::CASH_OVER_SHORT, variance, "Overage"),
            ]
        };
        accounting::post(
            &mut tx,
            &PostIn {
                key: &key,
                date: &time::today_dhaka().to_string(),
                memo: &memo,
                source_type: "cash_variance",
                source_id: &sid,
                lines,
                reverses: None,
                actor: Some(p.user_id),
            },
        )
        .await?;
    }
    audit::record(&mut tx, audit::AuditEntry { actor_id: Some(p.user_id), action: "cash.session_closed", entity_type: "cash_session", entity_id: Some(&sid), detail: Some(serde_json::json!({"number": session.number, "expected": sum.expected_cash, "counted": counted_cash, "variance": variance})), reason: Some(note.trim()).filter(|s| !s.is_empty()) }).await?;
    tx.commit().await?;
    session_summary(pool, p, id).await
}

pub(crate) async fn ensure_posted(pool: &SqlitePool) -> CoreResult<u64> {
    let mut n = 0;
    let rows: Vec<(String, i64, String)> = sqlx::query_as("SELECT id, variance, number FROM cash_sessions WHERE status = 'closed' AND variance <> 0 AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'cash_variance:' || cash_sessions.id)").fetch_all(pool).await?;
    for (sid, variance, number) in rows {
        let mut tx = pool.begin().await?;
        let key = format!("cash_variance:{sid}");
        let lines = if variance < 0 {
            vec![
                dr(code::CASH_OVER_SHORT, -variance, "Shortage"),
                cr(code::CASH, -variance, "Cash corrected to count"),
            ]
        } else {
            vec![
                dr(code::CASH, variance, "Cash corrected to count"),
                cr(code::CASH_OVER_SHORT, variance, "Overage"),
            ]
        };
        accounting::post(
            &mut tx,
            &PostIn {
                key: &key,
                date: &time::today_dhaka().to_string(),
                memo: &format!("Cash variance on {number}"),
                source_type: "cash_variance",
                source_id: &sid,
                lines,
                reverses: None,
                actor: None,
            },
        )
        .await?;
        tx.commit().await?;
        n += 1;
    }
    Ok(n)
}
