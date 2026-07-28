//! paymentSchedule — Tauri application entry point.
//! Opens the SQLite database in the platform app-data directory, manages it as
//! shared state, and registers the command handlers used by the Vue frontend.

mod commands;
mod db;
mod error;
mod models;
mod seed;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be registered first: it has to win the race before any other
        // plugin or the setup hook opens the database. Two processes on one
        // SQLite file is the classic corruption window, and nothing else stops
        // a user double-launching the app.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch should surface the window the user already has
            // rather than doing nothing visible.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        // Backend diagnostics. Before this, a failing command was stringified
        // to the renderer and recorded nowhere, so a user's bug report could
        // not be traced. Logs carry ids and error codes only — client names,
        // phones, addresses and payment notes are PII and must stay out.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .level(if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                })
                .build(),
        )
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        // No `fs` plugin on purpose: the WebView must never touch the filesystem
        // directly. The SQLite database lives in the app-data dir, so any
        // `fs:allow-read-file`/`fs:allow-write-file` grant over `$APPDATA` would
        // hand the renderer the ledger itself. All persistence goes through the
        // commands below; the only file the renderer reads is the logo, via the
        // asset protocol scoped to `$APPDATA/logo.*` in tauri.conf.json.
        //
        // Hands `tel:`/`sms:` URIs to the OS default handler. Without this the
        // WebView tries to navigate to them itself, fails, and replaces the SPA
        // with its native error page. Scoped in capabilities/default.json.
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("payment_schedule.db");
            let database =
                db::Db::open(&db_path).map_err(|e| format!("Failed to open database: {e}"))?;
            app.manage(database);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // clients
            commands::list_clients,
            commands::get_client_detail,
            commands::create_client,
            commands::update_client,
            commands::archive_client,
            commands::restore_client,
            commands::delete_client,
            // purchases
            commands::list_purchases,
            commands::get_purchase_detail,
            commands::create_purchase,
            commands::update_purchase,
            commands::archive_purchase,
            commands::restore_purchase,
            commands::delete_purchase,
            // installments
            commands::update_installment,
            // payments
            commands::record_payment,
            commands::list_payments_for_purchase,
            commands::list_payments_for_client,
            commands::list_all_payments,
            // impayés / échéances / dashboard
            commands::list_impayes,
            commands::list_schedule,
            commands::get_dashboard,
            // settings
            commands::get_settings,
            commands::update_settings,
            commands::set_logo,
            commands::clear_logo,
            commands::backup_database,
        ])
        .run(tauri::generate_context!())
        .expect("error while running paymentSchedule");
}
