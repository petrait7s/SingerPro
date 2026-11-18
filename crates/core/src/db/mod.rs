//! SQLite connection, migrations and health checks.

use crate::{CoreError, CoreResult};
use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

pub use sqlx::SqlitePool as Pool;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Open (creating if needed) and migrate the database at `path`.
/// Foreign keys are enabled on EVERY pooled connection via connect options.
pub async fn open(path: &Path) -> CoreResult<SqlitePool> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(CoreError::Database)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full) // financial data: prefer durability over speed
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;
    MIGRATOR.run(&pool).await?;
    bootstrap(&pool).await?;
    Ok(pool)
}

/// Idempotent first-run data: default branch and default (configurable) settings.
/// Never overwrites values that already exist.
pub async fn bootstrap(pool: &SqlitePool) -> CoreResult<()> {
    let now = crate::time::now_utc_string();
    sqlx::query("INSERT OR IGNORE INTO branches (id, code, name, is_default, created_at) VALUES (?, 'MAIN', 'Main Branch', 1, ?)")
        .bind(crate::Id::new().to_string())
        .bind(&now)
        .execute(pool)
        .await?;
    let defaults = [
        ("shop.name", "Singer Pro"),
        ("currency.code", "BDT"),
        ("timezone", "Asia/Dhaka"),
        ("tax.enabled", "false"),
        ("tax.default_rate_bp", "0"),
        ("installment.default_markup_bp", "0"),
        ("installment.rounding_remainder", "last_installment"),
        (
            "installment.markup_recognition",
            "proportional_to_collections",
        ),
        ("stock.allow_negative", "false"),
        ("returns.require_inspection", "true"),
    ];
    for (k, v) in defaults {
        sqlx::query("INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)")
            .bind(k)
            .bind(v)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Clone)]
pub struct DbStatus {
    pub schema_version: i64,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub integrity_ok: bool,
    pub integrity_detail: Vec<String>,
}

pub async fn status(pool: &SqlitePool) -> CoreResult<DbStatus> {
    let schema_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(pool)
    .await?;
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await?;
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    let detail = integrity_problems(pool).await?;
    Ok(DbStatus {
        schema_version,
        journal_mode,
        foreign_keys: fk == 1,
        integrity_ok: detail.is_empty(),
        integrity_detail: detail,
    })
}

/// Empty vec = healthy. Runs `integrity_check` and `foreign_key_check`.
pub async fn integrity_problems(pool: &SqlitePool) -> CoreResult<Vec<String>> {
    let mut problems = Vec::new();
    let rows: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(pool)
        .await?;
    for r in rows {
        if r != "ok" {
            problems.push(format!("integrity: {r}"));
        }
    }
    let fk = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await?;
    if !fk.is_empty() {
        problems.push(format!("foreign_key_check: {} violation(s)", fk.len()));
    }
    Ok(problems)
}

#[cfg(test)]
pub(crate) async fn test_pool() -> (SqlitePool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let pool = open(&dir.path().join("t.db")).await.unwrap();
    (pool, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_migrates_bootstraps_and_is_healthy() {
        let (pool, _d) = test_pool().await;
        let st = status(&pool).await.unwrap();
        assert!(st.schema_version >= 1);
        assert!(st.foreign_keys, "FK must be on");
        assert_eq!(st.journal_mode, "wal");
        assert!(st.integrity_ok, "{:?}", st.integrity_detail);
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM branches WHERE is_default = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
        let rate: String = sqlx::query_scalar(
            "SELECT value FROM app_settings WHERE key='installment.default_markup_bp'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rate, "0");
    }

    #[tokio::test]
    async fn reopen_is_idempotent_and_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let p1 = open(&path).await.unwrap();
        sqlx::query("UPDATE app_settings SET value='Custom' WHERE key='shop.name'")
            .execute(&p1)
            .await
            .unwrap();
        p1.close().await;
        let p2 = open(&path).await.unwrap();
        let v: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key='shop.name'")
            .fetch_one(&p2)
            .await
            .unwrap();
        assert_eq!(
            v, "Custom",
            "bootstrap must not overwrite existing settings"
        );
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM branches")
            .fetch_one(&p2)
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn foreign_keys_enforced_on_every_connection() {
        let (pool, _d) = test_pool().await;
        sqlx::query("CREATE TABLE p (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE c (pid TEXT REFERENCES p(id))")
            .execute(&pool)
            .await
            .unwrap();
        // hold several connections at once so the pool hands out distinct ones
        let mut conns = vec![];
        for _ in 0..4 {
            conns.push(pool.acquire().await.unwrap());
        }
        for c in conns.iter_mut() {
            let r = sqlx::query("INSERT INTO c (pid) VALUES ('nope')")
                .execute(&mut **c)
                .await;
            assert!(r.is_err());
        }
    }
}
