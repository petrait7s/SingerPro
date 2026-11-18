//! Authentication: Argon2id password hashing, first-run owner setup, login with lockout,
//! and in-memory session tokens (never persisted, never exposed to the UI layer).

use crate::audit::{self, AuditEntry};
use crate::authz::{self, Principal, OWNER_ROLE};
use crate::{time, CoreError, CoreResult, Id};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use chrono::{Duration as ChronoDuration, Utc};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const MAX_FAILED_ATTEMPTS: i64 = 5;
pub const LOCKOUT_MINUTES: i64 = 15;
pub const MIN_PASSWORD_LEN: usize = 10;

fn argon() -> Argon2<'static> {
    // OWASP-recommended Argon2id: 19 MiB, 2 iterations, 1 lane.
    let params = Params::new(19 * 1024, 2, 1, None).expect("valid argon2 params");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

pub fn hash_password(password: &str) -> CoreResult<String> {
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes)
        .map_err(|e| CoreError::Validation(format!("entropy source failed: {e}")))?;
    let salt =
        SaltString::encode_b64(&salt_bytes).map_err(|e| CoreError::Validation(e.to_string()))?;
    argon()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| CoreError::Validation(format!("password hashing failed: {e}")))
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => argon()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

const COMMON: &[&str] = &[
    "password",
    "password1",
    "password123",
    "1234567890",
    "12345678910",
    "qwertyuiop",
    "admin12345",
    "letmein123",
    "iloveyou12",
    "singerpro1",
    "singerpro123",
    "welcome123",
];

pub fn validate_password(password: &str, username: &str) -> CoreResult<()> {
    let bad = |m: &str| Err(CoreError::Validation(m.to_string()));
    if password.chars().count() < MIN_PASSWORD_LEN {
        return bad(&format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        ));
    }
    let lower = password.to_lowercase();
    if COMMON.contains(&lower.as_str()) {
        return bad("password is too common");
    }
    if !username.is_empty() && lower.contains(&username.to_lowercase()) {
        return bad("password must not contain the username");
    }
    let first = password.chars().next().unwrap_or(' ');
    if password.chars().all(|c| c == first) {
        return bad("password must not be a single repeated character");
    }
    if password.chars().all(|c| c.is_ascii_digit()) {
        return bad("password must not be only digits");
    }
    Ok(())
}

pub fn normalize_username(raw: &str) -> CoreResult<String> {
    let u = raw.trim().to_lowercase();
    let ok = (3..=32).contains(&u.len())
        && u.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(u)
    } else {
        Err(CoreError::Validation(
            "username must be 3-32 characters: letters, digits, '.', '_' or '-'".into(),
        ))
    }
}

pub async fn needs_setup(pool: &SqlitePool) -> CoreResult<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(n == 0)
}

/// First-run only: creates the Owner. Fails with Conflict once any user exists.
/// The password is chosen by the person installing; there is no default credential anywhere.
pub async fn setup_owner(
    pool: &SqlitePool,
    username: &str,
    full_name: &str,
    password: &str,
) -> CoreResult<Id> {
    let username = normalize_username(username)?;
    let full_name = full_name.trim();
    if full_name.is_empty() {
        return Err(CoreError::Validation("full name is required".into()));
    }
    validate_password(password, &username)?;
    let hash = hash_password(password)?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?; // serialises concurrent setup attempts
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if n > 0 {
        return Err(CoreError::Conflict("setup already completed".into()));
    }
    let owner_role: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(OWNER_ROLE)
        .fetch_one(&mut *tx)
        .await?;
    let id = Id::new();
    let now = time::now_utc_string();
    sqlx::query(
        "INSERT INTO users (id, username, full_name, password_hash, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&username)
    .bind(full_name)
    .bind(&hash)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES (?, ?)")
        .bind(id.to_string())
        .bind(owner_role)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        AuditEntry {
            actor_id: Some(id),
            action: "setup.owner_created",
            entity_type: "user",
            entity_id: Some(&id.to_string()),
            detail: Some(serde_json::json!({ "username": username })),
            reason: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

// Verifying against a dummy hash for unknown users keeps timing similar and avoids user enumeration.
fn dummy_hash() -> &'static str {
    static H: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    H.get_or_init(|| hash_password("not-a-real-password-timing-pad").expect("hash"))
}

const GENERIC_FAIL: &str = "invalid username or password";

/// Returns the user id on success. Failed attempts are counted and audited; the account locks
/// for `LOCKOUT_MINUTES` after `MAX_FAILED_ATTEMPTS`. Error text never reveals which part was wrong.
pub async fn login(pool: &SqlitePool, username: &str, password: &str) -> CoreResult<Id> {
    let uname = username.trim().to_lowercase();
    let row: Option<(String, String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, password_hash, is_active, failed_attempts, locked_until FROM users WHERE username = ?",
    )
    .bind(&uname)
    .fetch_optional(pool)
    .await?;
    let Some((id, hash, active, failed, locked_until)) = row else {
        let _ = verify_password(password, dummy_hash());
        let mut c = pool.acquire().await?;
        audit::record(
            &mut c,
            AuditEntry {
                actor_id: None,
                action: "auth.login_failed",
                entity_type: "user",
                entity_id: None,
                detail: Some(serde_json::json!({ "username": uname, "reason": "unknown_user" })),
                reason: None,
            },
        )
        .await?;
        return Err(CoreError::Validation(GENERIC_FAIL.into()));
    };
    let uid = Id::parse(&id)?;
    let now = Utc::now();
    if let Some(l) = &locked_until {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(l) {
            if t.with_timezone(&Utc) > now {
                let _ = verify_password(password, dummy_hash());
                return Err(CoreError::Forbidden(format!(
                    "account temporarily locked after repeated failures; try again after {}",
                    time::format_dhaka(t.with_timezone(&Utc))
                )));
            }
        }
    }
    let ok = verify_password(password, &hash);
    let mut tx = pool.begin().await?;
    if !ok || active != 1 {
        let reason = if active != 1 {
            "inactive"
        } else {
            "bad_password"
        };
        let new_failed = failed + 1;
        let lock = new_failed >= MAX_FAILED_ATTEMPTS;
        let locked: Option<String> = lock.then(|| {
            (now + ChronoDuration::minutes(LOCKOUT_MINUTES))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        });
        sqlx::query(
            "UPDATE users SET failed_attempts = ?, locked_until = ?, updated_at = ? WHERE id = ?",
        )
        .bind(if lock { 0 } else { new_failed })
        .bind(&locked)
        .bind(time::now_utc_string())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        audit::record(
            &mut tx,
            AuditEntry {
                actor_id: Some(uid),
                action: if lock {
                    "auth.account_locked"
                } else {
                    "auth.login_failed"
                },
                entity_type: "user",
                entity_id: Some(&id),
                detail: Some(serde_json::json!({ "reason": reason })),
                reason: None,
            },
        )
        .await?;
        tx.commit().await?;
        return Err(CoreError::Validation(GENERIC_FAIL.into()));
    }
    sqlx::query(
        "UPDATE users SET failed_attempts = 0, locked_until = NULL, last_login_at = ? WHERE id = ?",
    )
    .bind(time::now_utc_string())
    .bind(&id)
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        AuditEntry {
            actor_id: Some(uid),
            action: "auth.login",
            entity_type: "user",
            entity_id: Some(&id),
            detail: None,
            reason: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(uid)
}

struct Entry {
    user_id: Id,
    last_active: Instant,
    created: Instant,
}

/// In-memory sessions: random 256-bit tokens, only their SHA-256 is stored, idle + absolute expiry.
pub struct SessionStore {
    inner: Mutex<HashMap<String, Entry>>,
    idle: Duration,
    max_age: Duration,
}

fn key_of(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

impl SessionStore {
    pub fn new(idle: Duration, max_age: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            idle,
            max_age,
        }
    }

    /// 30 min idle, 12 h absolute.
    pub fn standard() -> Self {
        Self::new(Duration::from_secs(30 * 60), Duration::from_secs(12 * 3600))
    }

    pub fn create(&self, user_id: Id) -> CoreResult<String> {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b)
            .map_err(|e| CoreError::Validation(format!("entropy source failed: {e}")))?;
        let token = hex::encode(b);
        let now = Instant::now();
        self.inner.lock().expect("session lock").insert(
            key_of(&token),
            Entry {
                user_id,
                last_active: now,
                created: now,
            },
        );
        Ok(token)
    }

    /// Validates and refreshes idle timer. Expired sessions are removed.
    pub fn validate(&self, token: &str) -> Option<Id> {
        let mut m = self.inner.lock().expect("session lock");
        let k = key_of(token);
        let now = Instant::now();
        let e = m.get(&k)?;
        let expired = now.duration_since(e.last_active) > self.idle
            || now.duration_since(e.created) > self.max_age;
        if expired {
            m.remove(&k);
            return None;
        }
        let e = m.get_mut(&k).expect("present");
        e.last_active = now;
        Some(e.user_id)
    }

    pub fn revoke(&self, token: &str) {
        self.inner
            .lock()
            .expect("session lock")
            .remove(&key_of(token));
    }

    pub fn revoke_user(&self, user_id: Id) {
        self.inner
            .lock()
            .expect("session lock")
            .retain(|_, e| e.user_id != user_id);
    }

    pub fn revoke_all(&self) {
        self.inner.lock().expect("session lock").clear();
    }
}

/// Token -> fresh Principal. Any failure is `Unauthenticated`.
pub async fn authenticate(
    pool: &SqlitePool,
    store: &SessionStore,
    token: &str,
) -> CoreResult<Principal> {
    let uid = store.validate(token).ok_or(CoreError::Unauthenticated)?;
    match authz::load_principal(pool, uid).await {
        Ok(p) => Ok(p),
        Err(e) => {
            store.revoke_user(uid);
            Err(e)
        }
    }
}

pub async fn logout(pool: &SqlitePool, store: &SessionStore, token: &str) -> CoreResult<()> {
    if let Some(uid) = store.validate(token) {
        let mut c = pool.acquire().await?;
        audit::record(
            &mut c,
            AuditEntry {
                actor_id: Some(uid),
                action: "auth.logout",
                entity_type: "user",
                entity_id: Some(&uid.to_string()),
                detail: None,
                reason: None,
            },
        )
        .await?;
    }
    store.revoke(token);
    Ok(())
}
