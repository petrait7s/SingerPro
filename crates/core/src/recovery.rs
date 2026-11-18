//! Safe restore. A restore is STAGED while the app runs (verified, with a safety copy of the live
//! database taken first) and APPLIED at the next startup before any connection is opened — this
//! avoids Windows file locks and means a restore can never half-overwrite a live database.
//! The replaced database is always kept under `replaced/`; nothing is deleted silently.

use crate::audit::{self, AuditEntry};
use crate::authz::{perm, Principal};
use crate::backup::{self, inspect_db_file, record_history, sha256_file, snapshot_to, Preview};
use crate::{db, time, CoreError, CoreResult, Id};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DataPaths {
    pub root: PathBuf,
    pub db: PathBuf,
    pub pending: PathBuf,
    pub safety: PathBuf,
    pub replaced: PathBuf,
    pub default_backups: PathBuf,
}

impl DataPaths {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            db: root.join("data").join("singerpro.db"),
            pending: root.join("pending-restore"),
            safety: root.join("pre-restore"),
            replaced: root.join("replaced"),
            default_backups: root.join("backups"),
        }
    }
    fn marker(&self) -> PathBuf {
        self.pending.join("marker.json")
    }
    fn staged(&self) -> PathBuf {
        self.pending.join("restore.db")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub staged_at: String,
    pub source: String,
    pub sha256: String,
    pub safety_copy: String,
    pub actor_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StagedRestore {
    pub marker: Marker,
    pub preview: Preview,
}

pub fn pending(paths: &DataPaths) -> Option<Marker> {
    std::fs::read(paths.marker())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn stamp() -> String {
    time::to_dhaka(chrono::Utc::now())
        .format("%Y%m%d-%H%M%S")
        .to_string()
}

pub async fn stage_restore(
    pool: &SqlitePool,
    p: &Principal,
    paths: &DataPaths,
    backup_file: &Path,
) -> CoreResult<StagedRestore> {
    p.require(perm::BACKUP_RESTORE)?;
    if pending(paths).is_some() {
        return Err(CoreError::Conflict(
            "a restore is already pending; restart the app or cancel it first".into(),
        ));
    }
    let preview = backup::verify_path(backup_file).await?;
    if !preview.can_restore {
        return Err(CoreError::Integrity(format!(
            "backup rejected: {}",
            preview.problems.join("; ")
        )));
    }
    // 1. Safety copy of the CURRENT database first. If this fails, nothing else happens.
    let safety = paths.safety.join(format!(
        "pre-restore-{}-{}.{}",
        stamp(),
        &Id::new().to_string()[24..30],
        backup::BACKUP_EXT
    ));
    let insp = snapshot_to(pool, &safety).await.map_err(|e| {
        CoreError::Integrity(format!("could not take safety copy, restore aborted: {e}"))
    })?;
    let safety_sha = sha256_file(&safety)?;
    record_history(
        pool,
        "pre_restore",
        &safety,
        std::fs::metadata(&safety).ok().map(|m| m.len()),
        Some(&safety_sha),
        insp.schema_version,
        "ok",
        Some(p.user_id),
        None,
    )
    .await?;
    // 2. Stage a private copy and re-verify it byte for byte.
    std::fs::create_dir_all(&paths.pending)?;
    let part = paths.pending.join("restore.db.partial");
    let _ = std::fs::remove_file(&part);
    std::fs::copy(backup_file, &part)?;
    if sha256_file(&part)? != preview.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(CoreError::Integrity(
            "staged copy differs from source; restore aborted".into(),
        ));
    }
    std::fs::rename(&part, paths.staged())?;
    let marker = Marker {
        staged_at: time::now_utc_string(),
        source: backup_file.to_string_lossy().to_string(),
        sha256: preview.sha256.clone(),
        safety_copy: safety.to_string_lossy().to_string(),
        actor_id: Some(p.user_id.to_string()),
    };
    std::fs::write(
        paths.marker(),
        serde_json::to_vec_pretty(&marker).map_err(|e| CoreError::Validation(e.to_string()))?,
    )?;
    record_history(
        pool,
        "restore_staged",
        backup_file,
        Some(preview.size_bytes),
        Some(&preview.sha256),
        preview.schema_version,
        "ok",
        Some(p.user_id),
        Some("applies at next start"),
    )
    .await?;
    let mut c = pool.acquire().await?;
    audit::record(
        &mut c,
        AuditEntry {
            actor_id: Some(p.user_id),
            action: "restore.staged",
            entity_type: "backup",
            entity_id: Some(&preview.sha256),
            detail: Some(
                serde_json::json!({"source": marker.source, "safety_copy": marker.safety_copy}),
            ),
            reason: None,
        },
    )
    .await?;
    Ok(StagedRestore { marker, preview })
}

pub async fn cancel_pending(pool: &SqlitePool, p: &Principal, paths: &DataPaths) -> CoreResult<()> {
    p.require(perm::BACKUP_RESTORE)?;
    if pending(paths).is_none() {
        return Err(CoreError::NotFound("no pending restore".into()));
    }
    std::fs::remove_dir_all(&paths.pending)?;
    let mut c = pool.acquire().await?;
    audit::record(
        &mut c,
        AuditEntry {
            actor_id: Some(p.user_id),
            action: "restore.cancelled",
            entity_type: "backup",
            entity_id: None,
            detail: None,
            reason: None,
        },
    )
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreOutcome {
    pub applied: bool,
    pub message: String,
    pub replaced_dir: Option<String>,
}

/// Moves a file, retrying for a few seconds: on Windows an antivirus scanner, indexer or a database
/// handle that is still closing can briefly hold the file (os error 32).
async fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for attempt in 0..20 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                if attempt < 19 {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        }
    }
    Err(last.expect("at least one attempt"))
}

fn side_files(db: &Path) -> [PathBuf; 3] {
    let mk = |suf: &str| {
        let mut s = db.as_os_str().to_owned();
        s.push(suf);
        PathBuf::from(s)
    };
    [db.to_path_buf(), mk("-wal"), mk("-shm")]
}

fn fail_marker(paths: &DataPaths, reason: &str) {
    let dest = paths.root.join(format!("restore-failed-{}.txt", stamp()));
    let _ = std::fs::write(&dest, reason);
    let _ = std::fs::remove_dir_all(&paths.pending);
}

/// Call at startup BEFORE `db::open`. Returns None when nothing was pending.
/// On any failure the original database is left (or put back) exactly as it was.
pub async fn apply_pending(paths: &DataPaths) -> Option<RestoreOutcome> {
    let marker = pending(paths)?;
    let staged = paths.staged();
    let failed = |paths: &DataPaths, reason: String| {
        fail_marker(paths, &reason);
        Some(RestoreOutcome {
            applied: false,
            message: format!("Restore was NOT applied; your data is unchanged. {reason}"),
            replaced_dir: None,
        })
    };
    match sha256_file(&staged) {
        Ok(s) if s == marker.sha256 => {}
        _ => return failed(paths, "Staged backup failed its checksum.".into()),
    }
    let insp = inspect_db_file(&staged).await;
    if !insp.problems.is_empty()
        || insp.schema_version.unwrap_or(i64::MAX) > backup::app_schema_version()
    {
        return failed(
            paths,
            format!(
                "Staged backup failed verification: {}",
                insp.problems.join("; ")
            ),
        );
    }
    let replaced_dir = paths.replaced.join(stamp());
    if let Err(e) = std::fs::create_dir_all(&replaced_dir) {
        return failed(
            paths,
            format!("Cannot create {}: {e}", replaced_dir.display()),
        );
    }
    // Move current DB aside (kept, never deleted).
    let mut moved: Vec<(PathBuf, PathBuf)> = vec![];
    for f in side_files(&paths.db) {
        if f.exists() {
            let to = replaced_dir.join(f.file_name().expect("name"));
            if let Err(e) = move_file(&f, &to).await {
                rollback(&moved).await;
                return failed(paths, format!("Cannot move current database aside: {e}"));
            }
            moved.push((f, to));
        }
    }
    let install = async {
        if let Some(d) = paths.db.parent() {
            std::fs::create_dir_all(d)?;
        }
        move_file(&staged, &paths.db).await
    }
    .await;
    if let Err(e) = install {
        rollback(&moved).await;
        return failed(paths, format!("Cannot install restored database: {e}"));
    }
    // Open (migrates older schemas) and verify; on failure put the original back.
    let verified = match db::open(&paths.db).await {
        Ok(pool) => {
            let st = db::status(&pool).await;
            let ok = matches!(&st, Ok(s) if s.integrity_ok);
            if ok {
                let _ = backup::record_history(
                    &pool,
                    "restore_applied",
                    Path::new(&marker.source),
                    None,
                    Some(&marker.sha256),
                    None,
                    "ok",
                    marker.actor_id.as_deref().and_then(|a| Id::parse(a).ok()),
                    Some(&format!(
                        "previous database kept in {}",
                        replaced_dir.display()
                    )),
                )
                .await;
                if let Ok(mut c) = pool.acquire().await {
                    let _ = audit::record(
                        &mut c,
                        AuditEntry {
                            actor_id: marker.actor_id.as_deref().and_then(|a| Id::parse(a).ok()),
                            action: "restore.applied",
                            entity_type: "backup",
                            entity_id: Some(&marker.sha256),
                            detail: Some(serde_json::json!({"source": marker.source, "replaced_dir": replaced_dir, "safety_copy": marker.safety_copy})),
                            reason: None,
                        },
                    )
                    .await;
                }
            }
            pool.close().await;
            ok
        }
        Err(_) => false,
    };
    if !verified {
        for f in side_files(&paths.db) {
            let _ = std::fs::remove_file(f);
        }
        rollback(&moved).await;
        return failed(
            paths,
            "Restored database failed post-restore verification; original database put back."
                .into(),
        );
    }
    let _ = std::fs::remove_dir_all(&paths.pending);
    Some(RestoreOutcome {
        applied: true,
        message: format!(
            "Restore applied. Previous database kept at {}",
            replaced_dir.display()
        ),
        replaced_dir: Some(replaced_dir.to_string_lossy().to_string()),
    })
}

async fn rollback(moved: &[(PathBuf, PathBuf)]) {
    for (orig, aside) in moved {
        let _ = move_file(aside, orig).await;
    }
}
