//! SQLite persistence layer (rusqlite). All access to the database goes
//! through the `Db` state, which the Tauri commands lock per call. The
//! frontend never touches the file directly.

use std::path::{Path, PathBuf};
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
const MIGRATIONS: &[fn(&Connection) -> DbResult<()>] = &[
    m0001_initial_schema,
    m0002_client_archive,
    m0003_purchase_archive,
    m0004_payment_date_index,
];

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

/// What a launch needs to know about the database *before* opening it properly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMigration {
    /// The version the ladder is about to advance to.
    pub target: usize,
    /// The UI language stored in `setting`, used for the one dialog that has to
    /// speak to the user before the WebView exists. `None` when unreadable.
    pub language: Option<String>,
}

/// Report whether opening this path would migrate a database that holds data.
///
/// Called before [`Db::open`] so the caller can snapshot first. It answers
/// `None` whenever a snapshot would protect nothing, which is what keeps a
/// failed snapshot from ever blocking a fresh install:
///
/// - **The file does not exist.** Checked before anything opens a connection,
///   because `Connection::open` *creates* the file — open first and every first
///   launch looks like a pending migration.
/// - **There is no `client` table.** A file that exists but was never migrated
///   (created and abandoned, or a zero-byte leftover) has nothing to lose.
/// - **The version is already current**, or ahead of this build. Being ahead is
///   [`migrate`]'s refusal to make, not this function's — answering `None` hands
///   it back the untouched error path.
pub fn pending_migration(path: &Path) -> DbResult<Option<PendingMigration>> {
    if !path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(path)?;
    let has_client: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'client')",
        [],
        |r| r.get(0),
    )?;
    if !has_client {
        return Ok(None);
    }

    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current.max(0) as usize >= MIGRATIONS.len() {
        return Ok(None);
    }

    // Best-effort: a database old enough to need migrating may predate the
    // `setting` table, and an unreadable language must not stop the snapshot
    // that the caller is here to take.
    let language = conn
        .query_row(
            "SELECT value FROM setting WHERE key = 'language'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok();

    Ok(Some(PendingMigration {
        target: MIGRATIONS.len(),
        language,
    }))
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

/// Add a column only if the table does not already have it.
///
/// Every `ALTER` migration must go through this. The ladder has to survive a
/// replay from zero, SQLite has no `ADD COLUMN IF NOT EXISTS`, and a blind
/// `ALTER TABLE` on a database that already has the column fails with
/// "duplicate column name" — taking `Db::open`, and with it the whole app,
/// down on launch. This is the `ALTER` equivalent of what
/// `CREATE TABLE IF NOT EXISTS` does for `m0001`.
///
/// `table` and `column` are compile-time literals from the migration bodies
/// below, never caller input, so interpolating them is safe.
fn add_column_if_missing(conn: &Connection, table: &str, column: &str, ddl: &str) -> DbResult<()> {
    let already_present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        [table, column],
        |r| r.get(0),
    )?;
    if already_present == 0 {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ddl};"))?;
    }
    Ok(())
}

/// v2 — soft archive for clients. `NULL` means active; an ISO date means
/// archived, so the UI gets "archived on <date>" without a second column.
///
/// Archiving replaced the destructive `force` cascade on `delete_client`: a
/// client with purchases is now hidden rather than erased, and can be restored.
fn m0002_client_archive(conn: &Connection) -> DbResult<()> {
    add_column_if_missing(conn, "client", "archived_at", "TEXT")?;
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_client_archived ON client(archived_at);")?;
    Ok(())
}

/// v3 — soft archive for purchases, replacing the destructive delete.
///
/// Unlike an archived *client* — who is settled, and therefore contributes
/// nothing to the money aggregates either way — an archived *purchase* must
/// leave every total: a removed purchase is no longer owed. Every money read
/// model filters on this column; see `commands.rs`.
fn m0003_purchase_archive(conn: &Connection) -> DbResult<()> {
    add_column_if_missing(conn, "purchase", "archived_at", "TEXT")?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_purchase_archived ON purchase(archived_at);",
    )?;
    Ok(())
}

/// v4 — index `payment.payment_date`.
///
/// Every report aggregate filters or groups on this column, and the payment
/// ledger has always ordered by it, but nothing indexed it: both were full
/// scans. Index-only, so it is additive and replay-safe by construction — there
/// is no `add_column_if_missing` dance because `CREATE INDEX IF NOT EXISTS` is
/// already idempotent.
fn m0004_payment_date_index(conn: &Connection) -> DbResult<()> {
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_payment_date ON payment(payment_date);")?;
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

/// Inclusive bounds on how many rows the payment ledger will return at once.
///
/// The lower bound is the load-bearing half: **SQLite treats a negative `LIMIT`
/// as no limit at all**, so binding a caller's value straight in made
/// `listAllPayments(-1)` return every payment ever recorded — a four-table join
/// serialized whole across IPC. The upper bound is ordinary good manners; the UI
/// asks for 500.
pub const PAYMENT_LIMIT_RANGE: std::ops::RangeInclusive<i64> = 1..=5000;

/// Inclusive bounds on any money figure arriving from the renderer: a purchase
/// total, or one installment's share of it. Whole currency units, so a billion
/// is far past anything a shop writes and still leaves enormous headroom.
///
/// The headroom is the point. `SUM(amount)` is computed as `i64` in
/// `resolve_schedule`, and the release profile does not enable
/// `overflow-checks`, so a wrapping sum could otherwise satisfy the
/// `SUM_MISMATCH` equality it is meant to prove — `[i64::MAX, i64::MAX, 1002]`
/// wraps to exactly 1000. Capping each term at 1e9 against a schedule of at most
/// [`INSTALLMENT_COUNT_RANGE`] entries bounds any sum at 1.2e11, nine orders of
/// magnitude below `i64::MAX`, so no validated input can reach the wrap at all.
/// That is why this is a bound and not an `overflow-checks` flag: the flag would
/// turn the wrap into an abort under `panic = "abort"`, where this makes it
/// unreachable.
pub const MONEY_RANGE: std::ops::RangeInclusive<i64> = 0..=1_000_000_000;

/// Inclusive caps on free-text fields arriving from the renderer, **counted in
/// `chars()` and not bytes** — `Médina`, `Réfrigérateur` and the Arabic locale
/// would make a byte cap behave differently depending on the alphabet.
///
/// Nothing here is validated by SQLite: `TEXT` columns are unbounded and a
/// `VARCHAR(n)` would be ignored. Without a cap the renderer can persist a
/// multi-megabyte name, which is then read back into every list view, every
/// export and every dashboard card. The real data these bound is tiny — the
/// longest address in use is 29 characters and the longest product label 26 —
/// so these are generous by two orders of magnitude and exist only to stop the
/// pathological case.
pub const SHORT_TEXT_MAX: usize = 120;
/// As [`SHORT_TEXT_MAX`], for the two fields that are genuinely prose: a postal
/// address and the shop's free-form contact block.
pub const LONG_TEXT_MAX: usize = 500;

/// The languages the app ships translations for. Mirrors `SUPPORTED_LOCALES` in
/// `src/i18n/index.ts`; a value outside it leaves the UI falling back to French
/// forever with no way to tell why.
pub const LANGUAGES: [&str; 3] = ["fr", "en", "ar"];

/// The currencies the settings page offers. Mirrors `CURRENCIES` in
/// `src/stores/settings.ts`. An allow-list rather than an `[A-Z]{3}` shape
/// check, because `FCFA` is four characters.
pub const CURRENCY_CODES: [&str; 6] = ["TND", "EUR", "USD", "FCFA", "DZD", "MAD"];

/// The date patterns the settings page offers. Mirrors `DATE_FORMATS` in
/// `src/stores/settings.ts`. `formatDatePattern` substitutes into these for
/// every date the app renders, so an unrecognised pattern is repeated into
/// every row of every table.
pub const DATE_FORMATS: [&str; 4] = ["dd/MM/yyyy", "MM/dd/yyyy", "yyyy-MM-dd", "dd-MM-yyyy"];

/// How often the automatic backup runs. Mirrors `BACKUP_FREQUENCIES` in
/// `src/stores/settings.ts`. Expressed as an interval rather than a calendar
/// rule — see `autobackup::Frequency` for why a weekday picker was not worth
/// the ways it can be configured into never firing.
pub const BACKUP_FREQUENCIES: [&str; 3] = ["daily", "weekly", "monthly"];

/// The period sizes a report may bucket its collections into. Mirrors
/// `REPORT_GRANULARITIES` in `src/types/models.ts`. Each maps to an `strftime`
/// pattern in `commands::period_format`, so bucketing and ordering agree by
/// construction rather than by two implementations happening to match.
pub const REPORT_GRANULARITIES: [&str; 3] = ["day", "month", "year"];

/// Inclusive bounds on a report's span, in days.
///
/// The upper bound is what stops a hostile — or merely mistyped — range from
/// materializing one zero-filled bucket per day across a century. `day`
/// granularity is only ever auto-selected below [`REPORT_DAY_MAX_SPAN`], but a
/// caller may ask for it explicitly, so the bucket count has to be bounded here
/// rather than left to the granularity heuristic.
pub const REPORT_SPAN_DAYS_RANGE: std::ops::RangeInclusive<i64> = 1..=36_525;

/// Longest span still bucketed by day, and by month, when the caller does not
/// choose. Beyond the second, a report falls back to yearly buckets.
pub const REPORT_DAY_MAX_SPAN: i64 = 62;
pub const REPORT_MONTH_MAX_SPAN: i64 = 730;

/// Most buckets one collections series may carry.
///
/// [`REPORT_SPAN_DAYS_RANGE`] already bounds the *allocation*, but not the size
/// of the response: an explicit `day` granularity across a century is a legal
/// request that would serialize 36 525 points across IPC and ask the renderer to
/// draw as many bars. The auto-selected granularity never comes close — a
/// century resolves to 100 yearly buckets — so this only ever refuses a request
/// the UI does not make, in the same spirit as [`PAYMENT_LIMIT_RANGE`].
pub const REPORT_MAX_BUCKETS: usize = 1_000;

/// How many rows the "top clients" and "top products" tables carry. A report is
/// a page someone reads, not a data export; the CSV carries the same ten.
pub const REPORT_TOP_N: i64 = 10;

/// Largest CSV the renderer may ask to have written to disk, in bytes.
///
/// A report or an overdue list is a few hundred kilobytes at worst. The cap is
/// here because `export_csv` takes its payload straight from the renderer, so
/// without one a hostile WebView could ask the backend to fill a disk.
pub const EXPORT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Time of day the automatic backup runs, when the shop has not chosen one.
/// Late enough to catch a full trading day, early enough that someone is still
/// there to see a failure.
pub const DEFAULT_BACKUP_TIME: &str = "17:00";

// Error codes. Kept as constants so the Rust guard and the doc table in
// `error.rs` cannot drift apart, and so a typo is a compile error.
pub const INVALID_DATE: &str = "INVALID_DATE";
pub const INVALID_TOTAL_PRICE: &str = "INVALID_TOTAL_PRICE";
pub const INVALID_INSTALLMENT_COUNT: &str = "INVALID_INSTALLMENT_COUNT";
pub const INVALID_INTERVAL_KIND: &str = "INVALID_INTERVAL_KIND";
pub const INVALID_INTERVAL_DAYS: &str = "INVALID_INTERVAL_DAYS";
pub const INVALID_AMOUNT: &str = "INVALID_AMOUNT";
pub const SUM_MISMATCH: &str = "SUM_MISMATCH";
pub const INSTALLMENT_COUNT_MISMATCH: &str = "INSTALLMENT_COUNT_MISMATCH";
pub const TEXT_TOO_LONG: &str = "TEXT_TOO_LONG";
pub const TEXT_REQUIRED: &str = "TEXT_REQUIRED";
pub const INVALID_SETTING_VALUE: &str = "INVALID_SETTING_VALUE";
pub const OVERPAYMENT: &str = "OVERPAYMENT";
pub const CLIENT_HAS_PURCHASES: &str = "CLIENT_HAS_PURCHASES";
pub const ARCHIVE_HAS_OUTSTANDING: &str = "ARCHIVE_HAS_OUTSTANDING";
pub const CLIENT_ARCHIVED: &str = "CLIENT_ARCHIVED";
pub const CLIENT_NOT_FOUND: &str = "CLIENT_NOT_FOUND";
pub const PURCHASE_NOT_FOUND: &str = "PURCHASE_NOT_FOUND";
pub const PURCHASE_HAS_PAYMENTS: &str = "PURCHASE_HAS_PAYMENTS";
pub const PURCHASE_ARCHIVED: &str = "PURCHASE_ARCHIVED";
pub const PURCHASE_NOT_ARCHIVED: &str = "PURCHASE_NOT_ARCHIVED";
pub const INSTALLMENT_NOT_FOUND: &str = "INSTALLMENT_NOT_FOUND";
pub const AMOUNT_LOCKED: &str = "AMOUNT_LOCKED";
pub const DUE_DATE_LOCKED: &str = "DUE_DATE_LOCKED";
pub const DUE_DATE_OUT_OF_ORDER: &str = "DUE_DATE_OUT_OF_ORDER";
pub const SCHEDULE_VIA_PURCHASE: &str = "SCHEDULE_VIA_PURCHASE";
pub const PAID_ABOVE_AMOUNT: &str = "PAID_ABOVE_AMOUNT";
pub const NO_PAYMENT_TO_DATE: &str = "NO_PAYMENT_TO_DATE";
pub const PAYMENT_DATE_LOCKED: &str = "PAYMENT_DATE_LOCKED";
pub const FUTURE_PAID_DATE: &str = "FUTURE_PAID_DATE";
pub const PREVIOUS_UNPAID: &str = "PREVIOUS_UNPAID";
pub const BELOW_PAID: &str = "BELOW_PAID";
/// Raised by the purchase editor in the frontend, not by any Rust guard — see
/// [`rebalance_amounts`]. Kept here so the code inventory stays complete.
#[allow(dead_code)]
pub const NO_REBALANCE_ROOM: &str = "NO_REBALANCE_ROOM";
pub const INVALID_LOGO_TYPE: &str = "INVALID_LOGO_TYPE";
pub const LOGO_TOO_LARGE: &str = "LOGO_TOO_LARGE";
pub const BACKUP_FAILED: &str = "BACKUP_FAILED";
pub const EXPORT_FAILED: &str = "EXPORT_FAILED";
pub const INVALID_GRANULARITY: &str = "INVALID_GRANULARITY";
pub const REPORT_RANGE_TOO_LONG: &str = "REPORT_RANGE_TOO_LONG";
pub const LICENSE_REQUIRED: &str = "LICENSE_REQUIRED";
pub const INVALID_LICENSE: &str = "INVALID_LICENSE";

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

/// Re-split `pool` across the installments at `absorbers` (indices into
/// `amounts`), refusing any distribution that would push a row below what has
/// already been collected on it.
///
/// `None` means this absorber set cannot take the change: either the pool went
/// negative, or an even split lands under someone's `paid_amount` — which would
/// break the `paid_amount <= amount` invariant the outstanding aggregates rely
/// on.
#[allow(dead_code)] // Parity anchor; see `rebalance_amounts`.
fn apply_pool(
    amounts: &[i64],
    paid_amounts: &[i64],
    absorbers: &[usize],
    pool: i64,
) -> Option<Vec<i64>> {
    if absorbers.is_empty() || pool < 0 {
        return None;
    }
    let parts = split_amounts(pool, absorbers.len() as i64);
    let mut next = amounts.to_vec();
    for (part, &i) in parts.iter().zip(absorbers) {
        if *part < paid_amounts[i] {
            return None;
        }
        next[i] = *part;
    }
    Some(next)
}

/// The new amount vector after setting installment `index` (0-based) to
/// `new_amount`, holding the purchase total fixed.
///
/// `SUM(amount) == purchase.total_price` is assumed by every read model in the
/// app, so a single-installment edit has to move the difference somewhere rather
/// than change the total. The delta lands on the installments *after* the edited
/// one first — those are the ones still ahead of the client — and only falls
/// back to the earlier unsettled ones when there is nothing later to absorb it,
/// which is what makes the final installment editable at all.
///
/// Fully-paid installments are never absorbers: their amount is settled history.
///
/// Returns `None` when neither absorber set can take the change; the caller
/// turns that into `NO_REBALANCE_ROOM`. Mirrors `rebalanceAmounts` in
/// `src/lib/finance.ts`, and is covered by the shared parity fixture.
///
/// No Rust command calls it any more: a schedule edit now arrives from
/// `update_purchase` as a whole schedule whose sum is checked outright, so
/// there is no single-row delta left to absorb. It stays because it is one half
/// of a cross-language pair — `finance.ts` still runs this exact algorithm to
/// redistribute the purchase editor's rows as they are typed, and
/// `tests/fixtures/finance-parity.json` is what proves the two agree. Deleting
/// it would leave that fixture checking the TS side against nothing.
#[allow(dead_code)]
pub fn rebalance_amounts(
    amounts: &[i64],
    paid_amounts: &[i64],
    index: usize,
    new_amount: i64,
) -> Option<Vec<i64>> {
    if index >= amounts.len() || new_amount < 0 || new_amount < paid_amounts[index] {
        return None;
    }

    let delta = new_amount - amounts[index];
    let mut base = amounts.to_vec();
    base[index] = new_amount;
    if delta == 0 {
        return Some(base);
    }

    let all: Vec<usize> = (0..amounts.len())
        .filter(|&i| i != index && paid_amounts[i] < amounts[i])
        .collect();
    let later: Vec<usize> = all.iter().copied().filter(|&i| i > index).collect();

    let sum_of = |set: &[usize]| -> i64 { set.iter().map(|&i| amounts[i]).sum() };
    apply_pool(&base, paid_amounts, &later, sum_of(&later) - delta)
        .or_else(|| apply_pool(&base, paid_amounts, &all, sum_of(&all) - delta))
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

        let cases = fixture["rebalanceAmounts"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let nums = |key: &str| -> Vec<i64> {
                case[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap())
                    .collect()
            };
            let amounts = nums("amounts");
            let paid_amounts = nums("paidAmounts");
            let index = case["index"].as_u64().unwrap() as usize;
            let new_amount = case["newAmount"].as_i64().unwrap();
            let expected: Option<Vec<i64>> = if case["expected"].is_null() {
                None
            } else {
                Some(nums("expected"))
            };
            let got = rebalance_amounts(&amounts, &paid_amounts, index, new_amount);
            assert_eq!(
                got, expected,
                "rebalance_amounts({amounts:?}, {paid_amounts:?}, {index}, {new_amount}) \
                 diverges from finance.ts"
            );
            // The whole point of rebalancing: the purchase total never moves,
            // and no row ends up owing less than it has already collected.
            if let Some(next) = got {
                assert_eq!(next.iter().sum::<i64>(), amounts.iter().sum::<i64>());
                assert!(next.iter().zip(&paid_amounts).all(|(a, p)| a >= p));
            }
        }
    }

    /// The rebalance prefers the installments *after* the edited one, because
    /// those are the ones still ahead of the client.
    #[test]
    fn rebalance_spends_the_later_installments_first() {
        let amounts = [200, 200, 200, 200, 200];
        let paid = [0; 5];
        assert_eq!(
            rebalance_amounts(&amounts, &paid, 2, 350),
            Some(vec![200, 200, 350, 125, 125])
        );
    }

    /// Editing the *last* installment has nothing after it, so the earlier
    /// unsettled ones absorb backwards. Without this fallback the final
    /// installment — the one carrying the rounding remainder, and the one a
    /// shopkeeper most often renegotiates — could never be edited at all.
    #[test]
    fn rebalance_falls_back_to_the_earlier_installments() {
        let amounts = [200, 200, 200, 200, 200];
        let paid = [200, 200, 0, 0, 0];
        assert_eq!(
            rebalance_amounts(&amounts, &paid, 4, 100),
            Some(vec![200, 200, 250, 250, 100])
        );
    }

    /// A settled installment is history: it is never asked to give anything up,
    /// which is also what keeps `paid_amount <= amount` true for it.
    #[test]
    fn rebalance_never_touches_a_settled_installment() {
        let amounts = [200, 200, 200];
        let paid = [0, 0, 200];
        let next = rebalance_amounts(&amounts, &paid, 0, 100).unwrap();
        assert_eq!(next, vec![100, 300, 200]);
    }

    /// With every other installment settled there is nowhere for the delta to
    /// go, and the total is not allowed to move — so the edit is refused.
    #[test]
    fn rebalance_refuses_when_nothing_can_absorb() {
        assert_eq!(rebalance_amounts(&[200, 200], &[0, 200], 0, 100), None);
        // Raising past what the others can give up is refused too.
        assert_eq!(
            rebalance_amounts(&[200, 200, 200], &[0, 50, 0], 0, 600),
            None
        );
        // As is dropping below what this row has already collected.
        assert_eq!(rebalance_amounts(&[200, 200], &[150, 0], 0, 100), None);
    }

    /// A no-op edit still resolves, so a caller that resubmits an unchanged
    /// amount is not refused for it.
    #[test]
    fn rebalance_accepts_an_unchanged_amount() {
        assert_eq!(
            rebalance_amounts(&[200, 300], &[0, 0], 1, 300),
            Some(vec![200, 300])
        );
        // Even when nothing else could have absorbed a real change.
        assert_eq!(
            rebalance_amounts(&[200, 200], &[0, 200], 0, 200),
            Some(vec![200, 200])
        );
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

        // Replaying the ladder must not have added `archived_at` a second time.
        // `ALTER TABLE ADD COLUMN` has no `IF NOT EXISTS`, so an unguarded m0002
        // fails the whole `Db::open` above with "duplicate column name".
        for table in ["client", "purchase"] {
            let archived_cols: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = 'archived_at'",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(archived_cols, 1, "{table} archive step must be replay-safe");
        }

        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// A database written before the archive feature must come forward with
    /// every existing client active, not archived.
    #[test]
    fn m0002_defaults_existing_clients_to_active() {
        let path = temp_db_path("migrate_archive");
        {
            let conn = Connection::open(&path).unwrap();
            m0001_initial_schema(&conn).unwrap();
            conn.execute(
                "INSERT INTO client (first_name, last_name) VALUES ('Pre', 'Archive')",
                [],
            )
            .unwrap();
            // v1 exactly: tables present, nothing stamped.
            conn.execute_batch("PRAGMA user_version = 1").unwrap();
        }

        let db = Db::open(&path).unwrap();
        let conn = db.lock();
        let archived: Option<String> = conn
            .query_row(
                "SELECT archived_at FROM client WHERE last_name = 'Archive'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            archived.is_none(),
            "existing clients must migrate as active"
        );

        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// The real upgrade path for the purchase archive: a v2 database (clients
    /// already archivable, purchases not) must come forward with every purchase
    /// live, or archiving would appear to have happened retroactively.
    #[test]
    fn m0003_defaults_existing_purchases_to_active() {
        let path = temp_db_path("migrate_purchase_archive");
        {
            let conn = Connection::open(&path).unwrap();
            m0001_initial_schema(&conn).unwrap();
            m0002_client_archive(&conn).unwrap();
            conn.execute(
                "INSERT INTO client (first_name, last_name) VALUES ('Pre', 'V3')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO purchase (reference, client_id, product_label, total_price,
                     installment_count, purchase_date)
                 VALUES ('A-000001', 1, 'Machine', 1000, 4, '2024-01-15')",
                [],
            )
            .unwrap();
            // v2 exactly: both tables present, the ladder stamped two steps in.
            conn.execute_batch("PRAGMA user_version = 2").unwrap();
        }

        let db = Db::open(&path).unwrap();
        let conn = db.lock();
        let archived: Option<String> = conn
            .query_row("SELECT archived_at FROM purchase WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            archived.is_none(),
            "existing purchases must migrate as live"
        );

        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// `pending_migration` decides whether a launch has anything to protect.
    ///
    /// Every `None` here is load-bearing: the caller blocks startup when the
    /// snapshot it asks for fails, so a false `Some` would turn a full disk into
    /// an app that refuses to open on a database with nothing in it.
    #[test]
    fn pending_migration_reports_only_a_database_worth_protecting() {
        let path = temp_db_path("pending");

        // A path with no file behind it — the fresh-install case. This must not
        // create the file, or the next launch would see one and disagree.
        assert_eq!(pending_migration(&path).unwrap(), None);
        assert!(!path.exists(), "the probe must not create the database");

        // A file that exists but carries no schema.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA user_version = 0").unwrap();
        }
        assert_eq!(
            pending_migration(&path).unwrap(),
            None,
            "a file with no client table has nothing to lose"
        );

        // Fully migrated: nothing pending.
        {
            let db = Db::open(&path).unwrap();
            let conn = db.lock();
            conn.execute(
                "INSERT INTO client (first_name, last_name) VALUES ('Pending', 'Sentinel')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO setting (key, value) VALUES ('language', 'ar')
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            )
            .unwrap();
        }
        assert_eq!(pending_migration(&path).unwrap(), None);

        // A pre-versioning database: tables and rows present, version reset.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA user_version = 0").unwrap();
        }
        let pending = pending_migration(&path).unwrap().expect("must be pending");
        assert_eq!(pending.target, MIGRATIONS.len());
        assert_eq!(
            pending.language.as_deref(),
            Some("ar"),
            "the dialog has to speak the language the shop configured"
        );

        // Ahead of this build: still None. Refusing that is `migrate`'s job, and
        // answering `Some` here would snapshot before handing over to a path
        // that cannot proceed anyway.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&format!("PRAGMA user_version = {}", MIGRATIONS.len() + 1))
                .unwrap();
        }
        assert_eq!(pending_migration(&path).unwrap(), None);

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
