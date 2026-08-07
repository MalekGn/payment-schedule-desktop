# Database Report

> **Audit date:** 2026-08-05 · **Revision audited:** `834040a` (branch `dev`, clean tree)

## Scope note — read this first

This repository is **not a server application**. It is an offline-first **Tauri 2
desktop app** whose entire database is a single local SQLite file,
`payment_schedule.db`, in the OS app-data directory
(`src-tauri/src/lib.rs:66-71`, `README.md:267-280`). There is **no managed
database provider, no server, no container, no deployment environment** — a full
sweep of the 184 tracked files (excluding the vendored `.claude/skills/` tree)
found no Dockerfile, `docker-compose*`, Terraform, CloudFormation, Ansible,
Kubernetes/Helm, systemd unit, crontab, Makefile, or shell script anywhere.

Consequently several sub-questions below (RDS snapshots, blue/green, traffic
cutover, staging DB) have no analogue here. Those are marked **N/A — no such
layer exists**, distinct from **❌ missing**, which means the practice _does_
apply to a desktop app and is genuinely absent.

The "production database" in this architecture is **the end user's own file on
their own machine**. That is what shifts the risk profile: there is no operator
who can restore it for them.

---

## 1. Automated Backups

### Is the database backed up automatically?

**No.** Backup exists as a feature, but it is **100% manual and user-initiated**.
There is no scheduler, no timer, no on-startup or on-shutdown hook, and no
retention logic anywhere in the tree.

### Evidence — what does exist

| Layer    | Location                                    | Detail                                                                                       |
| -------- | ------------------------------------------- | -------------------------------------------------------------------------------------------- |
| UI       | `src/views/SettingsView.vue:136-145`        | Settings → "Backup database" button opens a native save dialog; nothing calls it but a click |
| Gateway  | `src/api/index.ts:202-203`                  | `backupDatabase(dest)` → `invoke("backup_database", { dest })`                               |
| Command  | `src-tauri/src/commands.rs:2217-2234`       | `#[tauri::command] backup_database(db, lic, app, dest)`                                      |
| Core     | `src-tauri/src/commands.rs:2236-2313`       | `backup_database_impl` — `VACUUM INTO` at line 2282                                          |
| Registry | `src-tauri/src/lib.rs:121`                  | `commands::backup_database` in `generate_handler!`                                           |
| Docs     | `architecture.md:424-429`, `features.md:20` | "the only recovery path in the app"; "Desktop only"                                          |

The implementation itself is careful and well tested. `backup_database_impl`:

- rejects any destination not ending in `.db` (`commands.rs:2248-2251`);
- refuses to overwrite an existing file whose first 16 bytes are not
  `SQLite format 3\0`, so a bad path cannot clobber unrelated files
  (`commands.rs:2254-2266`);
- stages into app-data under a `backup-{pid}-{nanos}.part` name
  (`commands.rs:2276-2280`), then `rename`s atomically, falling back to
  `fs::copy` across filesystems (`commands.rs:2301-2309`);
- is covered by five Rust tests — `commands.rs:4874`, `:4910`, `:4944`, `:4970`,
  `:5007` (readable snapshot, clobber refusal, sibling safety, overwrite,
  cross-filesystem fallback).

### Evidence — verified absences

- **No scheduler.** The Tauri `setup` hook (`src-tauri/src/lib.rs:66-85`) only
  creates the app-data dir, opens the DB, evaluates the licence, and calls
  `app.manage(...)`. No `tokio::spawn`, `thread::spawn`, `setInterval`, or timer
  relates to backup anywhere in `src-tauri/src/` or `src/`.
- **No CI/cron backup.** `.github/workflows/` contains only `build.yml`,
  `ci.yml`, `codeql.yml`, `e2e.yml`, `security.yml`. The three cron schedules
  present — `e2e.yml:14` (`"0 2 * * *"`), `security.yml:28` (`"0 6 * * 1"`),
  `codeql.yml:12` (`"0 7 * * 1"`) — are nightly E2E and weekly security/CodeQL
  scans. **None references a database.**
- **No restore command.** There is no `restore_database`; `restore_client` /
  `restore_purchase` are soft-archive un-archiving, unrelated to files. The only
  documented recovery is "delete `payment_schedule.db` and restart"
  (`README.md:309-310`) — which discards data rather than recovering it.

### Frequency and exact schedule

**N/A — user-triggered, on demand.** No cron expression, interval, or trigger
condition exists to cite.

### Last known / next scheduled backup date

- **Next:** none — nothing schedules a backup.
- **Last known:** one real artifact exists on this machine —
  `src-tauri/payment-schedule-2026-08-05.db`, **2026-08-05 02:14 local**,
  57 344 bytes; `file(1)` reports `SQLite 3.x database, user version 3`. Its name
  matches the default filename template at `SettingsView.vue:140`
  (`payment-schedule-${todayIso()}.db`), so it is a genuine product of this
  feature. It is untracked and ignored by `.gitignore:45-52` (which explains why:
  "these hold real client PII").

  **Treat this as a developer-machine artifact, not evidence of a backup
  cadence.** No release has ever been cut (`git tag` is empty), so no end-user
  backup has ever been taken.

- Nothing in the app **records** that a backup happened. `lastBackupPath`
  (`src/api/mock.ts:166`) is in-memory state in the browser mock only; the Rust
  path writes nothing to the `setting` table. So even on a live install, "when
  did this user last back up?" is unanswerable.

### Retention policy

**Not found in repo.** No retention, pruning, rotation, `max_backups`, or
`keep_last` logic exists. Each backup is a standalone file at a path the user
picked; overwriting or deleting old ones is entirely manual.

### Additional risk worth flagging

Backup is **licence-gated**. `require_license(&lic)?` runs first
(`commands.rs:2224`) and the UI button is `:disabled="locked"`
(`SettingsView.vue:360`); `docs/license-format.md:179` lists `backup` among
licensed features. A shop whose licence has expired **cannot take a backup** —
including the backup they would most want before troubleshooting. The unlicensed
baseline deliberately keeps reading one's own ledger available
(`architecture.md:112-116`); backup arguably belongs in that same safety
baseline.

---

## 2. Backup Format

### Are backups stored as `.zip` (or any compressed/archive format)?

**No.** Not zip, not tar, not gzip — **no compression or archiving of any kind.**

### What format exactly

A **plain, uncompressed SQLite database file** produced by SQLite's native
`VACUUM INTO`:

```rust
// src-tauri/src/commands.rs:2282
let vacuum = conn.execute("VACUUM INTO ?1", [&staged.to_string_lossy().to_string()]);
```

The `.db` extension is enforced (`commands.rs:2248`), and the artifact on disk
verifies as a real SQLite file (header `SQLite format 3\0`, `user version 3`).

Verified absences: no `zip`, `flate2`, `tar`, `zstd`, `bzip2`, `xz2`, or
`brotli` in `src-tauri/Cargo.toml`, and none referenced in `src-tauri/src/`
(`flate2`/`brotli` appear in `Cargo.lock` only as transitive deps of
tauri/wry). No `jszip`, `archiver`, `pako`, or `fflate` in `package.json`.
`rusqlite`'s `backup` feature is not enabled either — `VACUUM INTO` is used
instead of the Online Backup API.

### Assessment: would zip be a good choice here?

**No. `VACUUM INTO` is the right call and should be kept.** Point by point:

| Criterion                  | `VACUUM INTO` (current)                                                                                                                                                                                          | Zip                                                                                                                      |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| **Native tooling**         | It **is** SQLite's own supported snapshot mechanism. Any SQLite tool opens the file directly.                                                                                                                    | Not a SQLite format; needs unzipping before anything can read it.                                                        |
| **Consistency**            | Transaction-consistent by construction. Critically, it is **WAL-correct** — a naive file copy would race in-flight writes and miss the `-wal` contents (`architecture.md:426-427`, `commands.rs:2213-2216`).     | Zipping the live `.db` reintroduces exactly that race unless you `VACUUM INTO` first anyway.                             |
| **Restore speed / steps**  | Zero-step. Copy the file into place, done. Matters enormously for a non-technical shop owner under stress.                                                                                                       | Adds an extract step and a failure mode (partial/corrupt archive) at the worst moment.                                   |
| **Compression ratio**      | None — but `VACUUM` **defragments**, reclaiming free pages. The observed snapshot is 57 KB. Realistic full-scale data (a few thousand installments of integers and short text) stays in the low single-digit MB. | Would compress well (~5-10×) — saving megabytes nobody is short of.                                                      |
| **Integrity verification** | `PRAGMA integrity_check` / `PRAGMA foreign_key_check` validate the _database_, not just the bytes.                                                                                                               | CRC32 per entry — detects bit rot, proves nothing about schema validity.                                                 |
| **Encryption**             | None today (see recommendation below).                                                                                                                                                                           | ZipCrypto is cryptographically broken; AES-zip is a non-standard extension with patchy tool support. **Not a real win.** |
| **Dependency cost**        | Zero — `VACUUM INTO` is a SQL statement.                                                                                                                                                                         | A new crate in a tree deliberately kept minimal and `cargo deny`-audited (`README.md:117`).                              |

**Recommendation: keep the format as-is.** Compression is the only thing zip
would buy, and it is the one thing this data does not need. Two worthwhile
refinements that do not change the format:

1. **Verify the snapshot before reporting success.** Open the staged file and run
   `PRAGMA integrity_check` (and `PRAGMA foreign_key_check`) before the rename at
   `commands.rs:2301`. Today a corrupt-but-written snapshot reports success, and
   the user learns otherwise only when they need it.
2. **Consider encryption-at-rest for the backup, not compression.** The snapshot
   contains client names, phone numbers, addresses and debt positions — PII under
   Tunisian loi 2004-63, a sensitivity the repo already takes seriously
   (`.gitignore:42-45`, `architecture.md:186-188`). Today it is written in clear
   to a user-chosen path, routinely a USB stick (`commands.rs:2290-2292`). If this
   is ever addressed, use SQLCipher or age/`ChaCha20-Poly1305`, **not** zip
   encryption.

One known open item from the prior audit, unrelated to format: `AUDIT_REPORT.md:172`
(I3) notes `VACUUM INTO` holds the connection mutex on the async runtime and
suggests `spawn_blocking`. On a multi-MB file this briefly freezes other commands.

---

## 3. Safe Upgrade Practices (DB-related)

Upgrade in this architecture means: the user installs a newer `.deb`/`.msi`, and
the app migrates its own SQLite file on next launch. There is **no auto-updater**
— no `updater` key in `src-tauri/tauri.conf.json`, no `tauri-plugin-updater` in
`src-tauri/Cargo.toml`, corroborated at `AUDIT_REPORT.md:461-465`.

### 3.1 Are migrations version-controlled and applied via a migration tool? — ✅ (hand-rolled, but disciplined)

No third-party tool: **no Flyway/Liquibase/Alembic/Prisma, no `refinery`,
`rusqlite_migration`, `sqlx`, `diesel`, or `sea-orm`** (absent from both
`Cargo.toml` and `Cargo.lock`), **no `migrations/` directory, and no `.sql`
files anywhere.** Also no `tauri-plugin-sql` — persistence is hand-rolled
`rusqlite` behind Tauri commands, by design (`architecture.md:432-434`).

What exists instead is a proper migration ladder in Rust, version-controlled with
the code:

```rust
// src-tauri/src/db.rs:96-100 — index in the slice *is* the version
const MIGRATIONS: &[fn(&Connection) -> DbResult<()>] = &[
    m0001_initial_schema,
    m0002_client_archive,
    m0003_purchase_archive,
];
```

- Version is tracked in `PRAGMA user_version` (`db.rs:114`, `db.rs:138`).
- **Each step runs inside its own transaction together with the version bump**
  (`db.rs:133-147`), with `ROLLBACK` on failure — so a half-applied schema can
  never be recorded as complete.
- Append-only is documented as a hard rule (`db.rs:90-95`,
  `architecture.md:412-414`).
- Steps are **replay-safe**: `add_column_if_missing` (`db.rs:223-233`) checks
  `pragma_table_info` before `ALTER TABLE ADD COLUMN`, because SQLite has no
  `ADD COLUMN IF NOT EXISTS` and a blind `ALTER` would fail `Db::open` and take
  the app down at launch.
- Tested: `migrate_is_versioned_and_idempotent` (`db.rs:769`) replays from
  `user_version = 0` with a sentinel row and asserts no data loss and exactly one
  `archived_at` column per table; plus `db.rs:833` and `db.rs:870`.

This is a legitimate substitute for a migration framework at this scale. It is
not a gap.

### 3.2 Is there a backup-before-migrate step? — ❌ Missing (the most serious gap)

`Db::open` applies PRAGMAs, migrates, then seeds — with **no snapshot first**:

```rust
// src-tauri/src/db.rs:29-40
conn.execute_batch("PRAGMA journal_mode = WAL; ... PRAGMA foreign_keys = ON;")?;
migrate(&conn)?;                    // <-- line 35: no backup taken before this
let db = Db { conn: Mutex::new(conn) };
db.seed_if_empty()?;
```

Nothing in CI does it either (there is nothing to back up in CI). So on the first
launch after an upgrade, the user's only copy of their ledger is migrated in
place with no automatic fallback. If a future migration is wrong, the only
recovery is a manual backup the user had to have thought to take — which the code
itself acknowledges (`commands.rs:2215-2216`, `commands.rs:532`,
`architecture.md:428-429`).

The mitigation is real but partial: `migrate` is transactional per step, so a
_failing_ migration rolls back cleanly. It does not protect against a migration
that **succeeds and is wrong** — the case that destroys data.

### 3.3 Are migrations tested in a staging environment before production? — ⚠️ Partial

**Tested in CI: yes.** `ci.yml:120-149` runs `cargo test --locked`
(`ci.yml:148`), and the job's header comment names the migration ladder as its
reason to exist:

> `ci.yml:113-119` — "several of the rules the backend enforces — the money
> invariants, **the migration ladder**, the licence signature check — are runtime
> behaviour that compiles perfectly well when broken."

Crucially, this gate is **load-bearing for releases**: `build.yml:28-30` defines a
`gate` job that is `uses: ./.github/workflows/ci.yml`, so no installer is built
unless the migration tests pass.

**Staging environment: N/A / ❌.** There is no staging tier and nothing to host —
no `environment:` key in any of the five workflows, so not even a GitHub
Environments approval gate. The only prod/dev distinction is **compile-time**
(licence key in `src-tauri/build.rs:41-49`, demo seeding in `db.rs:86-88`, log
level). Note that "staging" elsewhere in the codebase
(`commands.rs:2227`, `:2268-2280`) refers to the backup _staging file_, not an
environment.

The substantive gap: migrations are only ever exercised against **fresh or
synthetic** databases created by the test fixtures. They are never run against a
copy of a real, aged, user database — the one that actually finds problems.

### 3.4 Is there a rollback strategy for failed migrations? — ⚠️ Partial

| Mechanism                     | Status                                                                                                                                                      |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Per-step transaction rollback | ✅ `db.rs:141-147` — `ROLLBACK` on failure, version not bumped                                                                                              |
| Down-migrations               | ❌ **None exist.** No `down`/`revert` function, no reverse DDL anywhere in `MIGRATIONS`                                                                     |
| Point-in-time restore         | ❌ N/A — no WAL archiving, no PITR concept for a local SQLite file                                                                                          |
| Blue/green                    | N/A — no server, no traffic to shift                                                                                                                        |
| Downgrade protection          | ✅ `db.rs:117-128` — refuses to open a DB whose `user_version` exceeds what the binary supports, returning `AppError::internal` rather than operating blind |

The downgrade guard at `db.rs:117-128` is correct behaviour, but it has an
important consequence to state plainly: **reinstalling the previous version is
not a rollback.** Once the ladder has advanced, the older binary refuses to
open the database at all — and because `Db::open` is propagated with `?` from
`setup` (`src-tauri/src/lib.rs:69-70`), **the app will not launch**. The only
actual rollback is restoring a backup file by hand.

### 3.5 Are schema changes backward-compatible with the previous app version? — ✅ So far, but unenforced

Both shipped migrations are **purely additive**: `m0002_client_archive`
(`db.rs:240-244`) and `m0003_purchase_archive` (`db.rs:252-258`) each add one
nullable `TEXT` column plus an index. Nothing drops or renames a table or column;
there are **no `DROP TABLE`, `DROP COLUMN`, or 12-step table-rebuild migrations**
anywhere. An older binary reading a newer-but-additive schema would be fine on
data — it is the `user_version` guard (3.4), not the schema, that stops it.

Two supporting patterns reduce future drift risk:

- New settings need no migration at all — `read_settings`
  (`commands.rs:1932-1946`) reads every key through `get_setting(conn, key,
default)`, and `seed.rs:224-228` uses `INSERT OR IGNORE`. The licence clock
  watermark explicitly exploits this (`license.rs:358-364`).
- **No `SELECT *` anywhere** (`architecture.md:386-389`, `commands.rs:2995-3001`)
  — so a future column added to `payment` cannot silently shadow a joined
  purchase column and put wrong data on screen with no error.

The gap is that additive-only is a **convention, not a rule**: nothing in CI
fails a migration that drops a column, and `architecture.md:412-414` only says
"never reorder or edit a step that has shipped".

Note that "zero-downtime deploy" is **N/A** — a desktop app has no rolling
deployment where two versions run concurrently against one database. The
single-instance plugin is registered first specifically so two _processes_ never
open the same file (`src-tauri/src/lib.rs:25-33`, `architecture.md:66-70`).

### 3.6 Are there health checks / smoke tests post-migration before traffic cutover? — ❌ Missing (cutover is N/A)

- **Post-migration verification: none.** After `migrate` succeeds
  (`db.rs:35`) the app proceeds straight to seeding and normal operation. There
  is **no `PRAGMA integrity_check`, no `PRAGMA foreign_key_check`, and no
  `PRAGMA application_id`** anywhere in the tree. A migration that corrupts a
  constraint would be discovered by a user, in production, as wrong numbers.
- **Post-release verification: none.** `build.yml` ends at
  `tauri-apps/tauri-action` (`build.yml:119-131`). No install test, no launch
  check, no checksum or signature step. There is also no code signing or
  notarization configured.
- **Nightly E2E is not a smoke test for this.** `e2e.yml` runs Playwright
  against the **in-memory browser mock** (`src/api/mock.ts`), which has no schema
  concept at all — it is a UI regression suite, not a database check.
- **Traffic cutover: N/A.** No server, no updater, no rollout mechanism.

The one human checkpoint that does exist: releases publish as **drafts**
(`build.yml:129` `releaseDraft: true`), so someone reviews the assets before
users can download them. That is the _last_ checkpoint — with no auto-updater and
no environment gate, there is no way to halt a bad release once installers are
out.

### 3.7 Overall assessment

**Currently applied — genuinely good:**

- Version-controlled, append-only, transactional migration ladder with correct
  `user_version` bookkeeping (`db.rs:96-150`)
- Replay-safe `ALTER TABLE` handling, the specific mistake that bricks SQLite
  apps at launch (`db.rs:223-233`)
- Forward-version refusal instead of blind operation (`db.rs:117-128`)
- Migration tests that CI runs on every push **and** as a release gate
  (`ci.yml:148`, `build.yml:28-30`)
- Durability PRAGMAs applied before migrating — WAL, `busy_timeout`,
  `synchronous = NORMAL`, `foreign_keys = ON` (`db.rs:29-34`)
- Additive-only schema changes to date
- A correct, well-tested, WAL-safe backup primitive to build on
  (`commands.rs:2236-2313`)

**Missing:**

- No backup before migrating (3.2) — the highest-impact gap
- No automated backup at all, no retention, no record that one ever happened (§1)
- No post-migration integrity verification (3.6)
- No rollback path other than a manual restore; the old installer won't launch (3.4)
- Migrations never exercised against realistic aged data (3.3)
- Additive-only is convention, not an enforced rule (3.5)

**Concrete recommendations, in priority order:**

1. **Snapshot before migrating.** In `Db::open`, when
   `user_version < MIGRATIONS.len()`, `VACUUM INTO` a
   `payment_schedule.pre-v{n}.db` beside the DB before the ladder runs. The logic
   already exists in `backup_database_impl` (`commands.rs:2236`) — extract the
   `VACUUM INTO` + stage + rename core so both callers share it. **This is one
   function call away and closes the worst gap.**
2. **Verify after migrating.** Run `PRAGMA integrity_check` and
   `PRAGMA foreign_key_check` once `migrate` returns `Ok`; on failure, refuse to
   start with a distinct error code and point the user at the pre-migration
   snapshot from (1). Add the code to the table in `error.rs:69` and the
   `errors.*` key to **all three** locale files, per `architecture.md:174-178`.
3. **Prune pre-migration snapshots to the last 2-3**, so app-data does not grow
   unbounded. There is no retention logic today for anything.
4. **Record and surface backup recency.** Write a `last_backup_at` key to the
   `setting` table on success (no migration needed — see 3.5) and show a Settings
   banner when it is stale. Users who never think to back up are exactly the ones
   the current design fails.
5. **Ungate backup from the licence check.** Move `backup_database` into the
   unlicensed baseline alongside `get_settings` (`architecture.md:112-116`). An
   expired licence should never block a shop from copying its own ledger.
6. **Add a migration test against a realistic fixture** — a checked-in
   pre-populated DB at each historical `user_version`, migrated forward and
   asserted on row counts and money invariants. This is the practical substitute
   for a staging environment.
7. **Write down the additive-only rule** explicitly in `architecture.md` §Schema
   versioning, next to the existing append-only rule at lines 412-414.
8. **Ship a documented restore procedure** in `README.md`. Right now `README.md:309-310`
   only documents how to _reset_ (destroy) the database, not how to restore a
   backup — and there is no `restore_database` command to do it in-app.

---

## Summary

| Question                                       | Status | Key Recommendation                                                                                                  |
| ---------------------------------------------- | :----: | ------------------------------------------------------------------------------------------------------------------- |
| **1. Automated backups**                       |   ❌   | None exist — backup is a manual button click. Add a pre-migration snapshot and a staleness nudge (recs 1, 4).       |
| **1b. Backup frequency / schedule**            |   ❌   | N/A — user-triggered. No cron, timer, or trigger anywhere.                                                          |
| **1c. Last / next backup date**                |   ⚠️   | Next: none. Last: only a dev-machine artifact (2026-08-05 02:14). Persist `last_backup_at` in `setting` (rec 4).    |
| **1d. Retention policy**                       |   ❌   | Not found in repo. Add pruning once automated snapshots exist (rec 3).                                              |
| **1e. Backup availability**                    |   ⚠️   | Licence-gated (`commands.rs:2224`) — an expired licence blocks it. Move to the unlicensed baseline (rec 5).         |
| **2. Backups stored as zip?**                  |   ✅   | No, and correctly so — uncompressed SQLite via `VACUUM INTO` (`commands.rs:2282`).                                  |
| **2b. Is zip a good choice here?**             |   ✅   | **No — keep `VACUUM INTO`.** Native, WAL-safe, zero-step restore. Zip buys only compression this data doesn't need. |
| **2c. Snapshot integrity / encryption**        |   ⚠️   | Verify with `PRAGMA integrity_check` before reporting success; if encrypting, use SQLCipher/age — never zip crypto. |
| **3a. Migrations version-controlled + tooled** |   ✅   | Hand-rolled but sound: append-only ladder, `user_version`, per-step transactions (`db.rs:96-150`).                  |
| **3b. Backup-before-migrate**                  |   ❌   | **Highest-impact gap.** Snapshot in `Db::open` before `migrate` at `db.rs:35` (rec 1).                              |
| **3c. Tested in staging before production**    |   ⚠️   | CI gates releases (`build.yml:28-30`), but only on synthetic DBs. Add aged-fixture migration tests (rec 6).         |
| **3d. Rollback strategy**                      |   ⚠️   | Per-step `ROLLBACK` only. No down-migrations; the old installer won't even launch (`db.rs:117-128`). Rely on rec 1. |
| **3e. Backward-compatible schema changes**     |   ✅   | Additive-only so far — but convention, not rule. Write it down (rec 7). Zero-downtime deploy is N/A.                |
| **3f. Post-migration health check**            |   ❌   | None. Add `integrity_check` + `foreign_key_check` after `migrate` succeeds (rec 2).                                 |
| **3g. Traffic cutover / blue-green**           |  N/A   | No server and no auto-updater — draft releases (`build.yml:129`) are the only checkpoint.                           |

**Bottom line for a ship/no-ship decision:** the migration machinery is
production-quality; the _safety net around it_ is not. No release has been cut
yet (`git tag` is empty), so no user database has ever been migrated — this is
the cheapest possible moment to add recommendations 1 and 2. Both are small,
local changes that reuse code already in the tree.
