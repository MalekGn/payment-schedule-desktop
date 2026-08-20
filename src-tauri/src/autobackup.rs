//! Automatic database snapshots, and the pool they are restored from.
//!
//! Backup in this app is otherwise entirely manual — one button in Settings —
//! which leaves the two moments that actually destroy data unprotected: a schema
//! migration running against the user's only copy, and an ordinary mistaken
//! delete (a purchase cascades through its installments and payments and cannot
//! be undone).
//!
//! Both snapshots go through [`crate::commands::backup_database_impl`]
//! unchanged, so they inherit its `.db` guard, its refusal to clobber a
//! non-SQLite file, its staged write and — the part that matters here — its
//! read-only `integrity_check`/`foreign_key_check` verification. An automatic
//! backup nobody watches is exactly the kind that must not be trusted blindly.
//!
//! **These snapshots are not a substitute for the manual one.** They live in
//! `backups/`, beside `payment_schedule.db` on the same disk, so a failed drive
//! or a stolen machine takes both. That is why they are recorded under their own
//! setting key and deliberately do not clear the Settings staleness nudge, which
//! asks for a copy that leaves the machine.
//!
//! This module also owns the *inbound* direction: the prefixes below are the
//! only naming scheme that knows which files in `backups/` are ours, so listing
//! them for the restore picker ([`list_snapshots`]) and taking the safety copy a
//! restore makes first ([`snapshot_before_restore`]) both belong here rather
//! than in `commands.rs`.

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime};
use rusqlite::Connection;

use crate::commands::{backup_database_impl, put_setting};
use crate::db::{today, Db, DbResult};
use crate::models::{BackupEntry, BackupKind, Settings};

/// `setting` key holding the ISO date of the last automatic snapshot. Separate
/// from `last_backup_at` on purpose — see the module docs.
pub(crate) const LAST_AUTO_BACKUP_KEY: &str = "last_auto_backup_at";

/// Filename prefix for the routine daily snapshots.
const AUTO_PREFIX: &str = "auto-";
/// Filename prefix for the snapshots taken before a schema migration.
const PRE_PREFIX: &str = "payment_schedule.pre-v";
/// Filename prefix for the snapshot taken before a restore overwrites the
/// database the user is working in.
const PRE_RESTORE_PREFIX: &str = "pre-restore-";

/// How many routine snapshots to keep. Five gives roughly a working week of
/// history at one launch a day, for a few MB a copy.
const KEEP_AUTO: usize = 5;
/// How many pre-migration snapshots to keep. Fewer, but pruned separately:
/// they are rarer and worth more, and a run of daily snapshots must never be
/// able to evict the copy taken before a schema change.
const KEEP_PRE: usize = 2;
/// How many pre-restore snapshots to keep. Its own pool for the same reason as
/// [`KEEP_PRE`], and small on purpose: this is the undo for "I restored the
/// wrong file", which the user either reaches for immediately or not at all.
const KEEP_PRE_RESTORE: usize = 2;

/// The directory holding every automatic snapshot, created if absent.
pub fn backups_dir(data_dir: &Path) -> DbResult<PathBuf> {
    let dir = data_dir.join("backups");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Snapshot the database before the schema ladder advances to `target`.
///
/// The caller treats a failure here as fatal — see `lib.rs`. That is the whole
/// point: the alternative is migrating the user's only copy with no fallback,
/// and a full disk is something they can fix while a bad migration is not.
///
/// Overwriting an existing `pre-v{target}.db` is intended. It only happens when
/// a previous launch snapshotted and then failed to migrate, in which case the
/// two files carry the same pre-migration state.
pub fn snapshot_before_migration(db_path: &Path, dir: &Path, target: usize) -> DbResult<PathBuf> {
    // Opens its own connection rather than taking one: this runs before
    // `Db::open`, precisely so the ladder has not advanced yet, and keeping the
    // rusqlite handle in here means `lib.rs` never touches the database
    // directly.
    let conn = Connection::open(db_path)?;
    let dest = dir.join(format!("{PRE_PREFIX}{target}.db"));
    backup_database_impl(&conn, &dest, dir)?;
    log::info!("took a pre-migration snapshot for schema version {target}");
    prune(dir, PRE_PREFIX, KEEP_PRE);
    Ok(dest)
}

/// Snapshot the database before a restore overwrites it.
///
/// The fallback for the fallback. Restoring is the one action in the app that
/// discards *everything* the user has, and it is routinely reached for in a
/// hurry — from the wrong file, or from a snapshot older than the one they
/// meant. Its caller treats a failure here as fatal to the restore, on the same
/// reasoning as [`snapshot_before_migration`]: never take the irreversible step
/// against the only copy with nothing to fall back to.
///
/// Opens its own connection because it runs inside [`crate::db::Db::replace_file`],
/// with the shared one already closed so the file can be swapped.
///
/// The `{ISO date}-{nanos}` suffix keeps two restores on one day from colliding
/// while staying sortable as a plain string, which is what [`prune`] relies on.
pub fn snapshot_before_restore(db_path: &Path, dir: &Path) -> DbResult<PathBuf> {
    let conn = Connection::open(db_path)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dest = dir.join(format!("{PRE_RESTORE_PREFIX}{}-{stamp}.db", today()));
    backup_database_impl(&conn, &dest, dir)?;
    drop(conn);

    log::info!("took a pre-restore snapshot");
    prune(dir, PRE_RESTORE_PREFIX, KEEP_PRE_RESTORE);
    Ok(dest)
}

/// Every snapshot in `dir` this app wrote, newest first.
///
/// Scoped to the three prefixes above, so a file the user dropped into
/// `backups/` is not offered as if the app had verified it — a restore from an
/// unknown file is the picker's job, and it validates what it is handed.
///
/// Unreadable entries are skipped rather than failed on: a restore list that
/// refuses to render because one file has odd permissions is worse than a
/// restore list missing that file.
pub fn list_snapshots(dir: &Path) -> Vec<BackupEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("could not list the backups directory: {e}");
            return Vec::new();
        }
    };

    let mut found: Vec<BackupEntry> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            if !name.ends_with(".db") {
                return None;
            }
            let kind = classify(&name)?;
            let size_bytes = e.metadata().ok().map(|m| m.len()).unwrap_or(0);
            Some(BackupEntry {
                taken_at: taken_at(&name, &path),
                path: path.to_string_lossy().into_owned(),
                file_name: name,
                kind,
                size_bytes,
            })
        })
        .collect();

    // By date first — what the user is choosing on — then by name, which
    // disambiguates two snapshots of the same day deterministically.
    found.sort_by(|a, b| {
        b.taken_at
            .cmp(&a.taken_at)
            .then_with(|| b.file_name.cmp(&a.file_name))
    });
    found
}

/// Which pool a filename belongs to, or `None` if it is not one of ours.
fn classify(name: &str) -> Option<BackupKind> {
    if name.starts_with(AUTO_PREFIX) {
        Some(BackupKind::Auto)
    } else if name.starts_with(PRE_PREFIX) {
        Some(BackupKind::PreMigration)
    } else if name.starts_with(PRE_RESTORE_PREFIX) {
        Some(BackupKind::PreRestore)
    } else {
        None
    }
}

/// The date a snapshot was taken, as an ISO string.
///
/// Read out of the filename where the naming scheme carries one, because that
/// is the date the snapshot is *of*; an mtime can be a copy or a restore of the
/// backups directory itself. Pre-migration snapshots carry a schema version
/// instead, so they fall back to the mtime, and so does anything unparseable.
fn taken_at(name: &str, path: &Path) -> String {
    for prefix in [AUTO_PREFIX, PRE_RESTORE_PREFIX] {
        if let Some(rest) = name.strip_prefix(prefix) {
            if rest.len() >= 10 && NaiveDate::parse_from_str(&rest[..10], "%Y-%m-%d").is_ok() {
                return rest[..10].to_string();
            }
        }
    }

    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| {
            chrono::DateTime::<Local>::from(t)
                .date_naive()
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

/// How often the routine backup runs, as an interval in days.
///
/// Deliberately an interval and not a calendar rule ("every Monday", "the 1st").
/// A calendar rule on a desktop app can be configured into never firing — a 31st
/// that most months never reach, a Monday the shop is closed — and it would need
/// localized weekday names in three languages to express. An interval cannot be
/// set to a value that never comes round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
}

impl Frequency {
    /// Parse the stored setting, falling back to daily. The value comes from a
    /// closed set validated on write, so an unknown one means a hand-edited
    /// database — back off to the safest cadence rather than stop backing up.
    fn parse(value: &str) -> Self {
        match value {
            "weekly" => Frequency::Weekly,
            "monthly" => Frequency::Monthly,
            _ => Frequency::Daily,
        }
    }

    fn interval_days(self) -> i64 {
        match self {
            Frequency::Daily => 1,
            Frequency::Weekly => 7,
            Frequency::Monthly => 30,
        }
    }
}

/// The shop's backup schedule, as the scheduler needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub enabled: bool,
    pub frequency: Frequency,
    pub time: NaiveTime,
}

impl Schedule {
    /// Read the schedule out of the settings the rest of the app already loads.
    pub fn from_settings(settings: &Settings) -> Self {
        Schedule {
            enabled: settings.auto_backup_enabled,
            frequency: Frequency::parse(&settings.auto_backup_frequency),
            time: parse_time(&settings.auto_backup_time).unwrap_or_else(|| {
                NaiveTime::from_hms_opt(17, 0, 0).expect("17:00 is a valid time")
            }),
        }
    }
}

/// Parse an `HH:MM` setting. `None` for anything else.
pub(crate) fn parse_time(value: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M").ok()
}

/// Normalise an `HH:MM` setting for storage, or `None` if it is not one.
///
/// Canonical on the way in so the scheduler never re-decides what "5 pm" means
/// and `<input type="time">` always round-trips what it is given.
pub(crate) fn canonical_time(value: &str) -> Option<String> {
    parse_time(value).map(|t| t.format("%H:%M").to_string())
}

/// Whether a routine backup is owed right now.
///
/// The one predicate behind both the launch check and every scheduler tick, so
/// there is no second rule that can disagree with this one. Two branches,
/// because a time of day alone does not survive a desktop app:
///
/// - **Overdue** — more than the interval has passed, so take one at the first
///   opportunity whatever the clock says. This is the branch that saves the shop
///   that only ever runs the app between 09:00 and 16:00: without it, 17:00
///   never arrives while the app is open and they are *never* backed up.
/// - **Today's window** — the interval is up and the scheduled time has passed.
///   The headline case: 17:00 comes round with the app open.
///
/// A missing `last` is the first run ever, and is owed one immediately.
pub fn due(now: NaiveDateTime, schedule: &Schedule, last: Option<&str>) -> bool {
    if !schedule.enabled {
        return false;
    }

    let Some(last) = last.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) else {
        // Never backed up, or a date this build cannot read: either way, owed.
        return true;
    };

    let elapsed = (now.date() - last).num_days();
    let interval = schedule.frequency.interval_days();

    elapsed > interval || (elapsed >= interval && now.time() >= schedule.time)
}

/// What a pass of [`snapshot_if_due`] did, so the scheduler can back off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The schedule says nothing is owed yet.
    NotDue,
    /// A snapshot was written.
    Taken,
    /// One was owed and could not be written. Already logged.
    Failed,
}

/// Take the routine snapshot if the schedule says one is owed.
///
/// Swallows every failure after logging it, and reports it through [`Outcome`]
/// rather than an error. A routine safety net must never stop a shop working:
/// unlike the pre-migration snapshot there is no irreversible step waiting
/// behind this one, so failing loudly would cost the user their afternoon and
/// protect nothing.
pub fn snapshot_if_due(db: &Db, db_path: &Path, dir: &Path) -> Outcome {
    // A restore is swapping the file out from under this path right now. The
    // shared mutex would not stop us — this function deliberately opens its own
    // connection so a scheduled backup does not block the shop typing — so the
    // flag is what serializes the two. Reported as `NotDue` rather than
    // `Failed`: nothing went wrong, and a backoff would be the wrong response to
    // an operation that takes seconds.
    if db.is_restoring() {
        return Outcome::NotDue;
    }

    let (schedule, last) = {
        let conn = db.lock();
        let settings = crate::commands::read_settings(&conn);
        (
            Schedule::from_settings(&settings),
            settings.last_auto_backup_at,
        )
    };

    if !due(Local::now().naive_local(), &schedule, last.as_deref()) {
        return Outcome::NotDue;
    }

    let today = today().to_string();
    let dest = dir.join(format!("{AUTO_PREFIX}{today}.db"));

    // Its own connection, rather than the shared `Mutex<Connection>` every
    // command queues behind. At launch the mutex was free, but a scheduled
    // backup lands while the shop is typing — in WAL mode a second connection
    // reads a consistent snapshot without blocking writers, so `VACUUM INTO`
    // costs nobody their keystrokes.
    let conn = match Connection::open(db_path) {
        Ok(conn) => conn,
        Err(e) => {
            log::warn!("the automatic backup could not open the database: {e}");
            return Outcome::Failed;
        }
    };
    if let Err(e) = backup_database_impl(&conn, &dest, dir) {
        log::warn!("the automatic backup could not be written: {e}");
        return Outcome::Failed;
    }
    drop(conn);

    log::info!("automatic backup written");
    prune(dir, AUTO_PREFIX, KEEP_AUTO);

    // Recorded after the snapshot is on disk, never before: a date written for a
    // backup that failed would suppress every retry for the rest of the day.
    if let Err(e) = put_setting(&db.lock(), LAST_AUTO_BACKUP_KEY, &today) {
        log::warn!("automatic backup succeeded but its date could not be recorded: {e}");
    }

    Outcome::Taken
}

/// How often the scheduler wakes to ask whether a backup is owed.
///
/// A poll, not a computed sleep until the scheduled time. One minute is cheap,
/// and it is the only shape that survives the three things that routinely
/// invalidate a long sleep: the user changing the time in Settings, the machine
/// suspending and waking hours later, and the system clock moving.
const TICK: std::time::Duration = std::time::Duration::from_secs(60);

/// How long to leave a failed backup alone before trying again.
///
/// Without this, a full disk means an owed backup stays owed, and the scheduler
/// retries `VACUUM INTO` **every minute for as long as the app is open** —
/// megabytes of write attempts and a warning line a minute, none of which can
/// succeed until the user frees space. Ticking continues (it is nearly free, and
/// it is what keeps a settings change responsive); only the attempt is held off.
const FAILURE_BACKOFF: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Start the background scheduler. Returns immediately.
///
/// A plain `std::thread`, not a `tauri::async_runtime` task: the work is
/// blocking (`VACUUM INTO` plus two verification PRAGMAs), and keeping it off
/// the async executor is exactly the fix `AUDIT_REPORT.md:172` (I3) asks for on
/// the manual path. Detached — process exit ends it, and there is nothing worth
/// joining on the way out.
pub fn start_scheduler(app: tauri::AppHandle, db_path: PathBuf, dir: PathBuf) {
    use tauri::Manager;

    std::thread::spawn(move || {
        log::info!("automatic backup scheduler started");
        let mut retry_after: Option<std::time::Instant> = None;

        loop {
            let held_off = retry_after.is_some_and(|t| std::time::Instant::now() < t);
            if !held_off {
                // Settings are re-read on every pass, so a change to the time or
                // the frequency takes effect within a tick with no restart and
                // nothing to signal. The state is fetched per tick rather than
                // held, so the thread borrows the database only while it is
                // actually working.
                retry_after = match snapshot_if_due(&app.state::<Db>(), &db_path, &dir) {
                    Outcome::Failed => {
                        log::warn!("holding off the automatic backup for an hour after a failure");
                        Some(std::time::Instant::now() + FAILURE_BACKOFF)
                    }
                    Outcome::Taken | Outcome::NotDue => None,
                };
            }
            std::thread::sleep(TICK);
        }
    });
}

/// Keep the newest `keep` files starting with `prefix`, remove the rest.
///
/// Both naming schemes — `auto-{ISO date}` and `pre-v{n}` for single-digit
/// versions — sort newest-last as plain strings, so a descending sort is a
/// recency order without touching the filesystem for mtimes.
///
/// Scoped to its own prefix inside `backups/` and never recursive: a file a user
/// dropped in there, and the *other* pool's snapshots, are not ours to delete.
fn prune(dir: &Path, prefix: &str, keep: usize) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("could not list the backups directory to prune it: {e}");
            return;
        }
    };

    let mut ours: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".db"))
        })
        .collect();

    if ours.len() <= keep {
        return;
    }

    ours.sort();
    for stale in ours.iter().rev().skip(keep) {
        match std::fs::remove_file(stale) {
            Ok(()) => log::info!("pruned an old automatic backup"),
            Err(e) => log::warn!("could not prune an old automatic backup: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::get_setting;
    use crate::db::Db;

    /// A schedule due at 17:00 every day — the shipped default.
    fn daily_at_five() -> Schedule {
        Schedule {
            enabled: true,
            frequency: Frequency::Daily,
            time: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        }
    }

    fn at(datetime: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(datetime, "%Y-%m-%d %H:%M").unwrap()
    }

    /// A scratch directory that cleans itself up.
    fn scratch(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ps_autobackup_{tag}_{}_{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seeded_db(dir: &Path) -> Db {
        let db = Db::open(&dir.join("payment_schedule.db")).unwrap();
        {
            let conn = db.lock();
            conn.execute(
                "INSERT INTO client (first_name, last_name) VALUES ('Auto', 'Sentinel')",
                [],
            )
            .unwrap();
        }
        db
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"SQLite format 3\0placeholder").unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut n: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        n.sort();
        n
    }

    /// The headline case: the app is open when the scheduled time comes round.
    #[test]
    fn the_daily_schedule_fires_at_its_time_and_not_before() {
        let s = daily_at_five();

        assert!(
            !due(at("2026-08-07 16:59"), &s, Some("2026-08-06")),
            "a minute early is not due"
        );
        assert!(due(at("2026-08-07 17:00"), &s, Some("2026-08-06")));
        assert!(due(at("2026-08-07 21:30"), &s, Some("2026-08-06")));

        // Already taken today: nothing more, whatever the clock says.
        assert!(!due(at("2026-08-07 17:00"), &s, Some("2026-08-07")));
        assert!(!due(at("2026-08-07 23:59"), &s, Some("2026-08-07")));
    }

    /// The branch the whole two-part rule exists for.
    ///
    /// A shop that opens the app at 09:00 and closes it at 16:00 never has it
    /// running at 17:00. Under a plain "fire at the scheduled time" rule they
    /// would never be backed up at all — the failure would be silent, and would
    /// only surface the day they needed the backup.
    #[test]
    fn a_shop_that_is_never_open_at_the_scheduled_time_is_still_backed_up() {
        let s = daily_at_five();

        // Yesterday's backup, and it is 09:00 — one day elapsed, time not yet
        // reached, so not due *yet*.
        assert!(!due(at("2026-08-07 09:00"), &s, Some("2026-08-06")));

        // They closed at 16:00 and came back the next morning. Two days elapsed
        // now, so the overdue branch fires regardless of the hour.
        assert!(due(at("2026-08-08 09:00"), &s, Some("2026-08-06")));
    }

    /// Starting the app after the window has already passed must catch up
    /// rather than wait for tomorrow.
    #[test]
    fn a_launch_after_the_window_catches_up_immediately() {
        let s = daily_at_five();
        assert!(due(at("2026-08-07 19:30"), &s, Some("2026-08-06")));
    }

    #[test]
    fn the_longer_frequencies_wait_out_their_interval() {
        let weekly = Schedule {
            frequency: Frequency::Weekly,
            ..daily_at_five()
        };
        assert!(!due(at("2026-08-07 17:00"), &weekly, Some("2026-08-04")));
        assert!(!due(at("2026-08-10 23:00"), &weekly, Some("2026-08-04")));
        assert!(due(at("2026-08-11 17:00"), &weekly, Some("2026-08-04")));

        let monthly = Schedule {
            frequency: Frequency::Monthly,
            ..daily_at_five()
        };
        assert!(!due(at("2026-08-07 17:00"), &monthly, Some("2026-07-20")));
        assert!(due(at("2026-08-19 17:00"), &monthly, Some("2026-07-20")));
    }

    #[test]
    fn a_first_run_is_owed_a_backup_immediately() {
        let s = daily_at_five();
        // No date recorded at all, and an unparseable one, both read as "never".
        assert!(due(at("2026-08-07 08:00"), &s, None));
        assert!(due(at("2026-08-07 08:00"), &s, Some("not a date")));
    }

    #[test]
    fn a_disabled_schedule_never_fires() {
        let off = Schedule {
            enabled: false,
            ..daily_at_five()
        };
        assert!(!due(at("2026-08-07 17:00"), &off, Some("2026-08-06")));
        // Not even when it is long overdue — off means off.
        assert!(!due(at("2026-09-30 17:00"), &off, None));
    }

    /// The stored value reaches `<input type="time">` and the scheduler alike,
    /// so it is normalised on the way in and anything unparseable is refused
    /// rather than silently treated as midnight.
    #[test]
    fn the_scheduled_time_is_canonical_or_rejected() {
        assert_eq!(canonical_time("17:00").as_deref(), Some("17:00"));
        assert_eq!(canonical_time("7:05").as_deref(), Some("07:05"));
        assert_eq!(canonical_time(" 09:30 ").as_deref(), Some("09:30"));

        // Single-digit minutes too — `%H:%M` is lenient about width on both
        // sides, and `src/api/mock.ts` has to be lenient in exactly the same
        // places or the browser build refuses times the desktop build accepts.
        assert_eq!(canonical_time("17:6").as_deref(), Some("17:06"));
        assert_eq!(canonical_time("7:5").as_deref(), Some("07:05"));

        for bad in [
            "25:00", "17:60", "17", "", "5pm", "17:00:00", "-1:00", "17:",
        ] {
            assert_eq!(canonical_time(bad), None, "{bad} must be refused");
        }
    }

    /// A hand-edited database must not be able to stop the backups by carrying
    /// a frequency this build does not know.
    #[test]
    fn an_unknown_frequency_falls_back_to_daily() {
        assert_eq!(Frequency::parse("weekly"), Frequency::Weekly);
        assert_eq!(Frequency::parse("monthly"), Frequency::Monthly);
        assert_eq!(Frequency::parse("daily"), Frequency::Daily);
        assert_eq!(Frequency::parse("hourly"), Frequency::Daily);
        assert_eq!(Frequency::parse(""), Frequency::Daily);
    }

    #[test]
    fn the_pre_migration_snapshot_carries_the_data_and_leaves_the_source_alone() {
        let home = scratch("pre");
        let db = seeded_db(&home);
        let dir = backups_dir(&home).unwrap();

        // Dropped first: the snapshot opens the file itself, as it does at
        // startup, where nothing else holds it either.
        drop(db);
        let dest = snapshot_before_migration(&home.join("payment_schedule.db"), &dir, 4).unwrap();

        // Assert on the sentinel rather than a row count: a debug build seeds
        // six demo clients on a fresh database, so a count pins the fixture
        // instead of the snapshot.
        let restored = Connection::open(&dest).unwrap();
        let sentinel: i64 = restored
            .query_row(
                "SELECT COUNT(*) FROM client WHERE last_name = 'Sentinel'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sentinel, 1, "the snapshot must carry the rows");
        drop(restored);

        // Staging happens in the same directory, so an orphan there would be
        // indistinguishable from a snapshot to the pruner.
        assert_eq!(names(&dir), vec!["payment_schedule.pre-v4.db".to_string()]);

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_daily_snapshot_runs_once_and_records_its_date() {
        let home = scratch("due");
        let db = seeded_db(&home);
        let dir = backups_dir(&home).unwrap();
        let today = today().to_string();

        let db_path = home.join("payment_schedule.db");
        snapshot_if_due(&db, &db_path, &dir);
        assert_eq!(names(&dir), vec![format!("auto-{today}.db")]);
        assert_eq!(
            get_setting(&db.lock(), LAST_AUTO_BACKUP_KEY, ""),
            today,
            "the date must be recorded or every launch re-snapshots"
        );

        // A second launch the same day is a no-op — proven by the mtime, since
        // the filename alone would look identical either way.
        let before = std::fs::metadata(dir.join(format!("auto-{today}.db")))
            .unwrap()
            .modified()
            .unwrap();
        snapshot_if_due(&db, &db_path, &dir);
        let after = std::fs::metadata(dir.join(format!("auto-{today}.db")))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(before, after, "the same day must not snapshot twice");
        assert_eq!(
            snapshot_if_due(&db, &db_path, &dir),
            Outcome::NotDue,
            "a pass that does nothing must say so, so the backoff is not armed"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_failed_daily_snapshot_never_reaches_the_caller() {
        let home = scratch("failed");
        let db = seeded_db(&home);

        // A directory that does not exist: `VACUUM INTO` cannot stage there.
        let missing = home.join("nowhere");
        snapshot_if_due(&db, &home.join("payment_schedule.db"), &missing);

        assert_eq!(
            get_setting(&db.lock(), LAST_AUTO_BACKUP_KEY, ""),
            "",
            "a failed snapshot must not record a date, or it suppresses retries"
        );
        // Reported rather than silent, because the scheduler holds off for an
        // hour on a failure. Without this signal it would retry `VACUUM INTO`
        // every minute against a disk that cannot take it.
        assert_eq!(
            snapshot_if_due(&db, &home.join("payment_schedule.db"), &missing),
            Outcome::Failed
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn pruning_keeps_the_newest_and_touches_nothing_else() {
        let dir = scratch("prune");

        for day in 1..=8 {
            touch(&dir, &format!("auto-2026-08-0{day}.db"));
        }
        // Neither of these belongs to the `auto` pool.
        touch(&dir, "payment_schedule.pre-v4.db");
        std::fs::write(dir.join("notes.txt"), b"someone else's file").unwrap();

        prune(&dir, AUTO_PREFIX, KEEP_AUTO);

        assert_eq!(
            names(&dir),
            vec![
                "auto-2026-08-04.db".to_string(),
                "auto-2026-08-05.db".to_string(),
                "auto-2026-08-06.db".to_string(),
                "auto-2026-08-07.db".to_string(),
                "auto-2026-08-08.db".to_string(),
                "notes.txt".to_string(),
                "payment_schedule.pre-v4.db".to_string(),
            ],
            "the newest five survive; the other pool and a stranger's file are untouched"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- restore side --------------------------------------------------------

    /// The pre-restore pool is pruned on its own, and cannot evict either of the
    /// pools it sits beside. Losing the copy taken before a schema change to a
    /// run of restores would be the same failure `KEEP_PRE` exists to prevent.
    #[test]
    fn the_pre_restore_pool_prunes_without_touching_the_others() {
        let dir = scratch("pre_restore_prune");
        let db = seeded_db(&dir);
        let db_path = dir.join("payment_schedule.db");
        let backups = backups_dir(&dir).unwrap();

        touch(&backups, "auto-2026-08-01.db");
        touch(&backups, "payment_schedule.pre-v3.db");
        drop(db);

        // One more than the cap, so pruning has to choose.
        for _ in 0..(KEEP_PRE_RESTORE + 1) {
            snapshot_before_restore(&db_path, &backups).unwrap();
        }

        let kept = names(&backups);
        let pre_restores = kept
            .iter()
            .filter(|n| n.starts_with(PRE_RESTORE_PREFIX))
            .count();
        assert_eq!(pre_restores, KEEP_PRE_RESTORE);
        assert!(kept.iter().any(|n| n == "auto-2026-08-01.db"));
        assert!(kept.iter().any(|n| n == "payment_schedule.pre-v3.db"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The picker gets newest first, classified, and nothing that is not ours —
    /// a file a user dropped into `backups/` must not be offered as if the app
    /// had written and verified it.
    #[test]
    fn the_snapshot_listing_is_scoped_classified_and_newest_first() {
        let dir = scratch("list_snapshots");
        let backups = backups_dir(&dir).unwrap();

        touch(&backups, "auto-2026-08-01.db");
        touch(&backups, "auto-2026-08-19.db");
        touch(&backups, "pre-restore-2026-08-18-123456789.db");
        touch(&backups, "payment_schedule.pre-v4.db");
        touch(&backups, "holiday-photos.db");
        touch(&backups, "auto-2026-08-02.txt");

        let listed = list_snapshots(&backups);
        let names: Vec<&str> = listed.iter().map(|e| e.file_name.as_str()).collect();

        assert!(!names.contains(&"holiday-photos.db"), "not one of ours");
        assert!(!names.contains(&"auto-2026-08-02.txt"), "not a database");
        assert_eq!(names.len(), 4);

        // Newest first. The pre-migration snapshot carries a schema version and
        // not a date, so it dates from its mtime — today, hence first.
        assert_eq!(names[0], "payment_schedule.pre-v4.db");
        assert_eq!(names[1], "auto-2026-08-19.db");
        assert_eq!(names[2], "pre-restore-2026-08-18-123456789.db");
        assert_eq!(names[3], "auto-2026-08-01.db");

        assert_eq!(listed[1].kind, BackupKind::Auto);
        assert_eq!(listed[1].taken_at, "2026-08-19");
        assert_eq!(listed[2].kind, BackupKind::PreRestore);
        assert_eq!(listed[2].taken_at, "2026-08-18");
        assert_eq!(listed[0].kind, BackupKind::PreMigration);
        assert!(listed[0].size_bytes > 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The scheduler runs on its own connection, outside the mutex, so the flag
    /// is the only thing that keeps it from reading the file mid-swap.
    #[test]
    fn the_scheduler_stands_down_while_a_restore_is_swapping_the_file() {
        let dir = scratch("restore_standoff");
        let db = seeded_db(&dir);
        let db_path = dir.join("payment_schedule.db");
        let backups = backups_dir(&dir).unwrap();

        // A backup is owed — nothing has ever been taken — so `NotDue` here can
        // only come from the stand-down.
        // Read through the flag rather than calling `snapshot_if_due` inside
        // the closure: it would take `Db::lock`, which `replace_file` holds.
        // What the scheduler thread actually does is check the flag first.
        let mut observed = Outcome::NotDue;
        db.replace_file(&db_path, || {
            observed = if db.is_restoring() {
                Outcome::NotDue
            } else {
                Outcome::Failed
            };
            Ok(())
        })
        .unwrap();
        assert_eq!(observed, Outcome::NotDue, "the flag must be set mid-swap");
        assert!(!db.is_restoring(), "and cleared afterwards");
        assert!(
            names(&backups).is_empty(),
            "nothing may be written while the file is being swapped"
        );

        // …and it is owed again the moment the restore is over.
        assert_eq!(snapshot_if_due(&db, &db_path, &backups), Outcome::Taken);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
