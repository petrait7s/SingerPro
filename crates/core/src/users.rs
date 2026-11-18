//! User and role administration. Every function authorizes the caller and audits the change.

use crate::audit::{self, AuditEntry};
use crate::auth::{self, SessionStore};
use crate::authz::{perm, Principal, OWNER_ROLE};
use crate::{time, CoreError, CoreResult, Id};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;

#[derive(Debug, Serialize)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub full_name: String,
    pub is_active: bool,
    pub must_change_password: bool,
    pub roles: Vec<String>,
    pub last_login_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct RoleRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub is_system: bool,
    pub permissions: Vec<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PermissionRow {
    pub key: String,
    pub category: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct NewUser {
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub role_ids: Vec<String>,
}

pub async fn list_users(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<UserRow>> {
    p.require(perm::USERS_VIEW)?;
    type Row = (String, String, String, i64, i64, Option<String>, String);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, username, full_name, is_active, must_change_password, last_login_at, created_at FROM users ORDER BY username",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, username, full_name, active, must, last, created) in rows {
        let roles: Vec<String> = sqlx::query_scalar(
            "SELECT r.name FROM user_roles ur JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = ? ORDER BY r.name",
        )
        .bind(&id)
        .fetch_all(pool)
        .await?;
        out.push(UserRow {
            id,
            username,
            full_name,
            is_active: active == 1,
            must_change_password: must == 1,
            roles,
            last_login_at: last,
            created_at: created,
        });
    }
    Ok(out)
}

pub async fn list_permissions(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<PermissionRow>> {
    p.require(perm::USERS_VIEW)?;
    Ok(
        sqlx::query_as("SELECT key, category, description FROM permissions ORDER BY category, key")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn list_roles(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<RoleRow>> {
    p.require(perm::USERS_VIEW)?;
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT id, name, description, is_system FROM roles ORDER BY is_system DESC, name",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::new();
    for (id, name, description, sys) in rows {
        let permissions: Vec<String> = sqlx::query_scalar(
            "SELECT perm_key FROM role_permissions WHERE role_id = ? ORDER BY perm_key",
        )
        .bind(&id)
        .fetch_all(pool)
        .await?;
        out.push(RoleRow {
            id,
            name,
            description,
            is_system: sys == 1,
            permissions,
        });
    }
    Ok(out)
}

async fn role_perms(
    conn: &mut SqliteConnection,
    role_id: &str,
) -> CoreResult<(String, HashSet<String>)> {
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM roles WHERE id = ?")
        .bind(role_id)
        .fetch_optional(&mut *conn)
        .await?;
    let name = name.ok_or_else(|| CoreError::NotFound(format!("role {role_id}")))?;
    let perms: Vec<String> =
        sqlx::query_scalar("SELECT perm_key FROM role_permissions WHERE role_id = ?")
            .bind(role_id)
            .fetch_all(&mut *conn)
            .await?;
    Ok((name, perms.into_iter().collect()))
}

/// Privilege-escalation guard: only an Owner may hand out the Owner role; everyone else may only
/// assign roles whose permissions they already hold themselves.
async fn check_can_grant_roles(
    conn: &mut SqliteConnection,
    p: &Principal,
    role_ids: &[String],
) -> CoreResult<()> {
    for rid in role_ids {
        let (name, perms) = role_perms(conn, rid).await?;
        if p.is_owner() {
            continue;
        }
        if name.eq_ignore_ascii_case(OWNER_ROLE) {
            return Err(CoreError::Forbidden(
                "only an Owner can assign the Owner role".into(),
            ));
        }
        if !perms.iter().all(|k| p.has(k)) {
            return Err(CoreError::Forbidden(format!(
                "role '{name}' has permissions you do not hold"
            )));
        }
    }
    Ok(())
}

async fn target_is_owner(conn: &mut SqliteConnection, user_id: &str) -> CoreResult<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_roles ur JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = ? AND r.name = ?",
    )
    .bind(user_id)
    .bind(OWNER_ROLE)
    .fetch_one(&mut *conn)
    .await?;
    Ok(n > 0)
}

async fn active_owner_count(conn: &mut SqliteConnection) -> CoreResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(DISTINCT u.id) FROM users u JOIN user_roles ur ON ur.user_id = u.id
         JOIN roles r ON r.id = ur.role_id WHERE r.name = ? AND u.is_active = 1",
    )
    .bind(OWNER_ROLE)
    .fetch_one(&mut *conn)
    .await?)
}

async fn audit_tx(
    conn: &mut SqliteConnection,
    p: &Principal,
    action: &str,
    entity: &str,
    id: &str,
    detail: serde_json::Value,
) -> CoreResult<()> {
    audit::record(
        conn,
        AuditEntry {
            actor_id: Some(p.user_id),
            action,
            entity_type: entity,
            entity_id: Some(id),
            detail: Some(detail),
            reason: None,
        },
    )
    .await?;
    Ok(())
}

pub async fn create_user(pool: &SqlitePool, p: &Principal, nu: NewUser) -> CoreResult<Id> {
    p.require(perm::USERS_MANAGE)?;
    let username = auth::normalize_username(&nu.username)?;
    if nu.full_name.trim().is_empty() {
        return Err(CoreError::Validation("full name is required".into()));
    }
    if nu.role_ids.is_empty() {
        return Err(CoreError::Validation("assign at least one role".into()));
    }
    auth::validate_password(&nu.password, &username)?;
    let hash = auth::hash_password(&nu.password)?;
    let mut tx = pool.begin().await?;
    check_can_grant_roles(&mut tx, p, &nu.role_ids).await?;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = ?")
        .bind(&username)
        .fetch_one(&mut *tx)
        .await?;
    if exists > 0 {
        return Err(CoreError::Conflict(format!(
            "username '{username}' is taken"
        )));
    }
    let id = Id::new();
    let now = time::now_utc_string();
    // New accounts must pick their own password at first login.
    sqlx::query(
        "INSERT INTO users (id, username, full_name, password_hash, must_change_password, created_at, updated_at)
         VALUES (?, ?, ?, ?, 1, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&username)
    .bind(nu.full_name.trim())
    .bind(hash)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    for r in &nu.role_ids {
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES (?, ?)")
            .bind(id.to_string())
            .bind(r)
            .execute(&mut *tx)
            .await?;
    }
    audit_tx(
        &mut tx,
        p,
        "user.created",
        "user",
        &id.to_string(),
        serde_json::json!({"username": username, "role_ids": nu.role_ids}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn set_user_active(
    pool: &SqlitePool,
    store: &SessionStore,
    p: &Principal,
    user_id: Id,
    active: bool,
    reason: &str,
) -> CoreResult<()> {
    p.require(perm::USERS_MANAGE)?;
    if user_id == p.user_id {
        return Err(CoreError::Validation(
            "you cannot change your own active status".into(),
        ));
    }
    if !active && reason.trim().is_empty() {
        return Err(CoreError::Validation(
            "a reason is required to deactivate a user".into(),
        ));
    }
    let uid = user_id.to_string();
    let mut tx = pool.begin().await?;
    if target_is_owner(&mut tx, &uid).await? && !p.is_owner() {
        return Err(CoreError::Forbidden(
            "only an Owner can modify an Owner account".into(),
        ));
    }
    let res = sqlx::query("UPDATE users SET is_active = ?, updated_at = ? WHERE id = ?")
        .bind(active as i64)
        .bind(time::now_utc_string())
        .bind(&uid)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(CoreError::NotFound("user".into()));
    }
    if active_owner_count(&mut tx).await? < 1 {
        return Err(CoreError::Conflict(
            "at least one active Owner must remain".into(),
        ));
    }
    audit::record(
        &mut tx,
        AuditEntry {
            actor_id: Some(p.user_id),
            action: if active {
                "user.activated"
            } else {
                "user.deactivated"
            },
            entity_type: "user",
            entity_id: Some(&uid),
            detail: None,
            reason: Some(reason.trim()).filter(|r| !r.is_empty()),
        },
    )
    .await?;
    tx.commit().await?;
    if !active {
        store.revoke_user(user_id);
    }
    Ok(())
}

pub async fn set_user_roles(
    pool: &SqlitePool,
    p: &Principal,
    user_id: Id,
    role_ids: Vec<String>,
) -> CoreResult<()> {
    p.require(perm::USERS_MANAGE)?;
    if role_ids.is_empty() {
        return Err(CoreError::Validation("assign at least one role".into()));
    }
    let uid = user_id.to_string();
    let mut tx = pool.begin().await?;
    if target_is_owner(&mut tx, &uid).await? && !p.is_owner() {
        return Err(CoreError::Forbidden(
            "only an Owner can modify an Owner account".into(),
        ));
    }
    check_can_grant_roles(&mut tx, p, &role_ids).await?;
    let before: Vec<String> =
        sqlx::query_scalar("SELECT role_id FROM user_roles WHERE user_id = ? ORDER BY role_id")
            .bind(&uid)
            .fetch_all(&mut *tx)
            .await?;
    sqlx::query("DELETE FROM user_roles WHERE user_id = ?")
        .bind(&uid)
        .execute(&mut *tx)
        .await?;
    for r in &role_ids {
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES (?, ?)")
            .bind(&uid)
            .bind(r)
            .execute(&mut *tx)
            .await?;
    }
    if active_owner_count(&mut tx).await? < 1 {
        return Err(CoreError::Conflict(
            "at least one active Owner must remain".into(),
        ));
    }
    audit_tx(
        &mut tx,
        p,
        "user.roles_changed",
        "user",
        &uid,
        serde_json::json!({"before": before, "after": role_ids}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reset_password(
    pool: &SqlitePool,
    store: &SessionStore,
    p: &Principal,
    user_id: Id,
    new_password: &str,
) -> CoreResult<()> {
    p.require(perm::USERS_MANAGE)?;
    let uid = user_id.to_string();
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(&uid)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("user".into()))?;
    auth::validate_password(new_password, &username)?;
    let hash = auth::hash_password(new_password)?;
    let mut tx = pool.begin().await?;
    if target_is_owner(&mut tx, &uid).await? && !p.is_owner() {
        return Err(CoreError::Forbidden(
            "only an Owner can reset an Owner password".into(),
        ));
    }
    sqlx::query("UPDATE users SET password_hash = ?, must_change_password = 1, failed_attempts = 0, locked_until = NULL, updated_at = ? WHERE id = ?")
        .bind(hash)
        .bind(time::now_utc_string())
        .bind(&uid)
        .execute(&mut *tx)
        .await?;
    audit_tx(
        &mut tx,
        p,
        "user.password_reset",
        "user",
        &uid,
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    store.revoke_user(user_id);
    Ok(())
}

pub async fn change_own_password(
    pool: &SqlitePool,
    store: &SessionStore,
    p: &Principal,
    old: &str,
    new: &str,
) -> CoreResult<()> {
    let uid = p.user_id.to_string();
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(&uid)
        .fetch_one(pool)
        .await?;
    if !auth::verify_password(old, &hash) {
        return Err(CoreError::Validation(
            "current password is incorrect".into(),
        ));
    }
    if old == new {
        return Err(CoreError::Validation(
            "new password must differ from the current one".into(),
        ));
    }
    auth::validate_password(new, &p.username)?;
    let new_hash = auth::hash_password(new)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE users SET password_hash = ?, must_change_password = 0, updated_at = ? WHERE id = ?",
    )
    .bind(new_hash)
    .bind(time::now_utc_string())
    .bind(&uid)
    .execute(&mut *tx)
    .await?;
    audit_tx(
        &mut tx,
        p,
        "user.password_changed",
        "user",
        &uid,
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    store.revoke_user(p.user_id);
    Ok(())
}

pub async fn create_role(
    pool: &SqlitePool,
    p: &Principal,
    name: &str,
    description: &str,
    perms: Vec<String>,
) -> CoreResult<Id> {
    p.require(perm::ROLES_MANAGE)?;
    let name = name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err(CoreError::Validation(
            "role name must be 1-60 characters".into(),
        ));
    }
    if !p.is_owner() && !perms.iter().all(|k| p.has(k)) {
        return Err(CoreError::Forbidden(
            "cannot grant permissions you do not hold".into(),
        ));
    }
    let id = Id::new();
    let mut tx = pool.begin().await?;
    let dup: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE name = ?")
        .bind(name)
        .fetch_one(&mut *tx)
        .await?;
    if dup > 0 {
        return Err(CoreError::Conflict(format!("role '{name}' already exists")));
    }
    sqlx::query(
        "INSERT INTO roles (id, name, description, is_system, created_at) VALUES (?, ?, ?, 0, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(description.trim())
    .bind(time::now_utc_string())
    .execute(&mut *tx)
    .await?;
    for k in &perms {
        sqlx::query("INSERT INTO role_permissions (role_id, perm_key) VALUES (?, ?)")
            .bind(id.to_string())
            .bind(k)
            .execute(&mut *tx)
            .await
            .map_err(|_| CoreError::Validation(format!("unknown permission: {k}")))?;
    }
    audit_tx(
        &mut tx,
        p,
        "role.created",
        "role",
        &id.to_string(),
        serde_json::json!({"name": name, "permissions": perms}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn set_role_permissions(
    pool: &SqlitePool,
    p: &Principal,
    role_id: Id,
    perms: Vec<String>,
) -> CoreResult<()> {
    p.require(perm::ROLES_MANAGE)?;
    let rid = role_id.to_string();
    let mut tx = pool.begin().await?;
    let (name, current) = role_perms(&mut tx, &rid).await?;
    if name.eq_ignore_ascii_case(OWNER_ROLE) {
        return Err(CoreError::Forbidden(
            "the Owner role cannot be modified".into(),
        ));
    }
    if !p.is_owner() {
        let new: HashSet<&String> = perms.iter().collect();
        let touched = current
            .iter()
            .filter(|k| !new.contains(k))
            .chain(perms.iter().filter(|k| !current.contains(*k)));
        for k in touched {
            if !p.has(k) {
                return Err(CoreError::Forbidden(format!(
                    "cannot change permission '{k}' you do not hold"
                )));
            }
        }
    }
    sqlx::query("DELETE FROM role_permissions WHERE role_id = ?")
        .bind(&rid)
        .execute(&mut *tx)
        .await?;
    for k in &perms {
        sqlx::query("INSERT INTO role_permissions (role_id, perm_key) VALUES (?, ?)")
            .bind(&rid)
            .bind(k)
            .execute(&mut *tx)
            .await
            .map_err(|_| CoreError::Validation(format!("unknown permission: {k}")))?;
    }
    let mut before: Vec<String> = current.into_iter().collect();
    before.sort();
    audit_tx(
        &mut tx,
        p,
        "role.permissions_changed",
        "role",
        &rid,
        serde_json::json!({"role": name, "before": before, "after": perms}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete_role(pool: &SqlitePool, p: &Principal, role_id: Id) -> CoreResult<()> {
    p.require(perm::ROLES_MANAGE)?;
    let rid = role_id.to_string();
    let mut tx = pool.begin().await?;
    let row: Option<(String, i64)> =
        sqlx::query_as("SELECT name, is_system FROM roles WHERE id = ?")
            .bind(&rid)
            .fetch_optional(&mut *tx)
            .await?;
    let (name, sys) = row.ok_or_else(|| CoreError::NotFound("role".into()))?;
    if sys == 1 {
        return Err(CoreError::Forbidden(
            "system roles cannot be deleted".into(),
        ));
    }
    let in_use: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_roles WHERE role_id = ?")
        .bind(&rid)
        .fetch_one(&mut *tx)
        .await?;
    if in_use > 0 {
        return Err(CoreError::Conflict("role is assigned to users".into()));
    }
    sqlx::query("DELETE FROM roles WHERE id = ?")
        .bind(&rid)
        .execute(&mut *tx)
        .await?;
    audit_tx(
        &mut tx,
        p,
        "role.deleted",
        "role",
        &rid,
        serde_json::json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
