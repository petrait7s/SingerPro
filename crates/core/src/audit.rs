//! Append-only audit trail. Callers pass their own transaction so the audit row commits
//! atomically with the business change it describes.

use crate::{time, CoreResult, Id};
use serde::Serialize;
use sqlx::SqliteConnection;

pub struct AuditEntry<'a> {
    pub actor_id: Option<Id>,
    pub action: &'a str,
    pub entity_type: &'a str,
    pub entity_id: Option<&'a str>,
    pub detail: Option<serde_json::Value>,
    pub reason: Option<&'a str>,
}

/// Convenience for the common case: actor + action + entity + JSON detail.
pub async fn log(
    conn: &mut SqliteConnection,
    p: &crate::authz::Principal,
    action: &str,
    entity_type: &str,
    entity_id: &str,
    detail: serde_json::Value,
) -> CoreResult<()> {
    record(
        conn,
        AuditEntry {
            actor_id: Some(p.user_id),
            action,
            entity_type,
            entity_id: Some(entity_id),
            detail: Some(detail),
            reason: None,
        },
    )
    .await?;
    Ok(())
}

pub async fn record(conn: &mut SqliteConnection, e: AuditEntry<'_>) -> CoreResult<Id> {
    let id = Id::new();
    sqlx::query(
        "INSERT INTO audit_logs (id, at, actor_id, action, entity_type, entity_id, detail, reason)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(time::now_utc_string())
    .bind(e.actor_id.map(|a| a.to_string()))
    .bind(e.action)
    .bind(e.entity_type)
    .bind(e.entity_id)
    .bind(e.detail.map(|d| d.to_string()))
    .bind(e.reason)
    .execute(conn)
    .await?;
    Ok(id)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AuditRow {
    pub id: String,
    pub at: String,
    pub actor_id: Option<String>,
    pub action: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub reason: Option<String>,
}

pub async fn recent(pool: &sqlx::SqlitePool, limit: i64) -> CoreResult<Vec<AuditRow>> {
    Ok(sqlx::query_as::<_, AuditRow>(
        "SELECT id, at, actor_id, action, entity_type, entity_id, reason
         FROM audit_logs ORDER BY at DESC, id DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 500))
    .fetch_all(pool)
    .await?)
}

#[derive(Debug, Serialize)]
pub struct AuditPage {
    pub total: i64,
    pub rows: Vec<AuditRow>,
}

/// Paged audit viewer; requires `audit.view`. Optional exact `action` prefix filter.
pub async fn list(
    pool: &sqlx::SqlitePool,
    p: &crate::authz::Principal,
    limit: i64,
    offset: i64,
    action_prefix: Option<&str>,
) -> CoreResult<AuditPage> {
    p.require(crate::authz::perm::AUDIT_VIEW)?;
    let pat = format!("{}%", action_prefix.unwrap_or("").replace(['%', '_'], ""));
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action LIKE ?")
        .bind(&pat)
        .fetch_one(pool)
        .await?;
    let rows = sqlx::query_as::<_, AuditRow>(
        "SELECT id, at, actor_id, action, entity_type, entity_id, reason FROM audit_logs
         WHERE action LIKE ? ORDER BY at DESC, id DESC LIMIT ? OFFSET ?",
    )
    .bind(&pat)
    .bind(limit.clamp(1, 200))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await?;
    Ok(AuditPage { total, rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    fn entry(action: &str) -> AuditEntry<'_> {
        AuditEntry {
            actor_id: None,
            action,
            entity_type: "test",
            entity_id: Some("x"),
            detail: Some(serde_json::json!({"a": 1})),
            reason: None,
        }
    }

    #[tokio::test]
    async fn records_and_lists() {
        let (pool, _d) = test_pool().await;
        let mut c = pool.acquire().await.unwrap();
        record(&mut c, entry("one")).await.unwrap();
        record(&mut c, entry("two")).await.unwrap();
        drop(c);
        let rows = recent(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn audit_is_append_only() {
        let (pool, _d) = test_pool().await;
        let mut c = pool.acquire().await.unwrap();
        let id = record(&mut c, entry("one")).await.unwrap();
        let upd = sqlx::query("UPDATE audit_logs SET action='x' WHERE id=?")
            .bind(id.to_string())
            .execute(&mut *c)
            .await;
        assert!(upd.is_err(), "update must be blocked");
        let del = sqlx::query("DELETE FROM audit_logs WHERE id=?")
            .bind(id.to_string())
            .execute(&mut *c)
            .await;
        assert!(del.is_err(), "delete must be blocked");
    }

    #[tokio::test]
    async fn audit_rolls_back_with_its_transaction() {
        let (pool, _d) = test_pool().await;
        let mut tx = pool.begin().await.unwrap();
        record(&mut tx, entry("one")).await.unwrap();
        tx.rollback().await.unwrap();
        assert!(recent(&pool, 10).await.unwrap().is_empty());
    }
}
