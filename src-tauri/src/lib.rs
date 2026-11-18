//! Tauri boundary: narrow, typed commands only. No SQL or business rules live here.

mod commands;
mod state;

use singerpro_core::recovery::{self, DataPaths};
use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Production data lives in the OS app-data dir, never the install dir.
            let root = app.path().app_data_dir()?;
            let paths = DataPaths::new(&root);
            // A staged restore is applied BEFORE the database is opened (no locks, fully reversible).
            let notice = tauri::async_runtime::block_on(recovery::apply_pending(&paths));
            let pool = tauri::async_runtime::block_on(singerpro_core::db::open(&paths.db))
                .map_err(|e| format!("database startup failed: {e}"))?;
            app.manage(state::AppState::new(pool, paths, notice));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::setup_status,
            commands::setup_owner,
            commands::login,
            commands::logout,
            commands::session,
            commands::change_password,
            commands::db_status,
            commands::audit_list,
            commands::users_list,
            commands::roles_list,
            commands::permissions_list,
            commands::user_create,
            commands::user_set_active,
            commands::user_set_roles,
            commands::user_reset_password,
            commands::role_create,
            commands::role_set_permissions,
            commands::role_delete,
            commands::settings_get,
            commands::settings_update,
            commands::branding,
            commands::logo_get,
            commands::logo_set,
            commands::logo_clear,
            commands::backup_create,
            commands::backup_history,
            commands::backup_status,
            commands::backup_default_dir,
            commands::backup_verify,
            commands::restore_stage,
            commands::restore_pending,
            commands::restore_cancel,
            commands::restart_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Singer Pro");
}
