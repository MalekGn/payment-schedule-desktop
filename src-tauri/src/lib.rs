//! paymentSchedule — Tauri application entry point.
//! Opens the SQLite database in the platform app-data directory, manages it as
//! shared state, and registers the command handlers used by the Vue frontend.

mod autobackup;
mod commands;
mod db;
mod error;
// `pub` on purpose: nothing calls into the licence validator yet, and a private
// module would make every item unreachable and fail `clippy -D warnings` on
// `dead_code`. Being part of the lib crate's public API is also honest — the
// enforcement task consumes it from outside this file.
pub mod license;
mod models;
mod seed;

use tauri::Manager;

/// Title and body for the pre-migration snapshot failure, per UI language.
///
/// The one piece of user-facing text this crate owns. It cannot come from
/// `src/locales/*.json` like everything else: the dialog fires before the
/// WebView is created, so vue-i18n does not exist yet. Three short strings
/// duplicated here is the price of an Arabic-only shop being told why their app
/// will not open, instead of getting French they cannot read at the one moment
/// it matters. Keep them in step with the `errors.*` keys by hand.
fn snapshot_failure_text(language: Option<&str>) -> (&'static str, &'static str) {
    match language {
        Some("ar") => (
            "تعذّر إنشاء نسخة احتياطية",
            "تعذّر إنشاء نسخة احتياطية قبل تحديث قاعدة البيانات، ولم يتم تعديل بياناتك. أفرغ بعض المساحة على القرص ثم أعد تشغيل التطبيق.",
        ),
        Some("en") => (
            "Backup could not be created",
            "A safety copy could not be created before updating the database, so your data has been left untouched. Free some disk space, then start the application again.",
        ),
        // French is the app's default language and the fallback for anything
        // unrecognised, including a database too old to hold the setting.
        _ => (
            "Copie de sécurité impossible",
            "Impossible de créer une copie de sécurité avant la mise à jour de la base de données ; vos données n'ont pas été modifiées. Libérez de l'espace disque, puis relancez l'application.",
        ),
    }
}

/// Managed only when startup refused to proceed, carrying what to tell the user.
///
/// The refusal has to be decided in `setup` — that is the one place with the
/// app-data path, and it must happen before anything migrates — but it cannot be
/// *shown* there. A native dialog needs a running event loop to pump it, and
/// inside `setup` the loop has not started; `blocking_show` on the main thread is
/// documented to deadlock. So `setup` records the verdict here and returns, and
/// `RunEvent::Ready` shows it once the loop is alive.
struct StartupBlocked {
    title: &'static str,
    body: &'static str,
}

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

            // Snapshot before the schema ladder can advance. Per-step
            // transactions in `migrate` already protect against a migration that
            // *fails*; nothing protects against one that succeeds and is wrong,
            // and this is the only copy the user has.
            //
            // `pending_migration` answers `Some` only when the database exists,
            // carries a schema and holds data, so a fresh install can never be
            // blocked by a snapshot that could not be written.
            if let Some(pending) = db::pending_migration(&db_path)
                .map_err(|e| format!("Failed to inspect the database: {e}"))?
            {
                let snapshot = autobackup::backups_dir(&data_dir).and_then(|dir| {
                    autobackup::snapshot_before_migration(&db_path, &dir, pending.target)
                });
                if let Err(e) = snapshot {
                    // Refuse to migrate rather than advance the schema with no
                    // fallback. The user can free disk space; they cannot undo a
                    // bad migration.
                    //
                    // Returning `Err` here would abort with no window and no
                    // explanation, so instead: leave the database untouched,
                    // hide the window that would otherwise show a UI with no
                    // backend behind it, and let `RunEvent::Ready` explain and
                    // exit. Nothing below runs, so `Db` is never managed and no
                    // command can reach a database this build has not checked.
                    log::error!("refusing to migrate without a pre-migration snapshot: {e}");
                    let (title, body) = snapshot_failure_text(pending.language.as_deref());
                    app.manage(StartupBlocked { title, body });
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                    return Ok(());
                }
            }

            let database =
                db::Db::open(&db_path).map_err(|e| format!("Failed to open database: {e}"))?;

            let backups_dir = autobackup::backups_dir(&data_dir);
            if let Err(e) = &backups_dir {
                log::warn!("no backups directory, automatic backups are disabled: {e}");
            }

            // Validate the licence here rather than per command: it reads a file
            // and hashes a machine identifier. This is the only synchronous
            // evaluation; the watcher started below refreshes it on a tick.
            //
            // Deliberately not propagated with `?`. Returning `Err` from `setup`
            // aborts startup, and "no licence" or "expired" must never stop the
            // app launching — an unlicensed install still lets the shop keeper
            // read their own clients and purchases, and it is the only way they
            // can reach the screen that installs a licence.
            let status = commands::evaluate_license(app.handle(), &database.lock());
            log::info!("licence status at startup: {}", status.to_info(None).status);

            app.manage(database);
            app.manage(license::LicenseState::new(status));

            // Started last, because it reads the managed `Db` on every tick and
            // its first pass runs immediately — which is also the launch
            // catch-up, so nothing else has to check whether a backup is owed.
            // Never fatal: a failure costs a backup, not the working day.
            if let Ok(dir) = backups_dir {
                autobackup::start_scheduler(app.handle().clone(), db_path, dir);
            }

            // The verdict above is a snapshot. Without a watcher it would stand
            // for the life of the process, so a shop that never closes the app
            // would keep full access past its expiry date. Started here, after
            // both states are managed, and never on the refusal path above.
            commands::start_license_watcher(app.handle().clone());

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
            // licence
            commands::get_license_status,
            commands::import_license,
        ])
        .build(tauri::generate_context!())
        .expect("error while running paymentSchedule")
        .run(|handle, event| {
            // The only place a dialog can be shown before the user has a window:
            // the loop is running now, so it can pump one.
            if matches!(event, tauri::RunEvent::Ready) {
                if let Some(blocked) = handle.try_state::<StartupBlocked>() {
                    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

                    let (title, body) = (blocked.title, blocked.body);
                    let handle = handle.clone();
                    // On its own thread, because this callback *is* the main
                    // thread and `blocking_show` there deadlocks. Exiting from
                    // the thread once the user has read it keeps the refusal
                    // final.
                    std::thread::spawn(move || {
                        handle
                            .dialog()
                            .message(body)
                            .title(title)
                            .kind(MessageDialogKind::Error)
                            .blocking_show();
                        handle.exit(1);
                    });
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::snapshot_failure_text;

    /// The dialog must never fall back to a language the shop cannot read, and
    /// an unset or unknown `language` is the common case on a database old
    /// enough to need migrating.
    #[test]
    fn the_abort_dialog_speaks_the_configured_language() {
        assert_eq!(
            snapshot_failure_text(Some("ar")).0,
            "تعذّر إنشاء نسخة احتياطية"
        );
        assert_eq!(
            snapshot_failure_text(Some("en")).0,
            "Backup could not be created"
        );
        assert_eq!(
            snapshot_failure_text(Some("fr")).0,
            "Copie de sécurité impossible"
        );

        // French is the app's own default, so it is what an absent or
        // unrecognised setting resolves to.
        assert_eq!(
            snapshot_failure_text(None),
            snapshot_failure_text(Some("fr"))
        );
        assert_eq!(
            snapshot_failure_text(Some("de")),
            snapshot_failure_text(Some("fr"))
        );

        for lang in [None, Some("ar"), Some("en"), Some("fr")] {
            let (title, body) = snapshot_failure_text(lang);
            // A body that is empty would show an explanationless box.
            assert!(!body.is_empty());
            // These strings were once written with `\` line continuations, which
            // a rewrite silently turned into runs of literal spaces inside the
            // sentence. Nothing but reading the rendered dialog would have shown
            // it, so it is pinned here instead.
            assert!(!title.contains("  "), "{title}");
            assert!(!body.contains("  "), "{body}");
        }
    }
}
