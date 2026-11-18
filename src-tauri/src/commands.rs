//! Thin command layer. Each command: authenticate (from Rust-held session) -> call a core service.
//! Authorization lives in the services; nothing here decides permissions.

use crate::state::AppState;
use base64::Engine;
use serde::Serialize;
use singerpro_core::authz::Principal;
use singerpro_core::recovery::{Marker, RestoreOutcome, StagedRestore};
use singerpro_core::users::{NewUser, PermissionRow, RoleRow, UserRow};
use singerpro_core::{audit, auth, backup, db, recovery, settings, users, CoreError, Id};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tauri::State;

type R<T> = Result<T, CoreError>;

#[derive(Serialize)]
pub struct AppInfo {
    product: &'static str,
    developer: &'static str,
    version: &'static str,
    data_dir: String,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        product: "Singer Pro",
        developer: "PETRA Software Solutions",
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.paths.root.display().to_string(),
    }
}

#[derive(Serialize)]
pub struct SetupStatus {
    needs_setup: bool,
    startup_notice: Option<RestoreOutcome>,
}

/// Public (pre-login): is this a fresh install, and did a restore just run?
#[tauri::command]
pub async fn setup_status(state: State<'_, AppState>) -> R<SetupStatus> {
    Ok(SetupStatus {
        needs_setup: auth::needs_setup(&state.pool).await?,
        startup_notice: state.startup_notice.lock().expect("lock").take(),
    })
}

#[tauri::command]
pub async fn setup_owner(
    state: State<'_, AppState>,
    username: String,
    full_name: String,
    password: String,
) -> R<Principal> {
    let id = auth::setup_owner(&state.pool, &username, &full_name, &password).await?;
    start_session(&state, id).await
}

async fn start_session(state: &AppState, id: Id) -> R<Principal> {
    let token = state.store.create(id)?;
    state.set_token(Some(token));
    state.principal().await
}

#[tauri::command]
pub async fn login(state: State<'_, AppState>, username: String, password: String) -> R<Principal> {
    let id = auth::login(&state.pool, &username, &password).await?;
    start_session(&state, id).await
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> R<()> {
    if let Some(t) = state.token() {
        auth::logout(&state.pool, &state.store, &t).await?;
    }
    state.set_token(None);
    Ok(())
}

/// `None` when not signed in or the session expired.
#[tauri::command]
pub async fn session(state: State<'_, AppState>) -> R<Option<Principal>> {
    match state.principal().await {
        Ok(p) => Ok(Some(p)),
        Err(CoreError::Unauthenticated) => Ok(None),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn change_password(state: State<'_, AppState>, old: String, new: String) -> R<()> {
    let p = state.principal().await?;
    users::change_own_password(&state.pool, &state.store, &p, &old, &new).await?;
    state.set_token(None); // sessions were revoked; the user signs in again with the new password
    Ok(())
}

#[tauri::command]
pub async fn db_status(state: State<'_, AppState>) -> R<db::DbStatus> {
    state.principal().await?;
    db::status(&state.pool).await
}

#[tauri::command]
pub async fn audit_list(
    state: State<'_, AppState>,
    limit: Option<i64>,
    offset: Option<i64>,
    action_prefix: Option<String>,
) -> R<audit::AuditPage> {
    let p = state.principal().await?;
    audit::list(
        &state.pool,
        &p,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        action_prefix.as_deref(),
    )
    .await
}

// ---- users & roles

#[tauri::command]
pub async fn users_list(state: State<'_, AppState>) -> R<Vec<UserRow>> {
    let p = state.principal().await?;
    users::list_users(&state.pool, &p).await
}

#[tauri::command]
pub async fn roles_list(state: State<'_, AppState>) -> R<Vec<RoleRow>> {
    let p = state.principal().await?;
    users::list_roles(&state.pool, &p).await
}

#[tauri::command]
pub async fn permissions_list(state: State<'_, AppState>) -> R<Vec<PermissionRow>> {
    let p = state.principal().await?;
    users::list_permissions(&state.pool, &p).await
}

#[tauri::command]
pub async fn user_create(state: State<'_, AppState>, user: NewUser) -> R<String> {
    let p = state.principal().await?;
    Ok(users::create_user(&state.pool, &p, user).await?.to_string())
}

#[tauri::command]
pub async fn user_set_active(
    state: State<'_, AppState>,
    user_id: String,
    active: bool,
    reason: String,
) -> R<()> {
    let p = state.principal().await?;
    users::set_user_active(
        &state.pool,
        &state.store,
        &p,
        Id::parse(&user_id)?,
        active,
        &reason,
    )
    .await
}

#[tauri::command]
pub async fn user_set_roles(
    state: State<'_, AppState>,
    user_id: String,
    role_ids: Vec<String>,
) -> R<()> {
    let p = state.principal().await?;
    users::set_user_roles(&state.pool, &p, Id::parse(&user_id)?, role_ids).await
}

#[tauri::command]
pub async fn user_reset_password(
    state: State<'_, AppState>,
    user_id: String,
    password: String,
) -> R<()> {
    let p = state.principal().await?;
    users::reset_password(
        &state.pool,
        &state.store,
        &p,
        Id::parse(&user_id)?,
        &password,
    )
    .await
}

#[tauri::command]
pub async fn role_create(
    state: State<'_, AppState>,
    name: String,
    description: String,
    permissions: Vec<String>,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        users::create_role(&state.pool, &p, &name, &description, permissions)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn role_set_permissions(
    state: State<'_, AppState>,
    role_id: String,
    permissions: Vec<String>,
) -> R<()> {
    let p = state.principal().await?;
    users::set_role_permissions(&state.pool, &p, Id::parse(&role_id)?, permissions).await
}

#[tauri::command]
pub async fn role_delete(state: State<'_, AppState>, role_id: String) -> R<()> {
    let p = state.principal().await?;
    users::delete_role(&state.pool, &p, Id::parse(&role_id)?).await
}

// ---- settings & logo

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> R<BTreeMap<String, String>> {
    let p = state.principal().await?;
    settings::get_all(&state.pool, &p).await
}

#[tauri::command]
pub async fn settings_update(
    state: State<'_, AppState>,
    changes: HashMap<String, String>,
) -> R<Vec<String>> {
    let p = state.principal().await?;
    settings::update(&state.pool, &p, changes).await
}

#[tauri::command]
pub async fn branding(state: State<'_, AppState>) -> R<settings::Branding> {
    let p = state.principal().await?;
    settings::branding(&state.pool, &p).await
}

#[tauri::command]
pub async fn logo_get(state: State<'_, AppState>) -> R<Option<String>> {
    let p = state.principal().await?;
    Ok(settings::get_logo(&state.pool, &p)
        .await?
        .map(|(mime, data)| {
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data)
            )
        }))
}

#[tauri::command]
pub async fn logo_set(state: State<'_, AppState>, data_base64: String) -> R<()> {
    let p = state.principal().await?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|_| CoreError::Validation("logo data is not valid base64".into()))?;
    settings::set_logo(&state.pool, &p, &bytes).await
}

#[tauri::command]
pub async fn logo_clear(state: State<'_, AppState>) -> R<()> {
    let p = state.principal().await?;
    settings::clear_logo(&state.pool, &p).await
}

// ---- backup / restore

#[tauri::command]
pub async fn backup_create(
    state: State<'_, AppState>,
    dest_dir: Option<String>,
) -> R<backup::BackupInfo> {
    let p = state.principal().await?;
    let dir = dest_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.paths.default_backups.clone());
    backup::create_backup(&state.pool, &p, &dir).await
}

#[tauri::command]
pub async fn backup_history(state: State<'_, AppState>) -> R<Vec<backup::HistoryRow>> {
    let p = state.principal().await?;
    backup::history(&state.pool, &p, 50).await
}

#[tauri::command]
pub async fn backup_status(state: State<'_, AppState>) -> R<backup::BackupStatus> {
    let p = state.principal().await?;
    backup::status(&state.pool, &p).await
}

#[tauri::command]
pub async fn backup_default_dir(state: State<'_, AppState>) -> R<String> {
    state.principal().await?;
    Ok(state.paths.default_backups.display().to_string())
}

#[tauri::command]
pub async fn backup_verify(state: State<'_, AppState>, path: String) -> R<backup::Preview> {
    let p = state.principal().await?;
    backup::verify_backup(&p, &PathBuf::from(path)).await
}

#[tauri::command]
pub async fn restore_stage(state: State<'_, AppState>, path: String) -> R<StagedRestore> {
    let p = state.principal().await?;
    recovery::stage_restore(&state.pool, &p, &state.paths, &PathBuf::from(path)).await
}

#[tauri::command]
pub async fn restore_pending(state: State<'_, AppState>) -> R<Option<Marker>> {
    state.principal().await?;
    Ok(recovery::pending(&state.paths))
}

#[tauri::command]
pub async fn restore_cancel(state: State<'_, AppState>) -> R<()> {
    let p = state.principal().await?;
    recovery::cancel_pending(&state.pool, &p, &state.paths).await
}

/// Restarts so a staged restore is applied before the database is opened.
#[tauri::command]
pub async fn restart_app(app: tauri::AppHandle, state: State<'_, AppState>) -> R<()> {
    state.principal().await?;
    state.pool.close().await;
    app.restart()
}
