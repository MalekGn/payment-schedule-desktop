//! SQLite persistence layer (rusqlite). All access to the database goes
//! through the `Db` state, which the Tauri commands lock per call. The
//! frontend never touches the file directly.

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{Months, NaiveDate};
use rusqlite::Connection;

/// Managed Tauri state wrapping a single SQLite connection behind a mutex.
pub struct Db {
    pub conn: Mutex<Connection>,
}

pub type DbResult<T> = Result<T, String>;

impl Db {
    /// Open (creating if needed) the database at `path`, apply the schema, and
    /// seed demo data on a fresh database — development builds only.
    pub fn open(path: &PathBuf) -> DbResult<Self> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| e.to_string())?;
        migrate(&conn)?;
        let db = Db {
            conn: Mutex::new(conn),
        };
        db.seed_if_empty()?;
        Ok(db)
    }

    /// Seed first-run demo data, but only in development builds. Production
    /// bundles (AppImage/deb/MSI/NSIS — built in release mode) ship empty so
    /// end users start with a clean database. Setting `PAYMENT_SCHEDULE_SEED`
    /// to `1`/`true` forces seeding in a release build (useful for QA/demos).
    fn seed_if_empty(&self) -> DbResult<()> {
        if !seeding_enabled() {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM client", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count == 0 {
            crate::seed::seed(&conn)?;
        }
        Ok(())
    }
}

/// Whether first-run demo seeding should run. Enabled in debug builds
/// (`tauri dev`), or when `PAYMENT_SCHEDULE_SEED` is set to a truthy value.
fn seeding_enabled() -> bool {
    seeding_decision(
        cfg!(debug_assertions),
        std::env::var("PAYMENT_SCHEDULE_SEED").ok().as_deref(),
    )
}

/// Pure gate for demo seeding, split out from the compile-time flag and the
/// environment lookup so the policy can be unit-tested deterministically.
/// Seed when this is a development (debug) build, or when the override env var
/// is set to `1`/`true`.
fn seeding_decision(debug_build: bool, seed_env: Option<&str>) -> bool {
    debug_build || matches!(seed_env, Some("1") | Some("true"))
}

/// Apply the schema. Idempotent — safe to call on every startup.
fn migrate(conn: &Connection) -> DbResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS client (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            first_name  TEXT NOT NULL,
            last_name   TEXT NOT NULL,
            phone       TEXT NOT NULL DEFAULT '',
            address     TEXT NOT NULL DEFAULT '',
            email       TEXT,
            created_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS purchase (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            reference         TEXT NOT NULL,
            client_id         INTEGER NOT NULL REFERENCES client(id) ON DELETE CASCADE,
            product_label     TEXT NOT NULL,
            total_price       INTEGER NOT NULL,
            installment_count INTEGER NOT NULL,
            interval_kind     TEXT NOT NULL DEFAULT 'monthly',
            interval_days     INTEGER,
            purchase_date     TEXT NOT NULL,
            created_at        TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS installment (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            purchase_id   INTEGER NOT NULL REFERENCES purchase(id) ON DELETE CASCADE,
            idx           INTEGER NOT NULL,
            amount        INTEGER NOT NULL,
            due_date      TEXT NOT NULL,
            paid_amount   INTEGER NOT NULL DEFAULT 0,
            paid_date     TEXT
        );

        CREATE TABLE IF NOT EXISTS payment (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            installment_id INTEGER NOT NULL REFERENCES installment(id) ON DELETE CASCADE,
            amount         INTEGER NOT NULL,
            payment_date   TEXT NOT NULL,
            note           TEXT,
            created_at     TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS setting (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_purchase_client   ON purchase(client_id);
        CREATE INDEX IF NOT EXISTS idx_inst_purchase     ON installment(purchase_id);
        CREATE INDEX IF NOT EXISTS idx_inst_due          ON installment(due_date);
        CREATE INDEX IF NOT EXISTS idx_payment_inst      ON payment(installment_id);
        "#,
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Date + status helpers (shared by commands and seed)
// ---------------------------------------------------------------------------

/// Today's local date as an ISO string.
pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

pub fn parse_date(s: &str) -> DbResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| format!("Invalid date: {s}"))
}

/// Advance `date` by `k` intervals of the given kind.
pub fn add_interval(date: NaiveDate, kind: &str, interval_days: Option<i64>, k: i64) -> NaiveDate {
    match kind {
        "weekly" => date + chrono::Duration::weeks(k),
        "custom" => date + chrono::Duration::days(interval_days.unwrap_or(30) * k),
        // default monthly
        _ => date
            .checked_add_months(Months::new(k as u32))
            .unwrap_or(date),
    }
}

/// Effective per-installment status computed against `today`.
/// "paid" once fully covered; "late" if past due with a balance;
/// "partial" if part-paid but not yet due; "pending" otherwise.
pub fn installment_status(
    amount: i64,
    paid: i64,
    due: NaiveDate,
    today: NaiveDate,
) -> &'static str {
    if paid >= amount {
        "paid"
    } else if due < today {
        "late"
    } else if paid > 0 {
        "partial"
    } else {
        "pending"
    }
}

/// Roll installment-level states up to a single purchase-level status.
pub fn purchase_status(statuses: &[&str], any_paid: bool) -> &'static str {
    if !statuses.is_empty() && statuses.iter().all(|s| *s == "paid") {
        "paid"
    } else if statuses.contains(&"late") {
        "late"
    } else if any_paid {
        "in_progress"
    } else {
        "pending"
    }
}

/// Compute the equal split of `total` across `n` installments, placing the
/// rounding remainder on the last one so the parts sum exactly to `total`.
pub fn split_amounts(total: i64, n: i64) -> Vec<i64> {
    if n <= 0 {
        return vec![];
    }
    let base = total / n;
    let remainder = total - base * n;
    (0..n)
        .map(|i| if i == n - 1 { base + remainder } else { base })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeding_decision_gate() {
        // Development (debug) build: always seed, whatever the env override says.
        assert!(seeding_decision(true, None));
        assert!(seeding_decision(true, Some("0")));

        // Release build: seed only when explicitly forced on. This `false, None`
        // case is the one that keeps shipped production databases empty.
        assert!(!seeding_decision(false, None));
        assert!(!seeding_decision(false, Some("")));
        assert!(!seeding_decision(false, Some("0")));
        assert!(!seeding_decision(false, Some("yes")));
        assert!(seeding_decision(false, Some("1")));
        assert!(seeding_decision(false, Some("true")));
    }

    fn client_count(db: &Db) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM client", [], |r| r.get(0))
            .unwrap()
    }

    fn temp_db_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut p = std::env::temp_dir();
        p.push(format!(
            "payment_schedule_test_{tag}_{}_{nanos}.db",
            std::process::id()
        ));
        p
    }

    /// Exercises the real `Db::open` wiring: a fresh database is seeded when the
    /// gate is enabled and left empty otherwise. Robust to both `cargo test`
    /// (debug → seeds) and `cargo test --release` (no override → empty).
    #[test]
    fn open_honors_seeding_gate() {
        let path = temp_db_path("open");
        let db = Db::open(&path).unwrap();
        let count = client_count(&db);
        if seeding_enabled() {
            assert!(count > 0, "gate enabled: fresh DB should hold demo clients");
        } else {
            assert_eq!(count, 0, "gate disabled: fresh DB should start empty");
        }
        drop(db);
        let _ = std::fs::remove_file(&path);
    }
}
