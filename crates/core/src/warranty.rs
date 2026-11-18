//! Warranty cases: look up a serial number, check coverage from the sale date, track service history.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::{sequence, time, CoreError, CoreResult, Id};
use chrono::{Months, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

fn can_see(p: &Principal) -> CoreResult<()> {
    if p.has(perm::WARRANTY_MANAGE) || p.has(perm::RETURNS_PROCESS) {
        Ok(())
    } else {
        Err(CoreError::Forbidden(perm::WARRANTY_MANAGE.into()))
    }
}

#[derive(Debug, Serialize)]
pub struct Coverage {
    pub serial_id: String,
    pub serial_no: String,
    pub variant_id: String,
    pub label: String,
    pub sale_id: Option<String>,
    pub sale_number: Option<String>,
    pub customer_id: Option<String>,
    pub customer: Option<String>,
    pub purchased_on: Option<String>,
    pub warranty_months: i64,
    pub warranty_terms: String,
    pub expires_on: Option<String>,
    pub in_warranty: bool,
    pub status: String,
}

/// Warranty status of a physical unit. Months run from the sale date (Asia/Dhaka calendar days).
pub async fn lookup_serial(
    pool: &SqlitePool,
    p: &Principal,
    serial_no: &str,
) -> CoreResult<Option<Coverage>> {
    can_see(p)?;
    type Raw = (
        String,
        String,
        String,
        String,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let r: Option<Raw> = sqlx::query_as(
        "SELECT ps.id, ps.serial_no, ps.variant_id, TRIM(pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END), pr.warranty_months, pr.warranty_terms, ps.status,
            (SELECT s.id FROM sale_items i JOIN sales s ON s.id = i.sale_id WHERE i.serial_id = ps.id ORDER BY s.created_at DESC LIMIT 1),
            (SELECT s.number FROM sale_items i JOIN sales s ON s.id = i.sale_id WHERE i.serial_id = ps.id ORDER BY s.created_at DESC LIMIT 1),
            (SELECT s.customer_id FROM sale_items i JOIN sales s ON s.id = i.sale_id WHERE i.serial_id = ps.id ORDER BY s.created_at DESC LIMIT 1),
            (SELECT c.name FROM sale_items i JOIN sales s ON s.id = i.sale_id JOIN customers c ON c.id = s.customer_id WHERE i.serial_id = ps.id ORDER BY s.created_at DESC LIMIT 1),
            (SELECT s.sale_date FROM sale_items i JOIN sales s ON s.id = i.sale_id WHERE i.serial_id = ps.id ORDER BY s.created_at DESC LIMIT 1)
         FROM product_serials ps JOIN product_variants v ON v.id = ps.variant_id JOIN products pr ON pr.id = v.product_id WHERE ps.serial_no = ? ORDER BY ps.received_at DESC LIMIT 1",
    )
    .bind(serial_no.trim())
    .fetch_optional(pool)
    .await?;
    let Some((
        serial_id,
        sn,
        variant_id,
        label,
        months,
        terms,
        status,
        sale_id,
        sale_number,
        customer_id,
        customer,
        purchased_on,
    )) = r
    else {
        return Ok(None);
    };
    let (expires_on, in_warranty) = match &purchased_on {
        Some(d) => {
            let start = NaiveDate::parse_from_str(d, "%Y-%m-%d")
                .map_err(|_| CoreError::Integrity("bad sale date".into()))?;
            let exp = start
                .checked_add_months(Months::new(months as u32))
                .ok_or_else(|| CoreError::Validation("date out of range".into()))?;
            (
                Some(exp.to_string()),
                months > 0 && time::today_dhaka() <= exp,
            )
        }
        None => (None, false),
    };
    Ok(Some(Coverage {
        serial_id,
        serial_no: sn,
        variant_id,
        label,
        sale_id,
        sale_number,
        customer_id,
        customer,
        purchased_on,
        warranty_months: months,
        warranty_terms: terms,
        expires_on,
        in_warranty,
        status,
    }))
}

#[derive(Debug, Deserialize)]
pub struct OpenCaseIn {
    pub serial_no: String,
    pub issue: String,
}

pub async fn open_case(pool: &SqlitePool, p: &Principal, o: OpenCaseIn) -> CoreResult<Id> {
    p.require(perm::WARRANTY_MANAGE)?;
    if o.issue.trim().is_empty() || o.issue.chars().count() > 500 {
        return Err(CoreError::Validation(
            "describe the problem (max 500 characters)".into(),
        ));
    }
    let cov = lookup_serial(pool, p, &o.serial_no)
        .await?
        .ok_or_else(|| CoreError::NotFound(format!("serial number '{}'", o.serial_no.trim())))?;
    if cov.sale_id.is_none() {
        return Err(CoreError::Validation(
            "this unit has not been sold, so it has no warranty claim".into(),
        ));
    }
    let open: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM warranty_cases WHERE serial_id = ? AND status IN ('open', 'in_service')").bind(&cov.serial_id).fetch_one(pool).await?;
    if open > 0 {
        return Err(CoreError::Conflict(
            "this unit already has an open warranty case".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    sequence::ensure(&mut tx, "warranty", "WAR-", 6).await?;
    let number = sequence::next(&mut tx, "warranty").await?;
    let id = Id::new();
    sqlx::query("INSERT INTO warranty_cases (id, number, customer_id, sale_id, variant_id, serial_id, serial_no, purchased_on, warranty_expires_on, in_warranty, issue, opened_on, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&number)
        .bind(&cov.customer_id)
        .bind(&cov.sale_id)
        .bind(&cov.variant_id)
        .bind(&cov.serial_id)
        .bind(&cov.serial_no)
        .bind(&cov.purchased_on)
        .bind(&cov.expires_on)
        .bind(cov.in_warranty as i64)
        .bind(o.issue.trim())
        .bind(time::today_dhaka().to_string())
        .bind(p.user_id.to_string())
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO warranty_events (id, case_id, at, actor_id, status, note) VALUES (?, ?, ?, ?, 'open', ?)").bind(Id::new().to_string()).bind(id.to_string()).bind(time::now_utc_string()).bind(p.user_id.to_string()).bind(if cov.in_warranty { format!("Opened: {}", o.issue.trim()) } else { format!("Opened OUT OF WARRANTY: {}", o.issue.trim()) }).execute(&mut *tx).await?;
    audit::log(&mut tx, p, "warranty.opened", "warranty_case", &id.to_string(), serde_json::json!({"number": number, "serial": cov.serial_no, "in_warranty": cov.in_warranty})).await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update_status(
    pool: &SqlitePool,
    p: &Principal,
    case_id: Id,
    status: &str,
    note: &str,
) -> CoreResult<()> {
    p.require(perm::WARRANTY_MANAGE)?;
    if note.trim().is_empty() {
        return Err(CoreError::Validation("a note is required".into()));
    }
    let cid = case_id.to_string();
    let mut tx = pool.begin().await?;
    let cur: Option<String> = sqlx::query_scalar("SELECT status FROM warranty_cases WHERE id = ?")
        .bind(&cid)
        .fetch_optional(&mut *tx)
        .await?;
    let cur = cur.ok_or_else(|| CoreError::NotFound("warranty case".into()))?;
    let ok = matches!(
        (cur.as_str(), status),
        ("open", "in_service" | "resolved" | "rejected") | ("in_service", "resolved" | "rejected")
    );
    if !ok {
        return Err(CoreError::Conflict(format!(
            "a {cur} case cannot become {status}"
        )));
    }
    let closing = matches!(status, "resolved" | "rejected");
    sqlx::query("UPDATE warranty_cases SET status = ?, resolution = CASE WHEN ? THEN ? ELSE resolution END, closed_on = CASE WHEN ? THEN ? ELSE closed_on END WHERE id = ?")
        .bind(status)
        .bind(closing)
        .bind(note.trim())
        .bind(closing)
        .bind(time::today_dhaka().to_string())
        .bind(&cid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO warranty_events (id, case_id, at, actor_id, status, note) VALUES (?, ?, ?, ?, ?, ?)").bind(Id::new().to_string()).bind(&cid).bind(time::now_utc_string()).bind(p.user_id.to_string()).bind(status).bind(note.trim()).execute(&mut *tx).await?;
    audit::log(
        &mut tx,
        p,
        "warranty.status",
        "warranty_case",
        &cid,
        serde_json::json!({"from": cur, "to": status}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CaseRow {
    pub id: String,
    pub number: String,
    pub serial_no: String,
    pub label: String,
    pub customer: Option<String>,
    pub status: String,
    pub in_warranty: bool,
    pub issue: String,
    pub opened_on: String,
    pub closed_on: Option<String>,
    pub resolution: Option<String>,
    pub warranty_expires_on: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EventRow {
    pub at: String,
    pub status: String,
    pub note: String,
    pub actor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CaseDetail {
    pub case: CaseRow,
    pub events: Vec<EventRow>,
}

const CASE_SELECT: &str = "SELECT w.id, w.number, w.serial_no, TRIM(pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END) AS label, c.name AS customer, w.status, w.in_warranty, w.issue, w.opened_on, w.closed_on, w.resolution, w.warranty_expires_on
    FROM warranty_cases w JOIN product_variants v ON v.id = w.variant_id JOIN products pr ON pr.id = v.product_id LEFT JOIN customers c ON c.id = w.customer_id";

pub async fn list_cases(
    pool: &SqlitePool,
    p: &Principal,
    status: Option<&str>,
    search: Option<&str>,
) -> CoreResult<Vec<CaseRow>> {
    can_see(p)?;
    let like = search.map(crate::catalogue::like_pattern);
    Ok(sqlx::query_as(&format!("{CASE_SELECT} WHERE (?1 IS NULL OR w.status = ?1) AND (?2 IS NULL OR w.number LIKE ?2 OR w.serial_no LIKE ?2 OR c.name LIKE ?2) ORDER BY w.created_at DESC LIMIT 300")).bind(status).bind(like).fetch_all(pool).await?)
}

pub async fn get_case(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<CaseDetail> {
    can_see(p)?;
    let case: CaseRow = sqlx::query_as(&format!("{CASE_SELECT} WHERE w.id = ?"))
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("warranty case".into()))?;
    let events = sqlx::query_as("SELECT e.at, e.status, e.note, u.username AS actor FROM warranty_events e LEFT JOIN users u ON u.id = e.actor_id WHERE e.case_id = ? ORDER BY e.at, e.id").bind(id.to_string()).fetch_all(pool).await?;
    Ok(CaseDetail { case, events })
}
