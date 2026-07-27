//! SQLite persistence layer (rusqlite). All access to the database goes
//! through the `Db` state, which the Tauri commands lock per call. The
//! frontend never touches the file directly.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use chrono::{Months, NaiveDate};
use rusqlite::Connection;

pub use crate::error::AppError;

/// Managed Tauri state wrapping a single SQLite connection behind a mutex.
pub struct Db {
    pub conn: Mutex<Connection>,
}

pub type DbResult<T> = Result<T, AppError>;

impl Db {
    /// Open (creating if needed) the database at `path`, apply the schema, and
    /// seed demo data on a fresh database — development builds only.
    pub fn open(path: &PathBuf) -> DbResult<Self> {
        let conn = Connection::open(path)?;
        // WAL lets readers proceed during a write and is the standard choice
        // for a desktop app; `busy_timeout` makes contention retry for 5s
        // instead of failing instantly with SQLITE_BUSY; `synchronous=NORMAL`
        // is the documented safe pairing with WAL.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;
        migrate(&conn)?;
        let db = Db {
            conn: Mutex::new(conn),
        };
        db.seed_if_empty()?;
        Ok(db)
    }

    /// Lock the shared connection, tolerating a poisoned mutex.
    ///
    /// Poisoning only happens when another thread panicked while holding the
    /// lock. In release that cannot be observed (`panic = "abort"` has already
    /// killed the process), but under `tauri dev` a plain `.unwrap()` here
    /// meant one panicking command bricked *every* later command — the app kept
    /// rendering while nothing worked. The data behind the guard is a SQLite
    /// connection whose own consistency is protected by transactions, so
    /// recovering the guard is sound.
    pub fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Seed first-run demo data, but only in development builds. Production
    /// bundles (AppImage/deb/MSI/NSIS — built in release mode) ship empty so
    /// end users start with a clean database. Setting `PAYMENT_SCHEDULE_SEED`
    /// to `1`/`true` forces seeding in a release build (useful for QA/demos).
    fn seed_if_empty(&self) -> DbResult<()> {
        if !seeding_enabled() {
            return Ok(());
        }
        let conn = self.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM client", [], |r| r.get(0))?;
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

/// Ordered schema migrations. **Append only — never reorder or edit a step
/// that has shipped**, because `PRAGMA user_version` records how many of these
/// a given database has already seen.
///
/// The index in this slice *is* the version: after applying `MIGRATIONS[i]`,
/// `user_version` becomes `i + 1`.
const MIGRATIONS: &[fn(&Connection) -> DbResult<()>] = &[m0001_initial_schema];

/// Bring the database up to the latest schema version.
///
/// Databases created before versioning existed sit at `user_version = 0` with
/// the v1 tables already present. That is safe here precisely because
/// `m0001_initial_schema` is the historical `CREATE TABLE IF NOT EXISTS` batch
/// verbatim: re-running it on such a database is a no-op that simply stamps the
/// version on. This is why the ladder has to land *before* the next schema
/// change rather than alongside it.
///
/// Each step runs inside its own transaction together with the version bump, so
/// a failure can never leave a half-applied schema recorded as complete.
fn migrate(conn: &Connection) -> DbResult<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let current = current.max(0) as usize;

    if current > MIGRATIONS.len() {
        // The file was written by a newer build. Refusing beats silently
        // operating against a schema this binary does not understand.
        log::error!(
            "database schema version {current} is newer than this build supports ({})",
            MIGRATIONS.len()
        );
        return Err(AppError::internal(format!(
            "database schema version {current} is newer than supported {}",
            MIGRATIONS.len()
        )));
    }

    for (i, step) in MIGRATIONS.iter().enumerate().skip(current) {
        let version = i + 1;
        log::info!("applying schema migration {version}");
        conn.execute_batch("BEGIN")?;
        let applied = step(conn).and_then(|()| {
            // `PRAGMA user_version` does not accept a bound parameter, and
            // `version` is a usize derived from a compile-time slice index —
            // never from user input.
            conn.execute_batch(&format!("PRAGMA user_version = {version}"))?;
            Ok(())
        });
        match applied {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }
    Ok(())
}

/// v1 — the original schema. Frozen: see [`MIGRATIONS`].
fn m0001_initial_schema(conn: &Connection) -> DbResult<()> {
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
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Date + status helpers (shared by commands and seed)
// ---------------------------------------------------------------------------

/// Today's local date as an ISO string.
pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

pub fn parse_date(s: &str) -> DbResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
        // The offending value goes to the log, not to the renderer.
        log::warn!("rejected malformed date: {s:?}");
        AppError::validation(INVALID_DATE)
    })
}

/// The three interval kinds a purchase schedule may use.
pub const INTERVAL_KINDS: [&str; 3] = ["weekly", "monthly", "custom"];

/// Inclusive bounds on a custom interval, in days.
pub const INTERVAL_DAYS_RANGE: std::ops::RangeInclusive<i64> = 1..=365;

/// Inclusive bounds on how many installments one purchase may be split into.
/// The upper bound exists so a hostile `installmentCount` cannot drive an
/// unbounded `Vec` allocation and insert loop; 120 is ten years of monthly
/// payments, far beyond anything a shop writes.
pub const INSTALLMENT_COUNT_RANGE: std::ops::RangeInclusive<i64> = 1..=120;

/// Inclusive bounds on the dashboard's "due soon" horizon, in days.
pub const UPCOMING_DAYS_RANGE: std::ops::RangeInclusive<i64> = 1..=365;

// Error codes. Kept as constants so the Rust guard and the doc table in
// `error.rs` cannot drift apart, and so a typo is a compile error.
pub const INVALID_DATE: &str = "INVALID_DATE";
pub const INVALID_TOTAL_PRICE: &str = "INVALID_TOTAL_PRICE";
pub const INVALID_INSTALLMENT_COUNT: &str = "INVALID_INSTALLMENT_COUNT";
pub const INVALID_INTERVAL_KIND: &str = "INVALID_INTERVAL_KIND";
pub const INVALID_INTERVAL_DAYS: &str = "INVALID_INTERVAL_DAYS";
pub const INVALID_AMOUNT: &str = "INVALID_AMOUNT";
pub const SUM_MISMATCH: &str = "SUM_MISMATCH";
pub const OVERPAYMENT: &str = "OVERPAYMENT";
pub const CLIENT_HAS_PURCHASES: &str = "CLIENT_HAS_PURCHASES";
pub const CLIENT_NOT_FOUND: &str = "CLIENT_NOT_FOUND";
pub const PURCHASE_NOT_FOUND: &str = "PURCHASE_NOT_FOUND";
pub const INSTALLMENT_NOT_FOUND: &str = "INSTALLMENT_NOT_FOUND";
pub const INVALID_LOGO_TYPE: &str = "INVALID_LOGO_TYPE";
pub const LOGO_TOO_LARGE: &str = "LOGO_TOO_LARGE";
pub const BACKUP_FAILED: &str = "BACKUP_FAILED";

/// Advance `date` by `k` intervals of the given kind.
///
/// Every arm is overflow-safe and saturates to `date`. The naive forms
/// (`date + Duration::days(n)`, `Duration::days` itself) *panic* on overflow,
/// and with `panic = "abort"` in the release profile a panic here would abort
/// the whole app — so an out-of-range `interval_days` reaching this function
/// from IPC used to be a remote kill switch. Callers still validate their
/// inputs; this is the second line of defence.
pub fn add_interval(date: NaiveDate, kind: &str, interval_days: Option<i64>, k: i64) -> NaiveDate {
    let offset_days = |days: i64| -> NaiveDate {
        chrono::TimeDelta::try_days(days)
            .and_then(|d| date.checked_add_signed(d))
            .unwrap_or(date)
    };
    match kind {
        "weekly" => k.checked_mul(7).map(offset_days).unwrap_or(date),
        "custom" => interval_days
            .unwrap_or(30)
            .checked_mul(k)
            .map(offset_days)
            .unwrap_or(date),
        // default monthly
        _ => u32::try_from(k)
            .ok()
            .and_then(|months| date.checked_add_months(Months::new(months)))
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
        db.lock()
            .query_row("SELECT COUNT(*) FROM client", [], |r| r.get(0))
            .unwrap()
    }

    /// `add_interval` must saturate, never panic, on inputs that overflow.
    ///
    /// This is not hypothetical: `interval_days` arrives from the renderer, and
    /// the naive `date + Duration::days(n)` these arms used to use panics on
    /// overflow. With `panic = "abort"` in the release profile that turned a
    /// single bad IPC argument into a process kill.
    #[test]
    fn add_interval_saturates_instead_of_panicking() {
        let d = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();

        for kind in ["weekly", "monthly", "custom"] {
            for k in [i64::MAX, i64::MIN, 1_000_000_000] {
                let out = add_interval(d, kind, Some(i64::MAX), k);
                assert!(
                    out >= NaiveDate::MIN && out <= NaiveDate::MAX,
                    "{kind}/{k} produced an out-of-range date"
                );
            }
        }

        // The overflowing cases specifically fall back to the input date.
        assert_eq!(add_interval(d, "custom", Some(i64::MAX), i64::MAX), d);
        assert_eq!(add_interval(d, "weekly", None, i64::MAX), d);
        // A negative k on the monthly arm used to wrap through `k as u32`.
        assert_eq!(add_interval(d, "monthly", None, -1), d);
    }

    /// Parity with `src/lib/finance.ts`, over the shared fixture.
    ///
    /// CLAUDE.md treats a divergence between the two implementations as a
    /// blocker: it would mean the schedule previewed in the UI is not the one
    /// written to the database. The TS half is
    /// `src/lib/finance-parity.test.ts`, reading this same file.
    #[test]
    fn finance_parity_fixture() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/fixtures/finance-parity.json"
        );
        let raw = std::fs::read_to_string(path).expect("parity fixture must exist");
        let fixture: serde_json::Value = serde_json::from_str(&raw).unwrap();

        let cases = fixture["splitAmounts"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let total = case["total"].as_i64().unwrap();
            let n = case["n"].as_i64().unwrap();
            let expected: Vec<i64> = case["expected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect();
            assert_eq!(
                split_amounts(total, n),
                expected,
                "split_amounts({total}, {n}) diverges from finance.ts"
            );
        }

        let cases = fixture["addInterval"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let date = parse_date(case["date"].as_str().unwrap()).unwrap();
            let kind = case["kind"].as_str().unwrap();
            let interval_days = case["intervalDays"].as_i64();
            let k = case["k"].as_i64().unwrap();
            let expected = case["expected"].as_str().unwrap();
            assert_eq!(
                add_interval(date, kind, interval_days, k).to_string(),
                expected,
                "add_interval({date}, {kind}, {interval_days:?}, {k}) diverges from finance.ts"
            );
        }

        let cases = fixture["installmentStatus"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let amount = case["amount"].as_i64().unwrap();
            let paid = case["paid"].as_i64().unwrap();
            let due = parse_date(case["dueDate"].as_str().unwrap()).unwrap();
            let today = parse_date(case["today"].as_str().unwrap()).unwrap();
            let expected = case["expected"].as_str().unwrap();
            assert_eq!(
                installment_status(amount, paid, due, today),
                expected,
                "installment_status({amount}, {paid}, {due}) diverges from finance.ts"
            );
        }
    }

    /// `migrate` is version-tracked and idempotent.
    ///
    /// The case that matters is the second one: databases already in the field
    /// sit at `user_version = 0` with the v1 tables present, and must migrate
    /// forward without losing their rows.
    #[test]
    fn migrate_is_versioned_and_idempotent() {
        let path = temp_db_path("migrate");
        {
            let db = Db::open(&path).unwrap();
            let conn = db.lock();
            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(version as usize, MIGRATIONS.len());

            conn.execute(
                "INSERT INTO client (first_name, last_name) VALUES ('Parity', 'MigrationSentinel')",
                [],
            )
            .unwrap();
        }

        // Simulate a pre-versioning database: tables present, version reset.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA user_version = 0").unwrap();
        }

        let db = Db::open(&path).unwrap();
        let conn = db.lock();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            version as usize,
            MIGRATIONS.len(),
            "version must be stamped"
        );
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM client WHERE last_name = 'MigrationSentinel'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "re-running migrations must not drop data");

        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// The durability PRAGMAs actually took effect on the live connection.
    #[test]
    fn open_applies_durability_pragmas() {
        let path = temp_db_path("pragmas");
        let db = Db::open(&path).unwrap();
        let conn = db.lock();

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");

        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 5000);

        // Cascade deletes depend on this being on, and it is per-connection.
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);

        drop(conn);
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
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
