//! paymentSchedule — Tauri application entry point.
//! Opens the SQLite database in the platform app-data directory, manages it as
//! shared state, and registers the command handlers used by the Vue frontend.

mod commands;
mod db;
mod models;
mod seed;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("payment_schedule.db");
            let database = db::Db::open(&db_path)
                .map_err(|e| format!("Failed to open database: {e}"))?;
            app.manage(database);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // clients
            commands::list_clients,
            commands::get_client_detail,
            commands::create_client,
            commands::update_client,
            commands::delete_client,
            // purchases
            commands::list_purchases,
            commands::get_purchase_detail,
            commands::create_purchase,
            commands::delete_purchase,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running paymentSchedule");
}
