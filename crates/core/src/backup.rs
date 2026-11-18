//! Consistent online backups (SQLite `VACUUM INTO`), verified before they are published,
//! with a checksum manifest. Existing backup files are never overwritten.

use crate::audit::{self, AuditEntry};
use crate::authz::{perm, Principal};
use crate::db::MIGRATOR;
use crate::{time, CoreError, CoreResult, Id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqlitePool};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MANIFEST_FORMAT: u32 = 1;
pub const BACKUP_EXT: &str = "spbak";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub app_version: String,
    pub schema_version: i64,
    pub created_at: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub db_file: String,
    pub users: i64,
    pub audit_rows: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub path: String,
    pub manifest_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub schema_version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub manifest: Option<Manifest>,
    /// `None` when no manifest exists (checksum cannot be confirmed).
    pub checksum_ok: Option<bool>,
    pub schema_version: Option<i64>,
    pub app_schema_version: i64,
    pub users: Option<i64>,
    pub audit_rows: Option<i64>,
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    pub can_restore: bool,
}

pub fn app_schema_version() -> i64 {
    MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
}

pub fn manifest_path_for(db: &Path) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(".manifest.json");
    PathBuf::from(s)
}

pub(crate) fn sha256_file(path: &Path) -> CoreResult<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub(crate) struct Inspection {
    pub problems: Vec<String>,
    pub schema_version: Option<i64>,
    pub users: Option<i64>,
    pub audit_rows: Option<i64>,
}

/// Opens a database file read-only and immutable (no -wal/-shm side files, no writes) and checks it.
pub(crate) async fn inspect_db_file(path: &Path) -> Inspection {
    let mut out = Inspection {
        problems: vec![],
        schema_version: None,
        users: None,
        audit_rows: None,
    };
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .immutable(true);
    let mut conn = match opts.connect().await {
        Ok(c) => c,
        Err(e) => {
            out.problems
                .push(format!("not a readable SQLite database: {e}"));
            return out;
        }
    };
    match sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_all(&mut conn)
        .await
    {
        Ok(rows) => {
            for r in rows.into_iter().filter(|r| r != "ok") {
                out.problems.push(format!("integrity_check: {r}"));
            }
        }
        Err(e) => {
            out.problems.push(format!("integrity_check failed: {e}"));
            return out;
        }
    }
    match sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut conn)
        .await
    {
        Ok(rows) if !rows.is_empty() => out
            .problems
            .push(format!("foreign_key_check: {} violation(s)", rows.len())),
        Ok(_) => {}
        Err(e) => out.problems.push(format!("foreign_key_check failed: {e}")),
    }
    match sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&mut conn)
    .await
    {
        Ok(v) => out.schema_version = Some(v),
        Err(_) => out
            .problems
            .push("missing migration table: this is not a Singer Pro database".into()),
    }
    out.users = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut conn)
        .await
        .ok();
    out.audit_rows = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&mut conn)
        .await
        .ok();
    out
}

/// Writes a consistent snapshot of the live DB to `dest` (must not exist). Safe while the app is running.
pub(crate) async fn snapshot_to(pool: &SqlitePool, dest: &Path) -> CoreResult<Inspection> {
    if dest.exists() {
        return Err(CoreError::Conflict(format!(
            "refusing to overwrite existing file {}",
            dest.display()
        )));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let res = sqlx::query("VACUUM INTO ?")
        .bind(dest.to_string_lossy().to_string())
        .execute(pool)
        .await;
    if let Err(e) = res {
        let _ = std::fs::remove_file(dest);
        return Err(CoreError::Io(std::io::Error::other(format!(
            "could not write backup (disk full or folder not writable?): {e}"
        ))));
    }
    let insp = inspect_db_file(dest).await;
    if !insp.problems.is_empty() {
        let _ = std::fs::remove_file(dest);
        return Err(CoreError::Integrity(format!(
            "fresh backup failed verification: {}",
            insp.problems.join("; ")
        )));
    }
    Ok(insp)
}

fn stamp() -> String {
    let t = time::to_dhaka(chrono::Utc::now());
    let short = &Id::new().to_string()[24..30];
    format!("{}-{}", t.format("%Y%m%d-%H%M%S"), short)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_history(
    pool: &SqlitePool,
    kind: &str,
    path: &Path,
    size: Option<u64>,
    sha: Option<&str>,
    schema: Option<i64>,
    status: &str,
    actor: Option<Id>,
    note: Option<&str>,
) -> CoreResult<()> {
    sqlx::query(
        "INSERT INTO backup_history (id, at, kind, path, size_bytes, sha256, schema_version, status, actor_id, note)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Id::new().to_string())
    .bind(time::now_utc_string())
    .bind(kind)
    .bind(path.to_string_lossy().to_string())
    .bind(size.map(|s| s as i64))
    .bind(sha)
    .bind(schema)
    .bind(status)
    .bind(actor.map(|a| a.to_string()))
    .bind(note)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_backup(
    pool: &SqlitePool,
    p: &Principal,
    dest_dir: &Path,
) -> CoreResult<BackupInfo> {
    p.require(perm::BACKUP_CREATE)?;
    std::fs::create_dir_all(dest_dir)?;
    let name = format!("singerpro-{}.{}", stamp(), BACKUP_EXT);
    let final_path = dest_dir.join(&name);
    let partial = dest_dir.join(format!(".{name}.partial"));
    let _ = std::fs::remove_file(&partial);
    let insp = match snapshot_to(pool, &partial).await {
        Ok(i) => i,
        Err(e) => {
            let _ = record_history(
                pool,
                "backup",
                &final_path,
                None,
                None,
                None,
                "failed",
                Some(p.user_id),
                Some(&e.to_string()),
            )
            .await;
            return Err(e);
        }
    };
    let partial_c = partial.clone();
    let sha = tokio::task::spawn_blocking(move || sha256_file(&partial_c))
        .await
        .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))??;
    let size = std::fs::metadata(&partial)?.len();
    if final_path.exists() {
        let _ = std::fs::remove_file(&partial);
        return Err(CoreError::Conflict("backup file already exists".into()));
    }
    std::fs::rename(&partial, &final_path)?;
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: insp.schema_version.unwrap_or(0),
        created_at: time::now_utc_string(),
        sha256: sha.clone(),
        size_bytes: size,
        db_file: name.clone(),
        users: insp.users.unwrap_or(0),
        audit_rows: insp.audit_rows.unwrap_or(0),
    };
    let mpath = manifest_path_for(&final_path);
    let mtmp = dest_dir.join(format!(".{name}.manifest.partial"));
    std::fs::write(
        &mtmp,
        serde_json::to_vec_pretty(&manifest).map_err(|e| CoreError::Validation(e.to_string()))?,
    )?;
    std::fs::rename(&mtmp, &mpath)?;
    record_history(
        pool,
        "backup",
        &final_path,
        Some(size),
        Some(&sha),
        insp.schema_version,
        "ok",
        Some(p.user_id),
        None,
    )
    .await?;
    let mut c = pool.acquire().await?;
    audit::record(
        &mut c,
        AuditEntry {
            actor_id: Some(p.user_id),
            action: "backup.created",
            entity_type: "backup",
            entity_id: Some(&name),
            detail: Some(serde_json::json!({"path": final_path, "sha256": sha, "bytes": size})),
            reason: None,
        },
    )
    .await?;
    Ok(BackupInfo {
        path: final_path.to_string_lossy().to_string(),
        manifest_path: mpath.to_string_lossy().to_string(),
        size_bytes: size,
        sha256: sha,
        schema_version: manifest.schema_version,
    })
}

/// Read-only verification of a backup file: checksum (if manifest), integrity, schema compatibility.
pub async fn verify_backup(p: &Principal, path: &Path) -> CoreResult<Preview> {
    if !(p.has(perm::BACKUP_CREATE) || p.has(perm::BACKUP_RESTORE)) {
        return Err(CoreError::Forbidden(perm::BACKUP_RESTORE.into()));
    }
    verify_path(path).await
}

pub(crate) async fn verify_path(path: &Path) -> CoreResult<Preview> {
    let meta = std::fs::metadata(path)
        .map_err(|e| CoreError::NotFound(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(CoreError::Validation("not a file".into()));
    }
    let pc = path.to_path_buf();
    let sha = tokio::task::spawn_blocking(move || sha256_file(&pc))
        .await
        .map_err(|e| CoreError::Io(std::io::Error::other(e.to_string())))??;
    let mut problems = vec![];
    let mut warnings = vec![];
    let mpath = manifest_path_for(path);
    let manifest: Option<Manifest> = match std::fs::read(&mpath) {
        Ok(b) => match serde_json::from_slice::<Manifest>(&b) {
            Ok(m) => Some(m),
            Err(e) => {
                warnings.push(format!("manifest unreadable: {e}"));
                None
            }
        },
        Err(_) => {
            warnings.push("no manifest found: checksum cannot be confirmed".into());
            None
        }
    };
    let checksum_ok = manifest.as_ref().map(|m| m.sha256 == sha);
    if checksum_ok == Some(false) {
        problems.push("checksum mismatch: the backup file was modified or corrupted".into());
    }
    let insp = inspect_db_file(path).await;
    problems.extend(insp.problems);
    let app_v = app_schema_version();
    if let Some(v) = insp.schema_version {
        if v > app_v {
            problems.push(format!(
                "backup schema v{v} is newer than this app (v{app_v}); update Singer Pro first"
            ));
        } else if v < app_v {
            warnings.push(format!(
                "backup schema v{v} is older; it will be upgraded to v{app_v} after restore"
            ));
        }
    }
    let can_restore = problems.is_empty();
    Ok(Preview {
        path: path.to_string_lossy().to_string(),
        size_bytes: meta.len(),
        sha256: sha,
        manifest,
        checksum_ok,
        schema_version: insp.schema_version,
        app_schema_version: app_v,
        users: insp.users,
        audit_rows: insp.audit_rows,
        problems,
        warnings,
        can_restore,
    })
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct HistoryRow {
    pub id: String,
    pub at: String,
    pub kind: String,
    pub path: String,
    pub size_bytes: Option<i64>,
    pub status: String,
    pub note: Option<String>,
}

pub async fn history(pool: &SqlitePool, p: &Principal, limit: i64) -> CoreResult<Vec<HistoryRow>> {
    if !(p.has(perm::BACKUP_CREATE) || p.has(perm::BACKUP_RESTORE)) {
        return Err(CoreError::Forbidden(perm::BACKUP_CREATE.into()));
    }
    Ok(sqlx::query_as(
        "SELECT id, at, kind, path, size_bytes, status, note FROM backup_history ORDER BY at DESC, id DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?)
}

#[derive(Debug, Serialize)]
pub struct BackupStatus {
    pub last_backup_at: Option<String>,
    pub reminder_days: i64,
    pub overdue: bool,
}

/// Visible to any signed-in user so the dashboard can show the last-backup state.
pub async fn status(pool: &SqlitePool, _p: &Principal) -> CoreResult<BackupStatus> {
    let last: Option<String> = sqlx::query_scalar(
        "SELECT MAX(at) FROM backup_history WHERE kind = 'backup' AND status = 'ok'",
    )
    .fetch_one(pool)
    .await?;
    let days: i64 = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = 'backup.reminder_days'",
    )
    .fetch_optional(pool)
    .await?
    .and_then(|v| v.parse().ok())
    .unwrap_or(7);
    let overdue = days > 0
        && match &last {
            None => true,
            Some(t) => chrono::DateTime::parse_from_rfc3339(t)
                .map(|t| {
                    chrono::Utc::now() - t.with_timezone(&chrono::Utc)
                        > chrono::Duration::days(days)
                })
                .unwrap_or(true),
        };
    Ok(BackupStatus {
        last_backup_at: last,
        reminder_days: days,
        overdue,
    })
}
