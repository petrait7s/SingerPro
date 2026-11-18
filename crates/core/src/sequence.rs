//! Transactional document numbering. Unique and monotonic; a rolled-back transaction
//! releases its number (no gaps are ever *committed*).

use crate::{CoreError, CoreResult};
use sqlx::SqliteConnection;

pub async fn ensure(
    conn: &mut SqliteConnection,
    name: &str,
    prefix: &str,
    padding: u8,
) -> CoreResult<()> {
    sqlx::query("INSERT OR IGNORE INTO document_sequences (name, prefix, padding, next_value) VALUES (?, ?, ?, 1)")
        .bind(name)
        .bind(prefix)
        .bind(padding as i64)
        .execute(conn)
        .await?;
    Ok(())
}

/// Reserve the next number inside the caller's transaction, e.g. `INV-000042`.
pub async fn next(conn: &mut SqliteConnection, name: &str) -> CoreResult<String> {
    let row: Option<(i64, String, i64)> = sqlx::query_as(
        "UPDATE document_sequences SET next_value = next_value + 1 WHERE name = ?
         RETURNING next_value - 1, prefix, padding",
    )
    .bind(name)
    .fetch_optional(conn)
    .await?;
    let (n, prefix, pad) = row.ok_or_else(|| CoreError::NotFound(format!("sequence {name}")))?;
    Ok(format!("{prefix}{n:0width$}", width = pad as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_pool;

    #[tokio::test]
    async fn sequential_and_padded() {
        let (pool, _d) = test_pool().await;
        let mut c = pool.acquire().await.unwrap();
        ensure(&mut c, "invoice", "INV-", 6).await.unwrap();
        ensure(&mut c, "invoice", "XXX-", 3).await.unwrap(); // idempotent, must not reset
        assert_eq!(next(&mut c, "invoice").await.unwrap(), "INV-000001");
        assert_eq!(next(&mut c, "invoice").await.unwrap(), "INV-000002");
        assert!(next(&mut c, "missing").await.is_err());
    }

    #[tokio::test]
    async fn rollback_releases_number() {
        let (pool, _d) = test_pool().await;
        let mut c = pool.acquire().await.unwrap();
        ensure(&mut c, "inv", "I", 3).await.unwrap();
        drop(c);
        let mut tx = pool.begin().await.unwrap();
        assert_eq!(next(&mut tx, "inv").await.unwrap(), "I001");
        tx.rollback().await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        assert_eq!(next(&mut tx, "inv").await.unwrap(), "I001");
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_callers_get_unique_numbers() {
        let (pool, _d) = test_pool().await;
        {
            let mut c = pool.acquire().await.unwrap();
            ensure(&mut c, "inv", "I", 3).await.unwrap();
        }
        let mut hs = vec![];
        for _ in 0..20 {
            let p = pool.clone();
            hs.push(tokio::spawn(async move {
                let mut tx = p.begin().await.unwrap();
                let n = next(&mut tx, "inv").await.unwrap();
                tx.commit().await.unwrap();
                n
            }));
        }
        let mut got: Vec<String> = vec![];
        for h in hs {
            got.push(h.await.unwrap());
        }
        got.sort();
        got.dedup();
        assert_eq!(got.len(), 20);
    }
}
