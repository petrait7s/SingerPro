//! Tauri boundary: narrow, typed commands only. No SQL or business rules live here.

mod commands;
mod state;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Production data lives in the OS app-data dir, never the install dir.
            let dir = app.path().app_data_dir()?;
            let db_path = dir.join("data").join("singerpro.db");
            let pool = tauri::async_runtime::block_on(singerpro_core::db::open(&db_path))
                .map_err(|e| format!("database startup failed: {e}"))?;
            app.manage(state::AppState {
                pool,
                data_dir: dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::db_status,
            commands::recent_audit
        ])
        .run(tauri::generate_context!())
        .expect("error while running Singer Pro");
}
