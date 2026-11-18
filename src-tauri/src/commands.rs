use crate::state::AppState;
use serde::Serialize;
use singerpro_core::{audit, db, CoreError};
use tauri::State;

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
        data_dir: state.data_dir.display().to_string(),
    }
}

#[tauri::command]
pub async fn db_status(state: State<'_, AppState>) -> Result<db::DbStatus, CoreError> {
    db::status(&state.pool).await
}

#[tauri::command]
pub async fn recent_audit(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<audit::AuditRow>, CoreError> {
    audit::recent(&state.pool, limit.unwrap_or(20)).await
}
