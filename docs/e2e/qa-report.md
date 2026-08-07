# QA Report — paymentSchedule

This file is the durable QA record for the project. Each QA pass appends a new
dated section (most recent first). Format per entry: **Summary → Test cases →
Issues found → Recommendations**. See `CLAUDE.md` (Phase 3: QA) for the workflow.

---

## 2026-08-07 (d) — Licence expiry takes effect without a restart (AUDIT_REPORT L4)

### Summary

`AUDIT_REPORT.md` **L4** was the last correctness gap in the licensing module:
the verdict was computed once in `lib.rs` and cached in `LicenseState` for the
life of the process. Every gated command consulted that cache faithfully — but
nothing ever recomputed it, so a shop that leaves the app open across its expiry
date kept full access until the next launch. For a desktop app a shop keeper
opens on Monday and closes on Saturday, "only on restart" can mean a week.

The fix has two halves, and both were needed:

- **The gate.** `commands::start_license_watcher` spawns a detached thread that
  re-runs `evaluate_license` every **15 minutes** and writes the result back into
  `LicenseState`, so `require_license` — the check that actually refuses — is
  authoritative within a tick. 15 minutes because expiry is date-granular: minute
  precision buys nothing, and a poll rather than a computed sleep to midnight is
  the only shape that survives suspend/resume and a moved system clock. Same
  reasoning, and the same `std::thread` shape, as the backup scheduler.
- **The screen.** Without the second half the UI would go on presenting a
  licensed install while every gated command refused — refusal toasts with no
  explanation, which is worse than the bug. `publish_license` emits
  `license://changed` when, and only when, the verdict differs; the renderer
  subscribes through the new `api.onLicenseChanged`, and the store applies the
  payload, flipping `App.vue`'s existing `blocked` computed with no restart and
  no polling.

Three decisions worth recording:

- **A timer, not window focus.** The audit offered either. Focus is a signal the
  renderer owns, and the renderer is the thing the gate defends against; a
  verdict that only refreshes when the user clicks the window is not a control.
- **`license://changed` is the app's first backend-pushed event.** Everything
  else in the tree is request/response through a command. It was allowed for
  free — `core:event:default` was already in `capabilities/default.json` — and it
  is emitted only on an actual change, so 96 ticks a day produce no traffic.
- **The lapse is announced.** The page swapping under a user mid-task without a
  word reads as a bug, so `App.vue` toasts `license.lapsed` on the licensed →
  unlicensed transition only. Error toasts persist until dismissed, so the reason
  is still on screen when the user looks up.

### Test cases

**Rust — `cargo test`, 163 passed (was 161), run.**

| #   | Case                                                                                                                                                                                                                                               | Result   |
| --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| R1  | `the_gate_follows_a_verdict_that_changes_mid_session` — `require_license` admits a `Valid` state, refuses with `LICENSE_REQUIRED` after `set(Expired)`, and admits again after `set(Valid)`. The L4 property itself, and it needs no Tauri runtime | ✅       |
| R2  | `a_re_evaluated_verdict_is_only_published_when_it_differs` — an unchanged licence compares equal across evaluations; `Valid` and `Expired` do not. The comparison `publish_license` gates its emit on                                              | ✅       |
| R3  | Pre-existing licence suite (45 cases) re-run unchanged — expiry inclusivity, the clock guard, the watermark, wire projection, signature handling                                                                                                   | ✅       |
| R4  | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`                                                                                                                                                                                   | ✅ clean |

**Frontend unit — `npm test`, 238 passed across 19 files (was 224/19), run.**

| #   | Case                                                                                                                          | Result   |
| --- | ----------------------------------------------------------------------------------------------------------------------------- | -------- |
| U1  | A verdict pushed by the backend is adopted with no second `load()`; `status`, `isLicensed` and `expiredOn` all follow         | ✅       |
| U2  | A licence installed elsewhere in the session unlocks the store the same way                                                   | ✅       |
| U3  | `watch()` subscribes exactly once — including two calls in the same tick, before the first resolves                           | ✅       |
| U4  | `unwatch()` stops delivery                                                                                                    | ✅       |
| U5  | A subscription that cannot be set up leaves the current verdict alone and logs. Deliberately **not** fail-closed — see Issues | ✅       |
| U6  | Pre-existing store suite (8 cases) re-run unchanged, including fail-closed on a broken `getLicenseStatus`                     | ✅       |
| U7  | `npm run lint`, `npm run build` (`vue-tsc --noEmit`)                                                                          | ✅ clean |

**Integration — `tests/integration/license-gate.integration.test.ts`, 5 new cases. Written, _not run_** (per `CLAUDE.md`: integration and E2E execute only on explicit request).

| #   | Case                                                                                                                              |
| --- | --------------------------------------------------------------------------------------------------------------------------------- |
| I1  | A lapse travels backend → gateway → store, carrying `expiredOn` and the attested licensee                                         |
| I2  | The pushed payload and `getLicenseStatus()` agree — two projections of one verdict, so the UI cannot depend on which arrived last |
| I3  | Releasing the subscription stops delivery                                                                                         |
| I4  | An import recovers the session, through the same publish path the watcher uses                                                    |
| I5  | `license.lapsed` resolves to a real sentence in fr, en and ar — a missing key would echo the key back at the user                 |

**E2E** — no new scenarios. `tests/e2e/run.mjs` drives the mock through the UI and has no hook for a timed backend push; the transition is covered at the integration layer instead.

**Manual (not yet performed — needs a real bundle):** install a licence expiring today; leave the app open across midnight or swap `license.json`; confirm within 15 minutes that the log records the change, the licensed route swaps to `LicenseRequiredPanel` with the date, the toast appears, and a gated action refuses. Repeat in Arabic to confirm the toast mirrors.

### Issues found

**1. Race between the watcher and `import_license` — found in self-review, fixed before QA.**
Both write `LicenseState`, and `publish_license` compares-then-sets. A tick that
read the licence file just before an import landed would publish its stale
verdict _after_ the import published the new one, locking a paying customer out
of the licence they had just installed for up to 15 minutes. Fixed by holding the
connection guard — which both paths already take — across the evaluate/publish
pair, so the file read is ordered against the publish. Lock order was checked for
inversion: `require_license` releases the `LicenseState` read lock before any
path acquires the connection, so no path takes them in the opposite order.

**2. Synchronous throw escaping a `Promise`-typed gateway call — found by a test, fixed.**
The gateway's browser branch is `Promise.resolve(mockDb.x())`, which evaluates
its argument first, so a mock that throws throws _synchronously_ despite the
declared `Promise` return. The store's first `.catch()`-on-a-chain form never saw
it. Rewritten as an `async` helper, which normalizes both. This shape is
pre-existing across the whole gateway (`importLicense` has it too) and is
harmless wherever the caller is itself `async` — noted below rather than fixed
project-wide.

**3. No blockers outstanding.** No security or data-loss issue open.

### Recommendations

- **A stale screen is preferred to a locked-out shop, deliberately.** If the
  subscription cannot be registered the store keeps its current verdict and logs,
  rather than failing closed as `load()` does. The Rust gate still refuses on its
  own, so the cost is a screen that lags until the next launch; failing closed
  would lock a paying shop out over a missing event listener. Recorded here
  because it is the one place in the licence path that does not fail closed.
- **The 15-minute window is a business choice, not a technical limit.** A shop
  can work up to 15 minutes past an expiry. Narrowing it is a one-constant change
  (`LICENSE_TICK`); the cost is a file read and a machine-id hash per tick.
- **Consider normalizing the gateway's mock branch** so no `Promise`-typed call
  can throw synchronously. Every current caller is `async` and absorbs it, so
  this is hygiene, not a live bug — but issue 2 above is what it looks like when
  a caller is not.
- **The event name is duplicated across the boundary** (`commands.rs` and
  `src/api/index.ts`), like every command name. Unavoidable without a codegen
  step; both sides carry a comment pointing at the other.
- **Run the integration suite** (`npm run test:integration`) before the release
  bundle, and perform the manual pass above once a signed licence is available —
  the timer and the real Tauri event bridge are the two things no test in the
  tree exercises.

---

## 2026-08-07 (c) — The automatic backup becomes a schedule the shop controls

### Summary

The automatic backup added in the previous entry fired at launch and nowhere
else, which is a safety net rather than a schedule: nobody could say when it ran,
and an app left open all week produced one copy. It now runs on a schedule —
**17:00 every day by default** — with the frequency (daily/weekly/monthly) and
the time editable in Settings. The pre-migration snapshot is untouched: it
answers a different risk and must stay tied to the launch that migrates.

Everything hangs off one pure predicate, `autobackup::due`, used by both the
scheduler tick and the launch pass so no second rule can disagree:

```
due = enabled AND ( elapsed >  interval                        // overdue → now
                    OR (elapsed >= interval AND now >= time) ) // today's window
```

The first branch is the one that makes a time of day mean anything here. A shop
that runs the app 09:00–16:00 is never open at 17:00; under a plain "fire at the
scheduled time" rule they would **never be backed up at all**, and the failure
would stay silent until the day they needed the copy. The overdue branch catches
them the next morning, and is also the launch catch-up.

Three supporting decisions, each visible in the code and its comment:

- **A 60 s poll, not a computed sleep to the next occurrence.** A poll is the
  only shape that survives the user changing the time, the machine suspending,
  and the clock moving — and it is what makes an edit take effect within a
  minute with no restart.
- **A plain `std::thread`, not an async task.** The work is blocking
  (`VACUUM INTO` plus two verification PRAGMAs); keeping it off the executor is
  the fix `AUDIT_REPORT.md:172` (I3) asks for on the manual path.
- **Its own connection, not the shared `Mutex<Connection>`.** At launch the mutex
  was free; 17:00 lands while the shop is typing, and WAL lets a second
  connection read a consistent snapshot without blocking writers.

The schedule is licensed configuration, like the currency or the alert window —
`is_language_only` refuses it. The backups keep running on whatever is stored,
and the manual button still carries no gate, so no expired install is ever left
unable to copy its ledger.

**Executed:** Rust `cargo test` **161 passed**, `cargo fmt --check`,
`cargo clippy --all-targets -D warnings` — clean. Frontend `npm test`
**233 passed** (19 files), `npm run lint`, `npm run build`,
`tsc -p tsconfig.test.json --noEmit` — clean.

**Also executed:** the real application, four times, against an isolated
`XDG_DATA_HOME`. See _Live application_ — this is where a scheduler either works
or does not, and unit tests cannot tell you which.

**Also executed on request:** integration **238 passed** (8 files) — after fixing
a real api/mock parity break it caught, see _Issues found_. **E2E could not
run**: Playwright's browser is missing from this machine
(`chromium_headless_shell-1234`), so `npx playwright install` is needed first.
It ran green earlier today (49/50, one pre-existing unrelated failure), and this
change adds nothing it exercises — the backup card is `v-if="isTauri()"`.

### Test cases

#### Rust unit — `autobackup.rs` (executed, part of 161 passed)

`due()` is the feature, so it gets a truth table rather than a happy path:

- `the_daily_schedule_fires_at_its_time_and_not_before` — 16:59 no, 17:00 yes,
  21:30 yes; and never twice on a day already covered.
- `a_shop_that_is_never_open_at_the_scheduled_time_is_still_backed_up` — the
  overdue branch, written out as the 09:00–16:00 shop it exists for.
- `a_launch_after_the_window_catches_up_immediately` — 19:30 start, nothing
  taken today.
- `the_longer_frequencies_wait_out_their_interval` — weekly and monthly at both
  edges of their interval.
- `a_first_run_is_owed_a_backup_immediately` — absent _and_ unparseable dates
  both read as "never".
- `a_disabled_schedule_never_fires` — including when long overdue.
- `the_scheduled_time_is_canonical_or_rejected` — `"7:05"` → `"07:05"`;
  `"25:00"`, `"17:60"`, `"17"`, `""`, `"5pm"`, `"17:00:00"`, `"-1:00"` refused.
  Refused rather than silently read as midnight, which would move every future
  backup to the middle of the night.
- `an_unknown_frequency_falls_back_to_daily` — a hand-edited database must not
  be able to stop the backups.
- The two pre-existing snapshot tests now also assert the `Outcome` they report,
  which is what arms the failure backoff.

`commands.rs`: `language_is_the_only_setting_an_unlicensed_user_may_change`
gained two cases covering the schedule fields, so the licence gate cannot start
admitting them by accident.

#### Frontend unit — `src/stores/settings-backup.test.ts` (executed, 9 passed)

Three added: the schedule does not feed `backupIsStale` either (an install set
to _monthly_ needs the nudge more, not less); the defaults ship as
enabled/daily/17:00; and `BACKUP_FREQUENCIES` matches the closed set the backend
accepts, so the `<select>` cannot offer a value that will be refused.

#### Live application (executed, four runs, isolated `XDG_DATA_HOME`)

1. **Catch-up.** Local time 18:06, default 17:00 schedule, nothing taken today →
   snapshot written on the first tick, same second as
   `automatic backup scheduler started`.
2. **No repeat.** Relaunch the same day → no second snapshot, mtime unchanged,
   zero `automatic backup written` lines.
3. **Fires at its time, not at startup.** `last_auto_backup_at` set to yesterday
   and the time set two minutes ahead. Scheduler started **18:08:05 and did not
   snapshot**; the copy was written at **18:10:05** — the first tick past the
   scheduled minute. This is the headline behaviour and the one that cannot be
   inferred from unit tests.
4. **A settings change applies without a restart.** Started with the time at
   23:59; after 75 s, no snapshot. The time was then changed **while the app
   kept running**; the copy appeared on the next tick. Confirms the per-tick
   re-read.

#### Integration — `error-contract.integration.test.ts` (executed, 238 passed)

The schedule round-trips through the gateway with the time normalised; three
lenient forms (`"9:05"`, `"17:6"`, `" 09:30 "`) are accepted and stored padded;
seven malformed times are each refused with `INVALID_SETTING_VALUE`; and a
frequency outside the set is refused **and writes nothing**.

These use the suite's own `codeOf` helper rather than `expect(...).rejects`: the
mock throws **synchronously** out of `api.updateSettings`, because the gateway
evaluates `mockDb.updateSettings(patch)` before wrapping it in
`Promise.resolve`, so there is no promise for `.rejects` to unwrap. The first
draft used `.rejects` and failed for that reason alone.

#### E2E — unchanged

The backup card is `v-if="isTauri()"`, so none of these controls exist in the
browser build the E2E suite drives.

### Issues found

**One parity break, found by running the integration suite and fixed.** The
mock's clock-time pattern required two digits (`^([01]\d|2[0-3]):([0-5]\d)$`),
while the Rust side parses with chrono's `%H:%M`, which takes one _or_ two
digits on either side. So `"9:05"` and `"17:6"` were accepted by the desktop
build and **refused by the browser build** — the api/mock parity invariant
`CLAUDE.md` calls a blocker rather than a nit. The unit tests could not see it:
each side was self-consistent, and only a test driving the shared gateway
compares them.

Fixed by making the mock width-lenient with the range check in code. The exact
leniency was then confirmed against chrono rather than assumed — the Rust test
now pins `"17:6"` → `"17:06"`, `"7:5"` → `"07:05"` and `"17:"` refused, so both
sides are held by assertions.

**One found in review and fixed: a retry storm.** On a persistent failure — a
full disk is the realistic case — an owed backup stays owed, so the scheduler
would have retried `VACUUM INTO` **every 60 seconds for as long as the app was
open**, writing megabytes of doomed attempts and a warning line a minute. Fixed
by having `snapshot_if_due` report an `Outcome` and the loop hold attempts off
for an hour after a failure. Ticking continues, so a settings change stays
responsive; only the attempt is suppressed. Pinned by the two tests that now
assert `Outcome::Failed` and `Outcome::NotDue`.

Nothing else introduced.

### Risks and edge cases

- **The "does not block the UI" claim is reasoned, not measured.** The scheduled
  snapshot opens its own connection and WAL permits a concurrent reader, so it
  should not stall commands — but no test drives the UI during a snapshot. At
  the observed 57 KB the window is too short to measure; it would matter at a
  much larger database.
- **A monthly schedule is 30 days, not a calendar month.** Deliberate — an
  interval cannot be configured into never firing, unlike "the 31st" — but
  someone expecting "the 1st of the month" will see it drift.
- **Moving the system clock backwards delays the next backup** by as much as the
  jump, because `due` compares dates. The licence clock watermark defends the
  licence against exactly this; a backup schedule is not worth the same
  machinery.
- **A disabled schedule still ticks**, taking the connection mutex once a minute
  to re-read settings. Cheap, and it is what lets re-enabling take effect
  without a restart, but it is not zero.
- **The hour-long backoff also delays a legitimate retry** after a transient
  failure — a USB stick briefly unmounted, say. An hour of exposure against a
  minute of hammering; the trade is deliberate.
- Unchanged and still open: no post-migration `integrity_check`
  (`db_report.md` rec 2), no encryption at rest, the unverified startup dialog
  from entry (b), and the E2E failure in `rescheduling an unpaid tranche from
the purchase editor holds the total`, which remains unrelated to any of this.

### Recommendations

1. **Run E2E once Playwright's browser is installed** (`npx playwright install`)
   — it is the only suite that has not run against this change.
2. **Confirm the Arabic layout** of the new controls: a `<select>` and an
   `<input type="time">` in an RTL card is the one thing here that reads
   differently by locale and that no test covers.
3. Still open from entry (b): confirm the startup dialog visually, and ship
   `db_report.md` rec 2 alongside `m0004`.

---

## 2026-08-07 (b) — Automatic database backup at launch

### Summary

Backup was manual-only: one button, and a nudge that merely prompts a human.
This adds two automatic snapshots, both taken at launch and both routed through
the existing `backup_database_impl` unchanged — no extraction, no second code
path, and they inherit its read-only `integrity_check`/`foreign_key_check`
verification for free. (`db_report.md` rec 1 asks for that core to be extracted;
it is already parameterised as `(conn, dest_path, staging_dir)`, so the
refactor was unnecessary.)

1. **Before a pending migration.** New `db::pending_migration` runs _before_
   `Db::open` and answers `Some` only when the file exists, carries a `client`
   table and sits behind `MIGRATIONS.len()`. On `Some`,
   `backups/payment_schedule.pre-v{n}.db` is written first. **If that fails the
   app refuses to migrate** — the ledger is left untouched and a native dialog
   explains why before it exits. This closes `db_report.md` rec 1, deferred in
   the previous entry.
2. **Once a day, after the schema is current.** `backups/auto-{date}.db`,
   guarded by a new `last_auto_backup_at` setting. Never fatal: a failure logs
   and is swallowed, because no irreversible step waits behind it.

Pruned as two pools — `auto-*` to 5, `pre-v*` to 2 — so a run of daily snapshots
can never evict the copy taken before a schema change.

**The automatic copies do not clear the manual-backup nudge, by design.**
`backups/` sits beside `payment_schedule.db` on one disk; a failed drive or a
stolen machine takes both. They defend against a bad migration and a mistaken
delete, not against losing the computer. `backupIsStale` still reads only
`lastBackupAt`, and a unit test pins that.

**Executed:** Rust `cargo test` **153 passed**, `cargo fmt --check`,
`cargo clippy --all-targets -D warnings` — all clean. Frontend `npm test`
**230 passed** (19 files), `npm run lint`, `npm run build` — all clean.
`tsc -p tsconfig.test.json --noEmit` clean.

**Also executed:** the real application, three times, against isolated
app-data directories via `XDG_DATA_HOME`. See _Test cases → Live application_.

**NOT executed:** integration and E2E, per the Phase 4 constraint.

### Test cases

#### Rust unit — `db.rs`, `autobackup.rs`, `lib.rs` (executed, 153 passed)

- `pending_migration_reports_only_a_database_worth_protecting` — walks all five
  answers in one fixture: absent file (and asserts the probe **does not create**
  it), file with no `client` table, fully-migrated database, pre-versioning
  database (returns the target _and_ the configured language), and a version
  ahead of this build. Every `None` is load-bearing: a false `Some` would turn a
  full disk into an app that refuses to open on a database with nothing in it.
- `the_pre_migration_snapshot_carries_the_data_and_leaves_the_source_alone` —
  the snapshot holds the sentinel row and staging is left clean.
- `the_daily_snapshot_runs_once_and_records_its_date` — snapshots, records the
  date, and a second call the same day is a no-op (asserted on mtime, since the
  filename would look identical either way).
- `a_failed_daily_snapshot_never_reaches_the_caller` — a missing directory logs
  and returns, and crucially **records no date**, so it does not suppress the
  retry for the rest of the day.
- `pruning_keeps_the_newest_and_touches_nothing_else` — 8 `auto-*` files reduce
  to the newest 5, while a `pre-v*` sibling and an unrelated `notes.txt` survive.
- `the_abort_dialog_speaks_the_configured_language` — ar/en/fr, with French as
  the fallback for absent and unrecognised values.

#### Frontend unit — `src/stores/settings-backup.test.ts` (executed, 6 passed)

Two cases added to the four from the previous entry:

- _is not satisfied by an automatic snapshot_ — a fresh automatic copy with no
  manual backup still nudges.
- _still nudges when the manual backup is stale but the automatic one is fresh_.

Both exist to stop the two notions being merged later by accident, which would
tell a shop they are covered when every copy they have is on the machine that
just died.

#### Live application (executed, three runs)

- **Daily snapshot, real dev database** (5 clients, 7 purchases, 30 payments,
  `user_version = 3`, with a 169 KB live `-wal`). Produced
  `backups/auto-2026-08-07.db`; `integrity_check` **ok**, `foreign_key_check`
  clean, all 5 clients present — so the snapshot carried the WAL contents, not
  just the main file. `last_auto_backup_at` recorded. The live database was
  verified healthy and unchanged afterwards (`integrity_check` ok,
  `user_version` still 3, same row counts).
- **Pre-migration snapshot**, isolated in a scratch `XDG_DATA_HOME` with a
  throwaway `m0004` appended and then reverted. The log ordering is the point:
  `took a pre-migration snapshot for schema version 4` precedes
  `applying schema migration 4`. The snapshot verified at `user_version = 3`
  **without** the probe table; the live copy advanced to 4 **with** it. The
  fallback is exactly the pre-migration state.
- **Refusal path**, same isolation, with `backups` made a regular file so only
  the snapshot could fail. The app logged
  `refusing to migrate without a pre-migration snapshot`, and the database was
  left at `user_version = 3` with no probe table and all 5 clients — the
  migration never ran.

#### Integration — `tests/integration/error-contract.integration.test.ts` (written, NOT run)

_carries the automatic-copy date through the gateway_ — `lastAutoBackupAt` is
`null` by default, travels the gateway when the mock holds the key, and survives
an `updateSettings` round trip. The Rust side writes it at launch, which the
browser build has no equivalent of, so without this the field could rot to
`undefined` unnoticed.

#### E2E — unchanged

Nothing to add: the backup card is `v-if="isTauri()"`, and the launch hook has
no browser equivalent at all.

### Issues found

**One defect found by this review and fixed, one design flaw found by running
the app and fixed.**

1. **The dialog strings contained runs of literal spaces.** They were written
   with `\` line continuations that a rewrite silently flattened, leaving
   `"...قاعدة البيانات،              ولم يتم..."` mid-sentence in all three
   languages. Nothing but reading the rendered dialog would have shown it.
   Fixed, and pinned by an assertion that no title or body contains a double
   space.
2. **The abort hung instead of aborting.** The first implementation called
   `blocking_show` from a thread joined inside `setup` — but a native dialog
   needs a running event loop to pump it, and inside `setup` the loop has not
   started. Measured: the process sat until the 45 s test timeout. Restructured
   so `setup` records the verdict in a `StartupBlocked` state, hides the window
   and returns, and `RunEvent::Ready` shows the dialog from a spawned thread
   once the loop is alive. Re-measured: **exits in 5 s**, ledger untouched.

### Risks and edge cases

- **The dialog's rendering is not verified.** GTK cannot initialise under Xvfb
  on this machine (`Failed to initialize GTK`), and the earlier window probe was
  invalid because `WAYLAND_DISPLAY` made GTK ignore the virtual X display
  entirely. What _is_ verified is everything around it: the refusal, the
  untouched database, and a prompt exit rather than a hang. **Confirm the dialog
  visually in a real desktop session before shipping** — the recipe is in
  _Recommendations_.
- **The exit code is 0, not the 1 requested.** `handle.exit(1)` is called but the
  process reports 0. Cosmetic for a desktop app — nothing consumes it — and
  untangling it was not worth delaying the correct behaviour, but it is a small
  known inaccuracy.
- **Startup now does disk work before the window appears**: `VACUUM INTO` plus
  two PRAGMAs, on the first launch of each day. Tens of milliseconds at the few-
  MB scale this data reaches; a much larger database would make it noticeable.
- **`backups/` grows to at most 7 files** (5 + 2). At the observed 57 KB that is
  nothing; at a few MB per copy it is tens of MB, all inside app-data.
- **The dialog text duplicates a fragment of `src/locales/*.json` in Rust**, and
  nothing keeps them in step but the comment saying so. Accepted: the dialog
  fires before vue-i18n exists, and the alternative is an Arabic-only shop
  reading French at the one moment it matters.
- **A same-day clock change can skip or repeat a snapshot**, since the guard
  compares ISO dates. Harmless in both directions.
- **The pre-migration snapshot still runs before `Db::open`**, so it opens the
  database on its own. It is the only writer at that moment — the
  single-instance plugin is registered first — but that ordering is load-bearing
  and worth remembering before anything else is added to `setup`.
- Unchanged from the previous entry: no post-migration `integrity_check`
  (`db_report.md` rec 2), no encryption at rest, and the E2E failure in
  `rescheduling an unpaid tranche from the purchase editor holds the total`,
  which is still open and still unrelated.

### Recommendations

1. **Confirm the dialog visually.** In a real desktop session:
   `mkdir -p /tmp/h/tn.paymentschedule && cp <a populated db> /tmp/h/tn.paymentschedule/ && printf x > /tmp/h/tn.paymentschedule/backups`,
   append a throwaway `m0004`, then
   `XDG_DATA_HOME=/tmp/h npm run tauri dev`. Expect a native error dialog in the
   database's configured language, and the app exiting when it is dismissed.
2. **Run the integration suite** before tagging — say the word.
3. **Ship `db_report.md` rec 2 with `m0004`**: a post-migration
   `integrity_check` whose failure message points at the snapshot this change
   now guarantees exists.
4. Still open from the previous entry: the purchase-editor E2E failure, aged-
   fixture migration tests (rec 6), and backup encryption at rest.

---

## 2026-08-07 — Pre-v1.0.0 backup safety: ungated, verified, and dated

### Summary

`db_report.md` audited the database practices at `834040a` and found the
migration machinery sound but the safety net around it absent. Its own priority
order puts "snapshot before migrating" first; that recommendation was
**deliberately deferred**, because at v1.0.0 every install creates a fresh file
and walks `user_version` 0→3 over an empty database — there is nothing to lose,
and the snapshot is taken by the binary that performs the migration, so it can
land in the release that introduces `m0004` and still protect that upgrade in
full.

What shipped instead is the category that only works if it is in the day-one
binary, plus the fixes that made the existing backup untrustworthy:

1. **Backup is no longer licence-gated.** `require_license` came off
   `backup_database` and `:disabled="locked"` came off the Settings button. An
   expired licence blocked the copy a shop most needs — the one taken before
   troubleshooting the expiry — while protecting nothing, since a snapshot only
   contains rows the unlicensed baseline already displays.
2. **The snapshot is verified before it is accepted.** `backup_database_impl`
   now opens the staged file **read-only** and runs `PRAGMA integrity_check` and
   `PRAGMA foreign_key_check` before the rename. A clean `VACUUM INTO` proves the
   statement ran, not that the bytes are a usable database; a full disk or a
   failing drive previously reported success and was discovered at restore time.
3. **`last_backup_at` is recorded and surfaced.** Success writes the date to the
   `setting` table (no migration needed — `read_settings` resolves every key
   through `get_setting` with a default) and the command returns the updated
   `Settings`, so the Settings page shows the date and warns after
   `BACKUP_STALE_DAYS` (30). This is the item that is worthless if deferred:
   users who install v1.0.0 and are never nudged cannot be helped retroactively.
4. **Restore is documented.** `README.md` previously documented only how to
   _destroy_ the database. It now leads with a five-step restore (quit first,
   replace the file, delete stale `-wal`/`-shm`), with the reset instructions
   kept and relabelled as data loss.
5. **The additive-only migration rule is written down** in `architecture.md`,
   next to the append-only rule, with the reason: a destructive step makes
   "reinstall the previous version" stop being a recovery option, because the
   older binary refuses a forward `user_version` and `Db::open` is propagated
   with `?` from `setup` — it will not launch at all.

**Executed:** Rust `cargo test` **147 passed**, `cargo fmt --check`,
`cargo clippy --all-targets -D warnings` — all clean. Frontend `npm test`
**228 passed** (19 files), `npm run lint`, `npm run build` (`vue-tsc --noEmit`)
— all clean. `tsc -p tsconfig.test.json --noEmit` clean.

**Also executed on request:** integration **225 passed** (8 files), including
the three new backup cases. E2E **49/50 passed** — the one failure,
`rescheduling an unpaid tranche from the purchase editor holds the total`, is
**pre-existing and unrelated**; it reproduces identically on a clean worktree of
`834040a`. See _Issues found_.

No new E2E case was added: the backup card is `v-if="isTauri()"`, so neither the
button nor the new status line exists in the browser build the E2E suite drives.

### Test cases

#### Rust unit — `src-tauri/src/commands.rs` (executed, 147 passed)

New:

- `a_snapshot_is_verified_before_it_is_accepted` — a real snapshot from
  `backup_database_impl` verifies; the same file truncated to half its length
  does not. Truncation is the shape a full disk leaves: the `SQLite format 3`
  header survives, so every destination guard still passes and only reading the
  pages reveals it.
- `the_backup_date_reads_as_absent_until_one_is_recorded` — `last_backup_at`
  reads as `None` on a database that has never seen the key (the no-migration
  claim), and round-trips once written.

Extended:

- `an_expired_licence_still_permits_reading_your_own_ledger` — now also takes a
  backup, pinning the capability the unlicensed baseline must keep exposing.
  **Stated limit:** the gate lived on the `backup_database` wrapper, which needs
  an `AppHandle` and cannot be reached from a unit test. This test pins the
  capability, not the absence of the call — that is a review rule.

Unchanged and still passing: the five existing backup tests (readable snapshot,
clobber refusal, sibling safety, overwrite, cross-filesystem fallback). The
"staging directory must be empty after a successful backup" assertion inside
`backup_writes_a_readable_snapshot` is load-bearing for the new verification
step — it is what proves the read-only verification connection leaves no
sidecar files behind for the rename to strand.

#### Frontend unit — `src/stores/settings-backup.test.ts` (executed, 4 passed)

New file, fake-timered on a fixed 2026-08-07 so the day arithmetic cannot drift:

- an install that has never backed up is stale (`lastBackupAt === null`);
- a backup taken today, and one taken yesterday, are not;
- `BACKUP_STALE_DAYS - 1` is quiet and `BACKUP_STALE_DAYS` nudges — both edges,
  so the comparison cannot silently invert.

#### Integration — `tests/integration/error-contract.integration.test.ts` (executed, 225 passed)

Two cases added to the existing `backup` block:

- _returns settings carrying the new backup date_ — `getSettings()` reports
  `null` first, `backupDatabase()` resolves to settings whose `lastBackupAt` is
  today, and a later independent read agrees. This pins the api/mock parity that
  the unit suite cannot see: the mock is what the E2E build runs against, and a
  drift means the browser build keeps nudging after a successful backup.
- _keeps the recorded date out of the writable settings patch_ — an
  `updateSettings` round trip preserves `lastBackupAt`. The field is read-only by
  construction (no counterpart in `SettingsPatch`, on either side), because the
  renderer serializes the patch and a writable field would let the UI lie about
  when the ledger was last copied.

#### E2E — `tests/e2e/run.mjs` (executed, 49/50 passed, unchanged)

`settings exposes a database backup action` still asserts the backup card is
**hidden** outside the Tauri runtime — passing. No case was added: there is no
database file in the browser preview, so the card, the button and the new status
line are all absent by design there.

### Issues found

**Pre-existing, not introduced here — E2E `rescheduling an unpaid tranche from
the purchase editor holds the total` fails.** Carried over from the 2026-08-04
entry, whose own record states the E2E cases were "written, NOT run" — this one
appears never to have passed.

- _Reproduction:_ `npm run test:e2e`. Confirmed identical on a detached
  worktree of `834040a` with none of this work applied, so it is not a
  regression from the backup changes.
- _Symptom:_ the test opens the Samsung purchase (id 1, 2 400 over 6 tranches of
  400, tranche 1 settled), types `600` into tranche 2, sees the sum indicator
  read `ok`, saves, and then finds tranche 2 still at 400 on the detail page.
  The failure screenshot
  (`tests/e2e/artifacts/rescheduling-an-unpaid-tranche-from-the-purchase-editor-holds-the-total.png`)
  shows the right purchase with **all six tranches unchanged** and the total
  still 2 400 — so nothing was written, rather than something wrong being
  written.
- _Isolated:_ a throwaway probe against the gateway performed the same
  reschedule (`api.updatePurchase` with `[400, 600, 350, 350, 350, 350]`) and it
  **persisted correctly**. The business layer is sound; the defect is in the
  editor's save path or in the test's own expectation, and telling those apart
  needs the modal driven in a real browser.
- _Severity:_ unknown until that is settled — a silently dropped schedule edit
  would be a should-fix before v1.0.0; a stale test expectation would not. It
  blocks nothing in this pass.

Nothing introduced by this work. Two findings from the self-review were fixed in
place before this entry:

1. **`verify_snapshot` opened the staged file read-write.** A verification step
   that can modify what it verifies is the wrong shape, and a journal file left
   beside the staged snapshot would be stranded by the rename. Changed to
   `OpenFlags::SQLITE_OPEN_READ_ONLY`; safe unconditionally because
   `VACUUM INTO` writes a rollback-journal database even from a WAL source, so
   no `-shm` is required.
2. **A stale comment in `SettingsView.vue`** still claimed every control on the
   page except the language was licensed. Backup is now a third exception;
   corrected rather than left to mislead the next reader.

### Risks and edge cases

- **The deferred gap is still open.** There is no pre-migration snapshot and no
  post-migration `integrity_check`. This is safe _only_ while no shipped
  migration runs against real data. The release that adds `m0004` must carry
  both, plus pruning of the snapshots — see `db_report.md` recs 1-3. If any
  machine outside this repo already holds a `payment_schedule.db` from a
  sideloaded dev build, v1.0.0 **will** migrate real data there and this
  assumption does not hold for that machine.
- **A failure to record `last_backup_at` is logged, not surfaced.** Deliberate:
  the snapshot is already on disk and good, and returning `BACKUP_FAILED` would
  send a user chasing a backup they actually have. The cost is that a settings
  table which has stopped accepting writes shows a permanently stale nudge, with
  only `logs/` to explain it.
- **The nudge fires on a brand-new empty install**, where there is nothing to
  lose. Accepted: it is confined to the Settings backup card, and teaching the
  habit before the data exists is the point.
- **`backupIsStale` compares against the local clock**, so a user who moves the
  system date backwards sees the nudge disappear. The licence clock watermark
  defends the licence against exactly this; a backup nudge is not worth the same
  machinery.
- **Nothing verifies the destination after the rename.** Verification happens in
  staging; a cross-filesystem `fs::copy` that truncates on the way out is still
  undetected, as documented in the existing comment at that fallback.
- **The snapshot is still written in clear.** It carries client names, phone
  numbers and debt positions — PII under Tunisian loi 2004-63 — to a
  user-chosen path, routinely a USB stick. Out of scope here; if addressed, use
  SQLCipher or age/ChaCha20-Poly1305, never zip encryption (`db_report.md` §2).

### Recommendations

1. **Settle the pre-existing E2E failure** above before tagging v1.0.0 — it is
   the only red in either suite, and it is a purchase-editor question, not a
   backup one.
2. **Manual desktop pass**, none of which the browser suites can reach: take a
   backup and confirm the status line switches from "never backed up" to the
   date; confirm the button works with an expired licence; walk the new README
   restore steps end to end; check the Arabic RTL rendering of the new line.
3. **Bundle recs 1-3 from `db_report.md` with the first new migration.** They are
   one work item and there is no user data to protect until then.
4. Longer term, unchanged from the audit: aged-fixture migration tests (rec 6),
   backup encryption at rest, and `AUDIT_REPORT.md` I3 (`VACUUM INTO` holds the
   connection mutex on the async runtime — now held slightly longer, since the
   verification and the settings write share the same guard).

---

## 2026-08-04 — Installment immutability: paid rows, payment dates, and one schedule editor

### Summary

Three business rules were enforced, and enforcing them moved the boundary
between the two editors:

1. A **settled** installment (`paid_amount >= amount`) has an immutable `amount`
   and `due_date` from anywhere. Its `paid_amount` stays editable.
2. A **recorded payment date is immutable**. Setting one the first time — which
   is what recording a payment is — is untouched.
3. An **unsettled** installment's `amount` and `due_date` move only through the
   Edit Purchase flow. The tranche editor no longer offers them.

Rule 3 forced a change to `update_purchase`. It regenerated the schedule by
`DELETE` + re-`INSERT` and so refused outright (`PURCHASE_HAS_PAYMENTS`) once
any payment existed — which, combined with rule 3, would have frozen every
unpaid tranche the moment one sibling was paid. It now applies the schedule
**in place** via `apply_schedule_in_place`, so the rows and the ledger hanging
off them survive, and unpaid tranches stay editable on a purchase that has taken
cash. This relaxes a rule `architecture.md` previously stated as absolute; the
doc has been rewritten accordingly.

Enforcement is in Rust (`commands.rs`), mirrored guard-for-guard in
`src/api/mock.ts`. The UI mirrors it to explain, not to enforce. Two new error
codes: `SCHEDULE_VIA_PURCHASE` and `PAYMENT_DATE_LOCKED`, both localized in
fr/en/ar (key parity verified: 375 leaf keys, identical sets).

**Executed:** Rust `cargo test` **126 passed**, `cargo fmt --check`,
`cargo clippy --all-targets -D warnings` — all clean. Frontend `npm test`
**147 passed** (10 files), `npm run lint`, `npm run build` (`vue-tsc --noEmit`)
— all clean.

**NOT executed:** integration and E2E, per the `CLAUDE.md` Phase 4 constraint.
They are written and typecheck/lint clean but have not been run. See
_Recommendations_.

### Test cases

#### Rust unit — `src-tauri/src/commands.rs` (executed, 126 passed)

New:

- `the_installment_editor_refuses_the_schedule_fields` — `amount` and `due_date`
  each rejected with `SCHEDULE_VIA_PURCHASE` on a settled _and_ an unsettled
  row, including a value identical to what is stored.
- `the_schedule_refusal_precedes_every_lookup` — the guard beats
  `INSTALLMENT_NOT_FOUND` on an unknown id, proving it runs before the
  transaction opens.
- `a_recorded_payment_date_cannot_be_rewritten` — `PAYMENT_DATE_LOCKED`; the
  ledger row and the derived `paid_date` are unchanged.
- `a_payment_date_still_dates_the_entry_it_arrives_with` — a date travelling
  with a moved figure dates the new entry; a second correction adds its own.
- `a_settled_tranche_keeps_its_collected_figure_editable` — rule 1's open half.
- `rescheduling_moves_the_unpaid_tranches_around_a_settled_one` — row ids and
  the payment survive; the schedule moves.
- `regenerating_the_schedule_is_refused_once_a_tranche_is_settled` —
  `AMOUNT_LOCKED` / `DUE_DATE_LOCKED` per anchor field, nothing written.
- `rescheduling_below_what_a_tranche_collected_is_refused` — `BELOW_PAID:100`.
- `rescheduling_onto_the_collected_figure_settles_the_tranche` — derived
  `paid_date` comes from the ledger.
- `shortening_the_schedule_past_a_paid_tranche_is_refused`,
  `shortening_past_a_row_corrected_back_to_zero_is_refused`,
  `shortening_the_schedule_drops_only_empty_tranches`,
  `lengthening_the_schedule_appends_new_tranches`.
- `a_schedule_whose_dates_run_backwards_is_refused` — on create and on update.
- `a_refused_reschedule_rolls_the_purchase_row_back_too` — the label is written
  before the schedule and must roll back with it.

Rewritten: the former "schedule half" block, which tested amount/due-date
editing through `update_installment_impl`; `the_gate_does_not_reach_the_schedule_fields`
became `the_gate_does_not_reach_the_purchase_editor`; the bad-argument and
archived-purchase guards now drive money fields.

#### Integration — `tests/integration/` (written, NOT run)

`installment-edit.integration.test.ts` rewritten around the new split, with a
`reschedule()` helper that saves a whole schedule through `api.updatePurchase`:

- _Rule 3_ — schedule fields refused whatever the tranche's state and before the
  lookup; amount/due date accepted and **persisted** through the purchase
  editor; still accepted once a payment exists, with row ids and the ledger
  intact.
- _Rule 1_ — `AMOUNT_LOCKED` / `DUE_DATE_LOCKED` from the purchase editor;
  purchase row rolled back on refusal; collected figure still editable;
  partially-paid rows still reschedulable down to `BELOW_PAID`.
- _Rule 2_ — `PAYMENT_DATE_LOCKED`; setting one the first time still works; a
  second correction dates its own entry; `NO_PAYMENT_TO_DATE`; `FUTURE_PAID_DATE`;
  a note alone still amends.
- Length changes: drop empty rows, refuse dropping rows with ledger history
  (including one corrected back to zero), append past the stored rows, refuse
  backwards dates.
- `expectConsistent` now also asserts due dates run in position order.

`error-contract.integration.test.ts` — the two new codes added to the inventory
so they must resolve to localized prose in all three locales.

`purchase-archive.integration.test.ts` — the "refuses to reschedule a purchase
that has payments" case now expects `AMOUNT_LOCKED` / `DUE_DATE_LOCKED`, since
the refusal moved from a purchase-wide gate to a per-row one.

#### E2E — `tests/e2e/run.mjs` (written, NOT run)

- `the purchase editor locks a settled tranche but not the rest` (rewritten) —
  count/interval locked, total and label open, row 1 disabled, row 2 enabled.
- `rescheduling an unpaid tranche from the purchase editor holds the total`
  (new) — type 600 into row 2, sum stays green, save, re-read the detail page.
- `the tranche modal offers no schedule fields at all` (rewritten) — no
  `#inst-amount`, no `.rebalance`, figures shown read-only with a pointer note.
- `a paid tranche keeps its collected money editable and its date frozen`
  (rewritten) — `#inst-paid` enabled, the payment-date trigger disabled.
- `a tranche whose predecessor is unpaid locks only its money fields` — the
  amount assertion dropped; the rest stands.

### Issues found

1. **Ledger history could be silently destroyed (found in self-review, fixed).**
   The first cut of `apply_schedule_in_place` guarded dropped rows on
   `paid_amount > 0`. A row corrected back down to zero still holds the entries
   that took the money and gave it back; both would have cascaded away on a
   shortening reschedule. Because they net to zero, `SUM(payment.amount) ==
SUM(installment.paid_amount)` would still have held — no total would ever have
   surfaced the loss. The guard now counts `payment` rows. Pinned by
   `shortening_past_a_row_corrected_back_to_zero_is_refused` and its integration
   twin.

2. **Typed due dates were being discarded (found in self-review, fixed).**
   `NewPurchaseModal.rebuild()` regenerates every unlocked row's due date from
   the anchor fields, and it was watched on `totalPrice` too — so changing the
   total threw away hand-typed dates. Pre-existing, but it only became load-
   bearing now that this form is the only place a due date can be typed. The
   watcher is split: dates rebuild on count/interval/purchase-date, and a new
   total re-splits amounts only while they are still automatic.

3. **Payment-date field defaulted to the date already on record (fixed).**
   `EditInstallmentModal` seeded it from `installment.paidDate`, which under the
   new rule would back-date a correction made today to the original payment's
   date. It now defaults to today and stays editable, so a payment taken last
   week can still be dated honestly.

4. **`rebalance_amounts` lost its only Rust caller (accepted, documented).**
   A schedule now arrives whole and its sum is checked outright, so there is no
   single-row delta to absorb. It is kept under `#[allow(dead_code)]` with a
   comment: it is one half of a cross-language pair, `finance.ts` still runs the
   same algorithm in the purchase editor, and `tests/fixtures/finance-parity.json`
   is what proves the two agree. Deleting the Rust half would leave the fixture
   checking nothing. `NO_REBALANCE_ROOM` is likewise Rust-unreachable but still
   raised by the frontend.

### Risks and edge cases

- **Behaviour relaxation.** A purchase carrying payments can now be rescheduled
  where the whole editor used to be locked. Intended, and confirmed with the
  user before implementation, but it is a real widening of what a shopkeeper can
  change after taking cash. The per-row guards are the only thing standing
  between that and a rewritten history.
- **Auto-split against settled rows.** `resolve_schedule`'s generated path
  (`installments: null`) ignores settled rows entirely, so it will usually
  disagree with one and refuse. The UI never sends that shape on an edit — it
  always sends the displayed rows — but a direct IPC caller would see
  `AMOUNT_LOCKED` where "recompute the split" was meant. Correct, if terse.
- **`splitAmounts` remainder placement.** The purchase editor's `recompute` puts
  the rounding remainder on the last _unsettled_ row rather than the last row
  overall. Exact, but a shopkeeper re-splitting a purchase with a settled tail
  will see the odd unit land somewhere new.
- **Arabic/RTL not visually verified.** The read-only figures row gained a
  fourth item and wraps (`flex-wrap`), and the tranche rows gained a fourth grid
  track for the settled marker. Key parity is verified programmatically; the
  mirrored layout is not.
- **No component-level tests** for either modal — their behaviour is covered
  only by E2E, which has not been run. `EditInstallmentModal` got simpler here,
  but `NewPurchaseModal` got materially more complex (locked rows, `committed`
  snapshot, split watchers).

### Recommendations

1. **Run the integration and E2E suites** (`npm run test:integration`,
   `npm run test:e2e`). They are the only coverage of the three rules end to end
   through the gateway, and of every UI assertion above. Nothing in this pass
   executed them.
2. Verify the Arabic layout by hand: the tranche modal's figures row and the
   purchase editor's locked-row marker.
3. Consider a component test for `NewPurchaseModal` covering the locked-row
   pinning in `rebuild()` and the `rebalanceAmounts` redistribution — the two
   places where this change added real logic to the frontend.

---

## 2026-07-30 (c) — The sidebar's new-purchase shortcut steps aside on Achats

### Summary

The sidebar's **Nouvel achat** button is now hidden on the Achats page, which
carries its own primary button for the same action. `AppSidebar.vue` gained
`showNewPurchase = computed(() => route.name !== "achats")`; the purchase
_detail_ page keeps the shortcut, having no button of its own.

Reading `AchatsView.vue` while planning turned up a second reason: `?new=1` is
read in `onMounted` only (`AchatsView.vue:103-109`). Pushing `{ name: "achats",
query: { new: "1" } }` **from the Achats page itself** does not remount the view,
so the modal never opened. The shortcut was not merely redundant there — it was
dead. Hiding it removes the duplicate and the dead control together.

Presentational only: `create_purchase` stays licence-gated in Rust.

### Test cases

**Executed.** Frontend unit **147 passed** (10 files), `npm run lint`,
`npm run build` (`vue-tsc --noEmit`), E2E **49/49 passed** — run at the user's
explicit request. No `cargo`: `src-tauri/` is not touched.

Two unit cases added to `src/components/layout/AppSidebar.test.ts` (hidden on
`achats`; shown on `dashboard` and `achat-detail`). The `vue-router` mock there
now exposes `useRoute` through a `vi.hoisted` route object, so each test picks
its route before mounting.

One E2E test added: shortcut present on the dashboard → absent on Achats → the
page's own button still opens the modal → present again on `/achats/1`.

### Issues found

1. **The new E2E test failed on its first run — the test was wrong, not the
   feature.** It waited on `table.table tbody tr` after clicking the nav item,
   but the dashboard renders `table.table` too (`RecentPurchasesCard.vue:41`,
   `PurchaseDetailCard.vue:112`). The wait resolved instantly, before the route
   changed, and the assertion ran against the dashboard: expected 0, got 1.
   Fixed by waiting on `h1.page-title` — a route-driven value — as the
   neighbouring navigation test does. Re-run: 49/49.
   **Reproduction, for the record:** replace the `waitForFunction` with
   `await page.locator("table.table tbody tr").first().waitFor()` and re-run.
2. Found during review: the comment at `run.mjs:576` justified scoping a role
   query with "the sidebar also carries a permanent Nouvel achat button", which
   this change makes false. The scoping is kept (it guards against the rule
   changing back); the comment now says so.

The stale failure screenshot written by issue 1 was deleted from
`tests/e2e/artifacts/` once the test passed.

### Risks and edge cases

1. **A stale wait can make an E2E assertion vacuous rather than red.** Issue 1
   failed loudly only because the control it checked was still present. Had the
   assertion been "the shortcut is visible", it would have passed on the wrong
   page and tested nothing. Worth remembering: `table.table` is not unique to a
   list page.
2. **The hide is keyed on the route name `achats`.** Renaming that route in
   `src/router/index.ts` silently brings the duplicate button back; nothing links
   the two beyond the unit test.
3. **The `?new=1`-on-remount limitation is untouched**, only routed around. A
   future entry point that pushes that query from the Achats page will hit the
   same dead end.

### Recommendations

1. If the shortcut is ever wanted on Achats again, fix the root cause first:
   `watch` the route query in `AchatsView` instead of reading it in `onMounted`.
2. Consider asserting the route name in the unit test alongside the class, so
   risk 2 fails a test rather than reaching a user.

---

## 2026-07-30 (b) — The shop name comes from the licence, and shows beside the logo

### Summary

The shop name was in two places at once: a `shop_name` row in the `setting` table,
editable in **Paramètres → Boutique** but rendered nowhere, and the `licensee`
field of the signed licence, shown only in the licence card. The name now comes
from the licence and is displayed in the sidebar brand block, beside the logo,
replacing the generic "Paiements / Échelonnés" title. The editable field is gone
from Paramètres.

Fallback chain, in `AppSidebar.vue`: `license.license?.licensee` → the stored
`shop_name` setting → the app title. The licence branch covers `valid`, `expired`
and `machineMismatch`, the three statuses where a payload has actually verified;
`missing`, `invalidSignature`, `malformed` and `clockTampered` carry no payload
and fall through to the setting.

No Rust change: `licensee` already crossed IPC on `LicenseInfo`. `shop_name`
stays in `Settings`/`SettingsPatch` as the fallback, so `is_language_only`'s
exhaustive destructure is untouched and the unlicensed write rule is unchanged.

### Test cases

**Executed.** Frontend unit **145 passed** (10 files), `npm run lint`,
`npm run build` (`vue-tsc --noEmit`). E2E **48/48 passed** — run at the user's
explicit request. No `cargo`: `src-tauri/` is not touched by this change.
Integration suite not run (not requested, and it exercises no UI).

New unit file `src/components/layout/AppSidebar.test.ts`, one case per branch:

| Case                                            | Expectation                                          |
| ----------------------------------------------- | ---------------------------------------------------- |
| Valid licence, a stored `shopName` also present | `.brand-name` = licensee; the setting loses          |
| `expired` with a verified payload               | Name still shown — a lapsed licence is not anonymous |
| No licence, stored `shopName` set               | `.brand-name` = the setting                          |
| No licence, `shopName` blank (whitespace only)  | Falls back to `.brand-line1` / `.brand-line2`        |

E2E changes, all in `tests/e2e/run.mjs`:

1. "app shell + sidebar render" now asserts `.brand-name` = "Boutique de
   démonstration" (the mock's licensee) instead of the two title lines.
2. "switching to Arabic mirrors the layout to RTL" no longer uses the brand block
   as its i18n probe — a licence holder's name is a proper noun and identical in
   every locale, so it would have silently stopped testing anything. Both the
   French baseline and the Arabic assertion moved to `h1.page-title`.
3. **New** — "the shop name is licence-owned: no input for it in settings":
   asserts `#set-shop` is absent, and that the licence card and the sidebar show
   the same name.

### Issues found

None. Two review findings were fixed during the work:

1. `saveShop` in `SettingsView.vue` no longer wrote a shop name, making its name
   misleading. Renamed `saveShopInfo`; it now patches `shopInfo` only.
2. `features.md` still described the shop name as an editable setting, and the
   "attested, not configured" decision was recorded nowhere. Updated
   `features.md` and added the design point to `architecture.md` § Licensing.

### Risks and edge cases

1. **A long name is truncated, not fitted.** The sidebar is a fixed 232px and the
   logo takes 42 of it. `.brand-name` clamps to two lines with the full value in
   a `title` attribute. Covered by the markup, not by an automated test — jsdom
   does not lay out CSS, and the E2E mock's licensee is short.
2. **The fallback branches are unreachable in E2E.** The browser mock always
   reports a valid licence, so only the unit tests exercise the setting and
   app-title fallbacks. An install that loses its licence is not covered
   end-to-end.
3. **`shop_name` is now write-once-then-frozen.** Nothing in the UI can change
   it, so an install seeded with "Électro Ménager" keeps that value forever as
   its fallback. Harmless while a licence is present, but it means the fallback
   name can be stale rather than merely generic.
4. **The window title is still static.** `tauri.conf.json` sets it at build time,
   so it cannot reflect the licence holder; only the in-app brand block does.
5. **RTL was verified by construction, not by eye.** The brand block is a flex
   row with `text-align: start`, and the E2E RTL test passes, but no test asserts
   the mirrored position of the name relative to the logo.

### Recommendations

1. Look at the sidebar with a genuinely long licensee (a 60-character company
   name) in both LTR and RTL before shipping — risk 1 and 5 are the two things
   this pass could not verify automatically.
2. If the shop name ever needs to appear on a printed receipt or export, read it
   from the licence there too rather than from `shop_name`, or the two will
   disagree exactly as they did before this change.

---

## 2026-07-30 — Format-doc audit, and retiring the development key

### Summary

Two requests: validate `docs/license-format.md` against the code, and clean up
the licence handling now that a signing key file exists.

**The audit found six defects in the doc, two of them factual.** Separately, and
more seriously, the key file at `~/secure/paymentschedule-signing.key` was found
to contain the **published development keypair**, not a generated one — see
Issues below.

The public key is now a compile-time requirement. `license.rs` reads it with
`env!`, so a release build without `PAYMENT_SCHEDULE_LICENSE_PUBKEY` does not
compile; the silent fallback to the development key is gone. `build.rs` supplies
the development key to debug builds.

### Test cases

**Executed.** Rust 122 passed, frontend 141 passed, `cargo fmt --check`,
`clippy --all-targets -D warnings` (0), `cargo deny check` (all ok),
`npm run lint`, `npm run build`, `prettier --check`.

| Check                                    | Result                                                                                           |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Release build with **no** key            | Fails to compile, naming the variable and pointing at §7                                         |
| Release build **with** a key             | Compiles                                                                                         |
| `cargo test` with a foreign key exported | **122 passed** — `build.rs` forces the development key in debug, and says so via `cargo:warning` |
| Dev public key identical in all 5 places | Confirmed (build.rs, license.rs, the doc, the CI guard, LicenseFormat.java)                      |
| `option_env!` fallback removed           | Confirmed absent                                                                                 |

### Issues found

1. **Blocker — the "production" signing key is the published development key.**
   `~/secure/paymentschedule-signing.key` holds a seed byte-identical to the one
   printed in `docs/license-format.md` §7. Verified by comparison without printing
   the value. A release built with it would accept licences minted by anyone who
   has read that page. **A real keypair must be generated before shipping.**
2. **`docs/license-format.md` §5 stated the wrong salt length** — "the 31-byte
   string `payment-schedule.machine-id.v1` followed by a single NUL". The string
   is 30 bytes; 31 is the total including the NUL. An implementer following it
   literally would derive a different fingerprint for every machine and see it as
   a signing failure.
3. **§9 omitted `ClockTampered`** — 7 variants in code, 6 documented.
4. **Three claims had gone stale** when enforcement landed: §6 "the validator does
   not enforce it", §9 "how the app reacts … is not implemented", §10 "the
   watermark … is part of the enforcement task". All three are implemented.
5. **§7's openssl recipe could not produce a usable value** — `openssl pkey -text
-noout` prints hex, not the base64url the variable needs. Replaced with the
   `certificate-generation` tool, which the doc had never mentioned.
6. **§10 overstated "the licence file is world-readable"** — `import_license` uses
   `std::fs::copy`, so permissions are inherited. Reworded to "not encrypted".

Found and fixed during the work itself:

7. `DEV_PUBLIC_KEY_B64` became dead code in debug builds once the fallback was
   removed, failing `clippy -D warnings`. Fixed with `cfg!` instead of `#[cfg]`
   so the constant is referenced in every profile.
8. **Exporting the key then running `cargo test` failed 17 tests.** The README now
   tells you to export it for release builds, so hitting this was likely. `build.rs`
   now forces the development key for _all_ debug builds and emits a
   `cargo:warning` when overriding — `cargo test` is hermetic. Nothing is lost:
   a debug build with a production public key is unusable anyway, since minting a
   licence for it needs the production seed.

### Risks and edge cases

1. **The release workflow now requires a repository variable.** `build.yml` gained
   a guard step that fails the job if `vars.PAYMENT_SCHEDULE_LICENSE_PUBKEY` is
   empty **or is the development key**. Set it before the next tag or the release
   build stops — deliberately, but it is a change you must action.
2. **Three copies of the development public key** exist (build.rs, license.rs, the
   doc) plus the CI guard and the Java tool. The test
   `the_documented_dev_seed_matches_the_embedded_public_key` derives it from the
   seed and checks two of them; CI checks a third. Not fully self-checking.
3. **`cargo test` still does not run in CI.** `build.yml` runs `npm test`, clippy
   and an MSRV `cargo check` only, so all 122 Rust tests — every licence test
   included — are unverified on every push.

### Recommendations

1. **Generate a real signing keypair** with
   `java -jar certificate-generation.jar keygen --out <path>`, keep the seed
   offline, and set the repository variable to the printed public key. Treat the
   current file as compromised for production use.
2. Add a `cargo test` job to `build.yml` (risk 3). Say the word and I'll do it.
3. After generating the real key, re-run the end-to-end check: mint against it,
   build a release with the matching public key, and confirm the app reports
   **Active** while a `--dev-key` licence is rejected.

---

## 2026-07-28 (g) — Licence enforcement: making the validator apply

### Summary

Reported: "the app keeps the same behaviour, the licence is not applied."

**Verified and confirmed — and it was not a defect.** The previous entry (f) built
validation only, by agreement; a grep showed nothing outside `license.rs`
referenced it, `lib.rs` registered no command for it, and `src/` had zero hits, so
the app _could not_ behave differently. This entry wires it up.

The gate is in **Rust**, not only the UI. `require_license` refuses 21 of the now
29 commands with `LICENSE_REQUIRED`. A check living only in the renderer would be
decoration — the WebView is the user's.

Distribution, audited mechanically rather than by eye:

- **Gated (21):** 10 client/purchase mutations, `update_installment`,
  `record_payment`, 3 payment reads, `list_impayes`, `list_schedule`,
  `get_dashboard`, `set_logo`, `clear_logo`, `backup_database`.
- **Baseline (4):** `list_clients`, `get_client_detail`, `list_purchases`,
  `get_purchase_detail` — these **degrade** rather than refuse, pinning an
  unlicensed caller to the active scope with no server-side search.
- **Open (2):** `get_settings`, and the new `get_license_status` /
  `import_license` — which must work unlicensed or a licence could never be
  installed.
- **Partial (1):** `update_settings` accepts a language-only patch. Locking the
  language would make the licence screen unreadable for a user who cannot read
  the current one.

Also added: the clock-rollback watermark, `LicenseInfo` as the IPC projection
(dropping `Malformed { reason }`), a licence section in Settings with the machine
fingerprint and import button, sidebar padlocks, and `LicenseRequiredPanel`
rendered from `App.vue` off `route.meta.licensed` — one gate site, not eleven.

### Test cases

**Executed.** Rust: 122 passed (was 107; +15). Frontend unit: 141 passed (was 133;
+8, a new `stores/license.test.ts`). Integration: 190 passed across 8 suites (was
183/7). `cargo fmt --check`, `cargo clippy --all-targets -D warnings` (0 errors),
`cargo deny check` (advisories/bans/licenses/sources ok), `npm run lint`,
`npm run build`, `prettier --check` — all clean.

> Integration was **executed** here, contrary to the usual default, because the
> plan's verification step required proving the existing suites still pass with
> the gate in place. That is a regression check, not new-feature validation.

| Area                | Covered                                                                                                                                                                                                                                                                               |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Gate (Rust)         | Every non-`Valid` verdict — missing, invalid signature, malformed, expired, machine mismatch, clock tampered — refuses with exactly `LICENSE_REQUIRED`; `Valid` admits.                                                                                                               |
| Baseline (Rust)     | Client and purchase reads still work with no licence; an unlicensed caller asking for `Archived`/`All` silently gets `Active` for both scope enums.                                                                                                                                   |
| Settings (Rust)     | A language-only patch is accepted unlicensed; anything carrying `currencyCode`, `shopName` or `alertSoonDays` is refused. `is_language_only` destructures exhaustively, so a new `SettingsPatch` field is a compile error until someone decides whether it is licensed.               |
| Clock guard (Rust)  | A licence that expired in 2027 reads `Valid` again with the clock wound back to 2026 — and is then refused as `ClockTampered` by the watermark. Verdicts that do not depend on the date (invalid signature, missing, malformed) pass through untouched. Watermark only ever advances. |
| Wire type (Rust)    | `LicenseInfo` never serializes `Malformed.reason`; withholds the licence body for unverified verdicts; emits ISO date strings; all 7 status tags stable and distinct; `ClockTampered` is **not** licensed.                                                                            |
| Watermark isolation | Round-trips through the `setting` table and does **not** appear in the JSON `get_settings` sends to the renderer.                                                                                                                                                                     |
| Store (frontend)    | Unlicensed before the first check; only `"valid"` unlocks; all 6 other verdicts lock; expiry date carried; fingerprint exposed; **fails closed when the check itself throws**.                                                                                                        |
| Import (frontend)   | Unlicensed → import → licensed with no restart; a refused file leaves the previous verdict intact.                                                                                                                                                                                    |
| Error contract      | `LICENSE_REQUIRED` and `INVALID_LICENSE:{status}` resolve to localized prose in all three locales, never a raw code or a bare `errors.*` key.                                                                                                                                         |

Locale parity re-verified programmatically: **424 keys identical across ar/fr/en**.

### Issues found

Two real defects, both found during the review pass and both fixed:

1. **Blocker — the unlicensed baseline was broken by the gate.** `/achats/:id` and
   `/clients/:id` are ungated routes, but their `load()` calls
   `listPaymentsForPurchase` / `listPaymentsForClient`, which **are** gated. Worse,
   both wrap the whole load in `try { … } catch { notFound.value = true }`, so an
   unlicensed user opening a purchase that exists would have seen **"page not
   found"**. Fixed: the payment fetch is skipped when unlicensed and the history
   section renders a licence notice instead. This is exactly the kind of thing the
   command-by-command audit was for — the route was open, its data was not.
2. **Should-fix — the scope tabs lied.** Because `list_clients`/`list_purchases`
   _degrade_ rather than refuse, clicking "Archivés" unlicensed returned **active**
   rows under an "Archived" heading. Fixed: non-active scope tabs are disabled
   unlicensed, with a tooltip.

Checked and clean: api/mock parity (both new methods mirrored), integer money
(untouched), `finance.ts` ↔ `db.rs` parity (untouched), transactional integrity
(the gate returns before any write; no new multi-write paths), resource cleanup
(two `OnceLock`s and one `RwLock`, all bounded; no listeners added), logging
(status tags and licence ids only — `licensee` is never logged, per the PII rule).

### Risks and edge cases

1. **Filters and sorting are not enforceable.** `useSort.ts` reorders rows already
   in the browser; `ListFilterBar.vue` filters in the parent. The backend never
   sees either, so disabling them communicates the boundary rather than enforcing
   it. Anyone with devtools can re-enable sorting. `scope` is the one real
   server-side filter and is genuinely degraded. **This is a design limit of the
   agreed baseline, not a bug.**
2. **The watermark shares the database it defends.** It stops a clock change; it
   does not stop restoring an older `.db`. Nothing in the app signs the database.
3. **Mutation buttons still appear unlicensed** on the Clients and Achats pages
   and error with a localized refusal when pressed. Correct and safe, but a
   further pass could disable them for a cleaner experience.
4. **`import_license` is callable with any path** and reports `missing` vs
   `malformed`, a weak file-existence oracle for the renderer. Same property
   `set_logo` already has; it copies only after a signature verifies, so it cannot
   exfiltrate content.
5. **The development key is still the compiled-in default** — unchanged from entry
   (f), and still the most important thing to fix before shipping.

### Recommendations

1. **Before shipping:** set `PAYMENT_SCHEDULE_LICENSE_PUBKEY` in the release build
   and make it a required variable, so a build cannot silently fall back to the
   published development key.
2. Run `npm run test:e2e` before release — the E2E suite drives the app against
   the mock, which is licensed by default, so it should be unaffected; that is
   worth confirming rather than assuming. No E2E scenario was added for the gate.
3. Consider disabling (not just refusing) the mutation buttons on the baseline
   pages, per risk 3.
4. `validate_installed` remains the one function with no automated coverage — it
   is the thin `AppHandle` wrapper the `*_impl` split exists to avoid. The manual
   steps below exercise it.

### Manual verification still outstanding

Not run — this pass covers the automated suites. The four-step check in the plan
(launch unlicensed → import a minted licence → expire it → wind the clock back)
needs a desktop session with `npm run tauri dev`.

---

## 2026-07-28 (f) — Offline Ed25519 licence validation

### Summary

New `src-tauri/src/license.rs`: reads a signed licence from `$APPDATA/license.json`,
verifies an Ed25519 signature against a public key compiled into the binary, checks
machine binding and expiry, and returns a typed `LicenseStatus`. No network, no
licence server.

**Validation only.** Nothing calls the module — there is no Tauri command, no
gateway method, no mock method and no locale key. Feature gating, lockout and
read-only fallback are a separate task by design.

Format is a self-contained envelope (`version`, `payload`, `signature`), both
strings base64url without padding. The signature covers
`b"payment-schedule-license.v1." || payload_b64`, i.e. the base64 text exactly as
it appears in the file. Two consequences: JSON canonicalization is a non-issue,
and the signature is verified **before** any untrusted JSON is parsed.

First cryptography in the tree: `ed25519-dalek`, `sha2` (pinned to the 0.10
already present transitively), `base64`, `machine-uid`. `cargo deny check` passes
unmodified — `advisories ok, bans ok, licenses ok, sources ok`.

### Test cases

**Rust unit — 28 new tests in `license.rs`, all executed, all passing**
(`cargo test`: 106 passed total, up from 79).

| Area           | Covered                                                                                                                                                                                                                                                                                                          |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Signature      | Valid licence round-trips its fields; **every byte of the payload flipped in turn** is rejected; signature from an untrusted key rejected; signature made without the domain prefix rejected; embedded public key is a usable Ed25519 point.                                                                     |
| Check order    | An unsigned, unparseable payload reports `InvalidSignature` — **not** `Malformed` — pinning that untrusted JSON is never parsed before the signature check.                                                                                                                                                      |
| Malformed      | Not JSON; empty file; missing `signature` field; envelope `version: 2`; non-base64 signature; 63-byte signature; non-ISO date; `issuedAt` after `expiresAt`; payload missing required fields.                                                                                                                    |
| Expiry         | Inclusive boundary (expires today → still `Valid`); day after → `Expired`, carrying the licence and the date.                                                                                                                                                                                                    |
| Machine        | Bound to this machine → `Valid`; bound elsewhere → `MachineMismatch` carrying both fingerprints; `machineId: null` valid anywhere; fingerprint unavailable → `MachineMismatch { local: None }`; comparison ignores hex case; **wrong machine reported ahead of expiry**.                                         |
| Fingerprint    | Normalization absorbs trailing newline, surrounding spaces, `{}` braces and upper case (all one machine); output is 64 lower-case hex chars; derivation is the salted SHA-256 and specifically _not_ a bare hash.                                                                                                |
| Forward compat | Unknown payload fields ignored, not rejected; absent `features` defaults to `[]`.                                                                                                                                                                                                                                |
| Filesystem     | Missing file → `Missing` (not an error); real file on disk validates; oversized file rejected on metadata without being read; a directory at the path → `Malformed`, not a crash.                                                                                                                                |
| Docs parity    | The worked example published in `docs/license-format.md` §8 — minted by an external **Python** signer — is embedded verbatim and asserted to validate. This is the only cross-language check that a third-party signer can interoperate, and it pins prefix, alphabet, salt and fingerprint derivation together. |

**Other gates, all executed:** `cargo fmt --check`, `cargo clippy --all-targets -D warnings`,
`cargo deny check`, and a `debug-assertions=off` build to compile the
release-only dev-key warning branch (a `cfg(not(debug_assertions))` block that a
normal debug build never type-checks). Frontend unaffected and confirmed:
`npm test` 133 passed, `npm run lint` clean, `npm run build` succeeded.

**Integration / E2E — none written, deliberately.** Those suites (`tests/integration/**`,
`tests/e2e/run.mjs`) drive the `src/api` facade against the browser mock. This
change adds no command, no gateway method and no UI, so there is no surface for
them to exercise; writing a suite here would test nothing. They become relevant
with the enforcement task.

### Issues found

One, in the new test code, found during Code Review and fixed:

- `a_single_flipped_payload_byte_is_rejected` computed a `position` variable and
  asserted a tautology against it — leftover scaffolding proving nothing. Replaced
  with a loop that corrupts **every** byte position in the payload individually and
  asserts each is rejected (~300 cases; suite runtime went 0.03s → 1.21s, which is
  the work actually happening).

No issues found in production code. No `unwrap`/`expect`/`panic!`/slice indexing
on file-derived data anywhere outside `#[cfg(test)]` — verified by grep, and load-
bearing because `panic = "abort"` in the release profile would turn a panic on a
hostile licence file into a crash of the whole app.

### Risks and edge cases

Flagged explicitly, including ones not covered by tests. All are documented in
`docs/license-format.md` §10 rather than left implicit:

1. **The system clock is trusted.** Setting the clock back makes an expired licence
   validate again. Inherent to offline validation with no stored state; the fix is a
   monotonic watermark, which is stateful enforcement and belongs to the next task.
   _Not covered by tests — it is a design limitation, not a defect._
2. **Binary patching.** The public key is a constant in the executable; anyone able
   to replace it can sign their own licences. Signature verification raises the cost
   of casual copying, it does not make the app tamper-proof.
3. **Cloned machine identifiers.** Some virtualised/imaged environments copy
   `/etc/machine-id` across hosts, so one bound licence would validate on every
   clone. Conversely a reinstall or motherboard swap changes it and needs a reissue.
4. **The development key is the compiled-in default.** Any build that forgets
   `PAYMENT_SCHEDULE_LICENSE_PUBKEY` trusts a keypair whose seed is published in the
   module docs. A release build in that state logs a warning; it cannot be a hard
   failure without crashing the app under `panic = "abort"`. **Recommend making the
   env var mandatory in the release CI job.**
5. **`machineId` is unverifiable at issue time.** The vendor takes the customer's
   word for the fingerprint they report. A customer could supply a colleague's.
6. **`validate_bytes` is `pub` and uncapped.** The 64 KiB cap lives in
   `validate_file`; a caller passing an enormous slice directly would allocate a
   copy of the payload. Not reachable today (no caller) and bounded in the real path.

### Recommendations

1. **Before shipping:** generate a production keypair, keep the seed offline, and
   set `PAYMENT_SCHEDULE_LICENSE_PUBKEY` in the release build. Make it a required
   variable so a build cannot silently fall back to the development key.
2. **The enforcement task** should decide the reaction per status, add the
   monotonic-clock watermark, surface `license::machine_fingerprint()` in Settings
   (support cannot issue a bound licence otherwise), and — when `LicenseStatus`
   crosses IPC — ensure `Malformed { reason }` collapses to an opaque code the way
   `AppError::Internal` does. That constraint is recorded in the module docs.
3. **The unlicensed baseline** is recorded in `docs/license-format.md` §6: reading
   clients and purchases, list and detail, without filters and without sorting,
   requires no licence. Everything else is licensed.
4. Integration/E2E coverage should be written with the enforcement task, once there
   is a command and a UI for them to drive.

---

## 2026-07-28 (e) — Bug: tables paint outside their card when the window is not maximized

### Summary

Bug fix, reported against the dashboard: at a non-maximized window (~1435px) the
**Status** column of _Recent purchases_ rendered past the card's right border, its
badges cut off mid-pill.

Root cause was structural, not local to that card. No table in the app had a scroll
container — a repo-wide grep for `overflow-x` / `table-layout` in `src/` returned
nothing — and `.card` sets no `overflow`. So any table wider than its card simply
painted through the border, and because `.app-content` sets `overflow-y: auto`
(which makes `overflow-x` compute to `auto`), the spill became a **page-wide**
horizontal scrollbar rather than a local one. All 12 table call sites shared the
defect; the dashboard was only where it showed first, since its card lives in a
`minmax(0, 1.8fr)` grid track (723px of usable width) while the table needed 751px.

Two changes:

- **`.table-scroll`** in `src/style.css` — a shared `overflow-x: auto` box, now
  wrapping every table (11 views/components, 12 sites). Overflow is contained in
  the card; the page never scrolls sideways. It also rounds its bottom corners, but
  only via `:last-child`, so mid-card tables (`PurchaseDetailCard`, the paginated
  lists) keep square corners and a highlighted `.is-late` last row is not clipped.
- **`RecentPurchasesCard`** — tighter gutters (`padding-inline: 10px` vs the global
  16px, outer edges keeping the card inset) and the product column made the flexible
  one (`max-width: 0; width: 100%`) instead of a fixed 170px cap. Measured: the table
  now needs 723px in a 723px card — it fits without a scrollbar and the product
  column absorbs slack as the window widens. The full label moved to a `title`
  attribute so truncation costs no information.

No new user-facing string, so the three locale files are untouched. Presentation
only: no Rust, no api gateway/mock change, no installment math.

### Test cases

**Unit — run, passing.** New `src/components/dashboard/RecentPurchasesCard.test.ts`
(4 cases; suite now 8 files / 133 tests): the table is inside `.table-scroll`; the
product cell is the `.ellipsis` column and carries the full label in `title`; one row
per purchase; the empty state renders with neither table nor wrapper.

**E2E — run at the user's request, 47/47 passing** (44 existing + 3 new, no
regressions). `tests/e2e/run.mjs`, three new scenarios under "table layout":

- _dashboard: the recent-purchases table stays inside its card_ — at the suite's
  1440×900 context the wrapper exists, does not overflow the card, does **not**
  scroll, the page has no horizontal scroll, and the status badge (the reported
  symptom) is inside the card.
- _dashboard: a narrow window scrolls the table inside the card, not the page_ — at
  900px the wrapper still fits the card, now scrolls itself, and the page does not.
- _every list page keeps its table inside the card at a narrow width_ — 1000px across
  `/achats`, `/clients`, `/paiements`, `/echeances`, `/alertes`, `/impayes`: no
  `.table-scroll` extends past its card, no page-level horizontal scroll.

**Manual verification (headless Chromium, mock backend).** Measured geometry rather
than eyeballed: at 1435px the table is 984px vs a card edge at 985px with no
scrollbar (was 751px of table in a 723px box); at 1100px it fits with the product
column at 194px; at 820px it scrolls inside the card with the page still fixed. In
Arabic the layout mirrors and the product truncates from the logical end. All six
list routes checked at 1435px and 1000px: zero tables past their card, zero pages
scrolling sideways.

Gates run and passing: `npm test` (133 unit tests, 8 files), `npm run lint` (clean,
incl. `eslint-plugin-security`), `npm run build` (`vue-tsc --noEmit` + Vite build),
`prettier --check` on every touched file.

### Issues found

None beyond the reported defect. One pre-existing behaviour was confirmed unchanged
rather than fixed — see below.

### Edge cases and risks not covered by the automated tests

- **The featured purchase's installment table** (`PurchaseDetailCard`, 7 columns
  incl. an action cell) needs 868px in the same 723px card, so it is contained but
  still scrolls at every window size. That is a legitimate use of the scroll box, not
  a regression — before this change it was spilling outside the card instead. Tighter
  gutters would save ~84px and still not make it fit; dropping or narrowing a column
  is the only real fix, and it was out of scope here.
- **Cells wrap before they scroll.** With `width: 100%` and auto layout, a squeezed
  table wraps whatever text can wrap (on `/achats` at 1000px, "A-000007" breaks
  across two lines) and only scrolls once the un-wrappable minimum is exceeded. This
  is pre-existing behaviour, now merely visible without the spill. Adding
  `white-space: nowrap` to the reference column would read better; not done here.
- **`overflow-x: auto` makes `overflow-y` compute to `auto`**, so a popover rendered
  inside a table would be clipped. Verified safe today — no table in `src/` contains
  a dropdown, menu or tooltip; row actions are plain buttons and modals render at the
  view root. A future in-row menu must render in a portal, not inside the cell.
- **Scrollbar appearance is platform-dependent.** Verified on Linux/Chromium; the
  overlay scrollbars on macOS and the WebView2 rendering on Windows were not checked.
- The RTL pass was manual. The new E2E scenarios run in French only.

### Recommendations

- Use `.table-scroll` for any table added from here on; it is the reason a wide table
  can no longer break the page layout.
- Consider revisiting the installment table's action column so the featured-purchase
  card can fit without a permanent scrollbar.

---

## 2026-07-28 (d) — Header notification bell navigates to the alerts page

### Summary

Bug fix. The bell in `AppHeader` rendered a live red badge sourced from
`stats.overdueInstallments` but was a bare `<button>` with no click handler — it
advertised a destination it never reached.

The bell is now a `<RouterLink>` to `{ name: "alertes" }`, matching the project's
convention of `RouterLink` for static nav destinations (`AppSidebar.vue`,
`DueAlertsCard.vue`). It lands on the Alerts page in its default **Toutes** tab —
no pre-applied filter — because the sidebar's own bell entry behaves the same way.

Two supporting changes: the hardcoded `aria-label="notifications"` became
`t("header.notifications")`, added to all three locale files; and `.icon-btn` gained
`text-decoration: none` so the now-anchor bell doesn't pick up the global
`a:hover { text-decoration: underline }` from `src/style.css`.

No new route, no new view, no Tauri command, no change to the api gateway/mock pair
or to the installment math.

### Test cases — E2E written, NOT run (awaiting confirmation)

Per the QA workflow, E2E is not executed automatically. Run it with `npm run test:e2e`.

`tests/e2e/run.mjs` — new scenario `"header bell navigates to the alerts page"`:

- The bell badge renders on first load with a count ≥ 1 from the seeded mock data.
- Clicking the bell pushes `window.location.pathname` to `/alertes`.
- The header title updates to `NAV.alertes` ("Alertes") via the existing `NAV_KEY` map.
- The Alerts page opens on the **Toutes** tab — the click does not pre-filter to overdue.
- The badge survives the navigation and is still visible on the destination page.

Gates run and passing for this change: `npm run lint` (clean, including
`eslint-plugin-security`), `npm test` (129 unit tests, 7 files), `npm run build`
(`vue-tsc --noEmit` typecheck + Vite build).

### Issues found

None beyond the reported defect, which is fixed.

### Edge cases and risks not covered by the automated test

- **Zero-alert state.** With no overdue installments the badge is hidden (`v-if`),
  but the bell remains clickable and still routes to an empty Alerts page. The
  seeded mock always has overdue rows, so the E2E case cannot reach this branch;
  it is exercised manually.
- **Arabic / RTL.** `.bell-badge` uses the logical `inset-inline-end`, so it should
  mirror without extra work, and the label now resolves to "التنبيهات". Not asserted
  by the new test — the existing RTL coverage in `run.mjs` targets the not-found
  page only. Verify visually when switching locale.
- **Clicking the bell while already on `/alertes`.** vue-router treats this as a
  duplicate navigation and no-ops; it does not reset the page's active tab or
  filters. Intentional, but worth knowing it is not a "refresh" affordance.
- **`RouterLink` adds `router-link-active`** to the bell when on `/alertes`. No
  style is defined for it in `AppHeader`'s scoped CSS, so there is no visual
  change today — a future rule on that class would affect the bell.

### Recommendations

- Run `npm run test:e2e` to confirm the new scenario passes before shipping.
- The `.user` block in `AppHeader` (avatar + "Admin" + chevron) has the same defect:
  `cursor: pointer` with no handler and no menu. Out of scope here; worth either
  wiring to Settings or dropping the pointer cue.

---

## 2026-07-28 (c) — Installment management: inverted rules, remaining column, ledger corrections

### Summary

A revision of the (b) pass. Two of its rules were the wrong way round, and the
table was missing the number a shopkeeper looks at first.

The editor now splits cleanly in two, with each half's rule blind to the other's:

- **The schedule** (installment amount, due date) is editable until the tranche
  settles, after which it is history (`AMOUNT_LOCKED`, `DUE_DATE_LOCKED`).
  Nothing about the neighbouring tranches gates it — this is the **inversion** of
  the previous pass, where the amount was gated on the previous tranche.
- **The money** (paid amount, payment date, note) is editable only once tranche
  `N-1` is settled (`PREVIOUS_UNPAID:{index}`) — cash is collected in order.
  Nothing about this tranche's own status gates it. Also an inversion: these
  fields were previously gated on the tranche being settled.

Alongside: a **Remaining** column (`amount − paidAmount`) in the installment
table; the paid amount kept out of the list but editable in the form; the row
action relabelled **"Update payment"**; and `PaymentModal` merged away, so one
form is the single editor of an installment.

Two decisions carry the money safety, both flagged as spec/model
inconsistencies before implementing:

1. **`paid_amount` is a denormalised cache of the `payment` ledger**, not a plain
   field. The dashboard's "Amount collected" is `SUM(payment.amount)`; every
   other paid/remaining/outstanding figure is `SUM(installment.paid_amount)`.
   Moving one alone would make that tile contradict every other total. So a
   changed paid amount now writes a **correction entry** — one payment row for
   the difference, carrying the date and note, negative when the figure comes
   down.
2. **Ordering.** The spec states installments are ordered chronologically by due
   date; the schema orders by `idx`. With due dates editable the two could
   diverge. Resolved by **clamping a due date to `[prev, next]`**
   (`DUE_DATE_OUT_OF_ORDER`), which makes position order and chronological order
   provably identical, so "the previous installment" means one thing either way.

### Test cases run

| Suite                                    | Result                    |
| ---------------------------------------- | ------------------------- |
| Rust `cargo test`                        | **79 passed** (was 72)    |
| TS unit (`npm test`)                     | 129 passed                |
| Integration (`npm run test:integration`) | **183 passed** (was 168)  |
| E2E (`npm run test:e2e`)                 | **43/43 passed** (was 42) |

**Gates:** eslint · vue-tsc · vite build · cargo fmt · cargo clippy
(`--all-targets -D warnings`) — all clean.

New coverage:

- **A ledger invariant, asserted rather than assumed.** `MoneySnapshot` gained
  `assert_ledger_matches_installments`, checking
  `SUM(payment.amount) == SUM(installment.paid_amount)` (and that no row owes
  less than it collected). Every Rust test that touches collected money calls it;
  the integration suite's `expectConsistent` does the same per purchase. This is
  the single assertion that stops the dashboard's "Amount collected" drifting
  away from every other total.
- **Rust (rewritten + 7 net new).** Both inverted gates, `AMOUNT_LOCKED`,
  `PAID_ABOVE_AMOUNT`, the due-date interval at both bounds and the unbounded
  outer tranches, correction entries in both directions, the zero-out reversal,
  payment-date re-dating, note-only amendment, `NO_PAYMENT_TO_DATE`, and the
  combined amount+paid-amount edit that must _not_ trip a false `BELOW_PAID`.
- **Integration (rewritten, 20 → 24 cases)**, including the dashboard's collected
  total moving up _and back down_ with a paid-amount edit.
- **E2E (5 scenarios).** Recording a payment through the merged modal and seeing
  the correction entry in the history; the rebalance preview and save; a tranche
  with an unpaid predecessor locking _only_ its money fields; a paid tranche
  locking its schedule and warning before a change; the remaining column.
- **Manual, driven with Playwright against the built app**: the full flow, the
  correction entry appearing as `-100 TND` in the payment history, the
  confirmation dialog, the success toast, and the Arabic RTL rendering.

### Issues found

1. **A note-only edit was silently dropped.** The re-dating branch was gated on a
   payment date being present, so typing a note without touching the amount or
   the date wrote nothing and still reported success. Fixed in both backends: a
   date _or_ a note amends the row's latest ledger entry, and either with no
   entry at all is refused with `NO_PAYMENT_TO_DATE` rather than discarded.
   Covered by new Rust and integration tests.
2. **The payment-date picker stayed enabled with nothing collected**, where the
   frontend then declined to send it — a silent no-op. It is now disabled unless
   the installment has a payment or this edit is creating one, which matches the
   backend guard instead of quietly working around it.
3. **Both lock notes rendered against the wrong section.** "Installment 2 has to
   be settled…" sat between the due-date field and the money legend, reading as
   if it explained the due date. Moved inside their own `<fieldset>`, which is
   also what makes one `disabled` cover a whole half of the form rather than each
   input opting in.
4. **The guards had to key off resolved values, not stored ones.** Lowering the
   amount and the collected figure together is a request that resolves its own
   conflict; comparing against the stored `paid_amount` refused it. `finalPaid`
   and `finalAmount` are computed first, and `rebalance_amounts` is called with
   `finalPaid` at the edited position for the same reason.
5. **The amount column was the only money column in the app using `fmt.number`.**
   Adding a `fmt.money` Remaining column beside it would have read as "400" next
   to "400 TND"; switched to `fmt.money`.
6. **`errors.paidDateNotPaid` was left behind** by the rule inversion — the code
   no longer exists. Removed from all three locales; the error-contract suite's
   code list was updated in the same pass.

### Recommendations

- **A downward correction shows as a negative line in the Paiements log** and
  inside its amount-range filter. This is the honest reading — the money came
  back — but the log has no visual treatment distinguishing a correction from a
  collection. Worth a badge or a filter if operators find it confusing.
- **`record_payment` now has no caller in `src/`.** It remains the incremental
  payment path, is fully tested, and both test suites use it to set up state, so
  it was deliberately left in place rather than deleted alongside `PaymentModal`.
  If it is not going to come back, the gateway entry and the command are dead
  surface worth removing on purpose.
- The paid amount is **absolute**, so recording a second collection means typing
  the running total rather than the increment. That is the direct consequence of
  merging the two modals; if the daily flow suffers, the fix is a separate
  "add a payment" action rather than reverting the merge.
- The confirmation popup fires only for a **fully paid** installment, the literal
  reading of the requirement. A partially-paid one is edited without it, even
  though that also touches collected money.
- `installment.idx` still has no `UNIQUE(purchase_id, idx)` constraint backing
  the ordering the sequential rule depends on. Pre-existing, and unchanged here.

---

## 2026-07-28 (b) — Editing a single installment

### Summary

`update_purchase` goes hard-locked at the first payment (`PURCHASE_HAS_PAYMENTS`)
because saving there deletes and reinserts the installment rows, and those rows
own the payments through an `ON DELETE CASCADE`. That left a real gap: pushing
one due date back a week, or re-cutting the tranches a client renegotiated, only
ever happens _after_ payments have started. `update_installment` fills it by
updating rows in place — nothing is regenerated, so the payment ledger is never
at risk and the command stays available for the whole life of a live purchase.

Three rules were specified and implemented:

1. **The due date is editable** — until the installment is settled, after which
   it is history (`DUE_DATE_LOCKED`). In exchange the **payment date** becomes
   editable on a settled installment (`PAID_DATE_NOT_PAID`), capped at today
   (`FUTURE_PAID_DATE`).
2. **The amount is gated on the previous installment** being fully paid
   (`PREVIOUS_UNPAID:{index}`).
3. **The amount is editable even on a collected installment, behind a
   confirmation popup**, and `0` is a legal value.

Two decisions carry the money safety. **`purchase.total_price` is never
written**: a changed amount is absorbed by the other unsettled installments
(`rebalance_amounts` / `rebalanceAmounts`, later-first with a backwards
fallback), so `SUM(amount) == total_price` holds by construction rather than by
a check, and `NO_REBALANCE_ROOM` is returned when no absorber set can take it.
And **the amount floors at `paid_amount`** (`BELOW_PAID:{paid}`) — so 0 is
reachable on an untouched row but never below what was collected. That is the
`OVERPAYMENT` invariant approached from the other side:
`SUM(i.amount - i.paid_amount)` feeds the outstanding and overdue aggregates, and
one negative row cancels out another client's real debt.

`status` is never written (it is derived), but `paid_date` is: `sync_paid_date`
re-derives it for every row whose amount moved, so zeroing an untouched tranche
reads as paid with no date, and raising a settled one puts it back in debt and
clears the date it no longer has.

### Test cases run

| Suite                                    | Result                    |
| ---------------------------------------- | ------------------------- |
| Rust `cargo test`                        | **72 passed** (was 57)    |
| TS unit (`npm test`)                     | **129 passed** (was 110)  |
| Integration (`npm run test:integration`) | **168 passed** (was 145)  |
| E2E (`npm run test:e2e`)                 | **42/42 passed** (was 39) |

**Gates:** eslint · vue-tsc · vite build · cargo fmt · cargo clippy
(`--all-targets -D warnings`) — all clean.

New coverage:

- **Shared math.** `rebalanceAmounts` / `rebalance_amounts` as a pure function in
  both languages, with 10 cases added to `tests/fixtures/finance-parity.json` so
  the two cannot drift. Every non-null fixture case is additionally asserted to
  preserve the total and respect each row's `paidAmount` floor.
- **Rust (15 cases).** Later-first rebalance, backwards fallback, the
  previous-tranche gate, the `paid_amount` floor from both sides, `paid_date`
  clearing on un-settle and filling on settle, the due-date/payment-date swap,
  archived refusal, bad arguments, and a rollback case proving a refused edit
  writes nothing (compared against the seven-figure `MoneySnapshot`).
- **Integration (20 cases).** The same behaviour through the real `api` facade
  against the browser mock, with an `expectConsistent` helper re-asserting both
  invariants after every successful edit, plus two cases checking the dashboard
  and the schedule follow an edit.
- **E2E (3 scenarios).** The rebalance preview and save on the purchase page, the
  locked amount field with its on-screen reason, and the below-collected refusal
  plus the confirmation dialog.
- **Manual, driven with Playwright against the built app**: the full edit flow,
  the live rebalance preview, the confirmation dialog, and the Arabic RTL
  rendering of the modal.

### Issues found

1. **`NO_REBALANCE_ROOM` was going to carry a `{max}` parameter, and could not.**
   The obvious ceiling — everything the other rows could give up — is not
   actually reachable, because the pool is re-split _evenly_ rather than to each
   row's floor. With `[200,200,200]` and 50 collected on the second, the naive
   ceiling of 550 is refused: an even split of the remaining pool lands both
   absorbers at 25, under that 50. Shipping the parameter would have told the
   user a number that fails when they type it. Dropped it; the code is now
   param-less and the modal's live preview shows the refusal as the amount is
   typed, which is better than a number in a toast anyway.
2. **The date rules had to key off the state the row _ends_ in, not the one it
   starts in.** Raising a settled installment's amount un-settles it, so locking
   its due date against the pre-edit state would refuse a due-date move that is
   legitimate by the time the edit lands. `final_amount` is computed from the
   resolved rebalance before either date guard runs;
   `un_settling_a_tranche_unlocks_its_due_date_in_the_same_edit` pins it.
3. **"Absorb backwards" is narrower than it looks.** Reaching the last tranche at
   all requires the one before it to be settled (rule 2), so the backwards
   fallback only has room while an _earlier_ tranche is still open — a client who
   paid out of order. In the ordinary case where tranches are paid in sequence,
   editing the final tranche is refused with `NO_REBALANCE_ROOM`. This is a
   direct consequence of holding the total fixed and is working as specified, but
   it is the most likely source of a "why can't I edit this?" question.
4. **The `→` in the rebalance preview is not auto-mirrored by bidi.** U+2192 kept
   pointing at the _old_ value in Arabic. Fixed with the project's existing
   `.icon-flip` convention plus `display: inline-block` (a transform is a no-op
   on an inline box).
5. **The error-contract suite's `CODES` list was already incomplete.**
   `PURCHASE_HAS_PAYMENTS`, `PURCHASE_ARCHIVED` and `PURCHASE_NOT_ARCHIVED` were
   in `error.rs` but not in the test that exists to prove every code resolves to
   a localized sentence. Added alongside the six new ones. The list is hand-kept,
   so it will drift again.
6. **The purchase-detail page offered actions an archived purchase always
   refuses.** The card rendered a "Record" button on every unpaid tranche
   regardless of `archivedAt`, so the only possible outcome was an error toast
   from `PURCHASE_ARCHIVED`. `canPay` now checks it, and the new Edit action does
   the same.
7. **The detail page's action button was labelled "Edit" but opened the _payment_
   modal.** With a real edit action alongside it there would have been two
   buttons labelled "Edit", so the pay button is now "Record"
   (`dashboard.detail.register`) on both the dashboard and the detail page.

### Recommendations

- **Derive the error-contract `CODES` list instead of hand-writing it** (issue
  5). Parsing the `pub const` block in `db.rs`, or the doc table in `error.rs`,
  would make an unlocalized code a test failure by construction rather than by
  someone remembering.
- **`errors.invalidAmount` reads "must be greater than zero"**, which is right
  for `record_payment` but imprecise for an edit, where 0 is legal and the code
  only fires on a negative. Unreachable through the UI (the field has `min="0"`
  and its own message), but worth splitting if a second caller ever needs it.
- If issue 3 turns out to bite in practice, the fix is the alternative that was
  considered and not taken: let an edit with no absorber move
  `purchase.total_price` by the delta. That is a deliberate policy change, not a
  bug fix — it would make this the one command that can change what a client owes
  in total.
- `installment.idx` still has no `UNIQUE(purchase_id, idx)` constraint backing
  it; the ordering this command relies on comes from `insert_installments` alone.
  Pre-existing, and unchanged here.

---

## 2026-07-28 — Editing and archiving purchases

### Summary

Purchases were write-once: no edit, and `delete_purchase` existed in Rust, the
gateway and the mock but had **zero callers in `src/**`** — the Achats table had
no action column at all. Both are now built.

**Editing.** The product label is always editable. Everything the schedule is
derived from — total, installment count, interval, and the purchase date that
anchors it — locks once a payment is recorded, because applying a change
regenerates the installment rows and those rows own the payments through an
`ON DELETE CASCADE`.

**Archiving replaces deleting**, and the money rule is the _inverse_ of the
client archive: an archived client is settled so the totals do not move, whereas
an archived purchase must **leave every total** — a removed purchase is not still
owed. Nine read models gained an `archived_at` filter. A permanent delete is
offered only inside the Archivés tab and refuses anything not already archived.

The whole thing rests on one invariant: **an archived purchase carries zero
payments.** `archive_purchase` refuses once a payment exists and `record_payment`
refuses an archived purchase; there is no delete-payment command, so it cannot be
worked around. That is what lets `total_collected` skip the filter instead of
joining payment → installment → purchase on the app's hottest aggregate.

### Test cases run

| Suite                                    | Result                    |
| ---------------------------------------- | ------------------------- |
| Rust `cargo test`                        | **52 passed** (was 37)    |
| TS unit (`npm test`)                     | 110 passed, 7 files       |
| Integration (`npm run test:integration`) | **125 passed** (was 108)  |
| E2E (`npm run test:e2e`)                 | **39/39 passed** (was 35) |

**Gates:** eslint · vue-tsc · vite build · prettier · cargo fmt · cargo clippy
(`--all-targets -D warnings`) — all clean.

New coverage: 15 Rust cases (edit guards, archive/restore/delete guards, the
zero-payments invariant from both directions, `list_purchases` scope, the m0003
upgrade path, and `archiving_removes_the_purchase_from_every_money_view` which
snapshots all seven money figures at once); a new 17-case
`purchase-archive.integration.test.ts`; and four E2E scenarios (label edit,
locked schedule fields, blocked archive, and the archive → dashboard → restore
round trip).

### Issues found

1. **`listPurchases(search)` silently became `listPurchases(scope)`.** Adding the
   scope as the _first_ parameter meant the one existing caller passing a bare
   search string — `purchase-lifecycle.integration.test.ts` — bound `"A-000009"`
   to `scope`, which the mock's filter treated as "not archived" and returned all
   9 rows. Caught by the integration suite, not the typechecker: **`npm run build`
   typechecks `src/` only, not `tests/`.** Worth knowing that gateway signature
   changes are unverified against the test suites until they actually run.
2. **The editor always sends the rows it is displaying**, so a label-only edit
   arrives carrying an installment list identical to the stored one. Treating the
   mere presence of `input.installments` as a reschedule would have locked the
   label behind the payment guard. `schedule_changed` compares the _resolved_
   schedule against what is stored instead; `resending_the_unchanged_schedule_is_not_a_reschedule`
   pins it.
3. **`DatePicker` had no `disabled` prop**, so the purchase date could not be
   locked with the other schedule fields. Added one (trigger disabled, clear
   cross hidden) rather than faking it with a wrapper.
4. **Three dashboard aggregates never mention `purchase`.** `total_outstanding`,
   `overdue_count` and `upcoming_count` query `installment` alone, so they could
   not simply gain a `WHERE`; each carries an `EXISTS` subquery. This was the
   sharpest hazard in the change — missing one leaves a headline figure
   disagreeing with the list it links to, with nothing failing loudly.
5. **`list_clients_impl`'s purchase filter had to go in the `LEFT JOIN … ON`
   clause**, not the `WHERE`. In the `WHERE` it degrades the outer join into an
   inner one and drops every client with no live purchase.
   `list_clients_keeps_clients_with_no_purchases_under_every_scope` already
   existed for exactly this and would have caught it.

### Recommendations

- **Typecheck the test suites.** Issue 1 was a type error that no gate could see.
  A `vue-tsc` pass over `tests/` (or including them in the build tsconfig) would
  turn that class of break into a compile failure.
- The payment ledger (`list_payments_*`) and `total_collected` are deliberately
  unfiltered, each with a comment saying why. If the zero-payments invariant is
  ever relaxed, those four queries are what breaks — and silently.
- Manual passes still outstanding: the Arabic RTL rendering of the new Achats
  action column and scope tabs, and one launch against a `user_version = 2`
  database to exercise `m0003` on disk.
- Deleting a _client_ still counts archived purchases when deciding
  `CLIENT_HAS_PURCHASES` — deliberate, since those rows still exist and would
  still cascade, but worth revisiting if it ever surprises anyone.

---

## 2026-07-27 (e) — Executing the integration and E2E suites for the archive work

### Summary

Ran the two opt-in suites that passes (c) and (d) had written but never executed.
Both are now green. Seven failures surfaced on the first run — **all seven were
faults in the new test code, none in the application**; the feature guards fired
correctly with the right codes and parameters every time.

Supersedes the "written, NOT run (awaiting confirmation)" sections of the (c)
and (d) entries below.

### Test cases run

| Suite                                    | Result                  |
| ---------------------------------------- | ----------------------- |
| Rust `cargo test`                        | 37 passed               |
| TS unit (`npm test`)                     | 110 passed, 7 files     |
| Integration (`npm run test:integration`) | **108 passed, 5 files** |
| E2E (`npm run test:e2e`)                 | **35/35 passed**        |

**Gates:** eslint · vue-tsc · vite build · prettier — all clean.

### Issues found

1. **`expect(...).rejects` does not work against the browser/mock backend
   (5 integration failures, fixed in the tests).** The gateway builds the mock
   path as `Promise.resolve(mockDb.x())`, so the mock executes _before_ the
   promise is constructed and a failure is thrown synchronously out of
   `api.x(...)` rather than carried by a rejected promise — whereas under Tauri
   `invoke` rejects. Awaiting inside a `try` reads both backends identically,
   which is why the pre-existing suites already did it that way; my new file did
   not. Added a `failureOf()` helper to `client-archive.integration.test.ts` and
   converted the one case in `overdue-dashboard`.

   **This is a real api/mock divergence, still open.** All current callers
   `await` inside `try`, so nothing is broken today, but a caller written as
   `api.deleteClient(id).catch(handle)` would work on the desktop and throw in
   the browser. Fixing it properly means routing the mock branch of every
   `src/api/index.ts` method through an `async` wrapper so a throw becomes a
   rejection. Not done here — it touches all ~25 gateway methods and is outside
   the scope of running the tests. Recommended as its own change.

2. **`open()` mid-test silently undoes everything the test just did
   (2 E2E failures, fixed in the tests).** `tests/e2e/run.mjs` documents that a
   full document load re-instantiates the in-memory mock — that is what keeps
   tests independent. Two of the new scenarios called `open()` _after_
   archiving, which reset the mock and wiped the archive, so the assertions ran
   against a fresh seed. Replaced with in-app navigation (`.nav-item` clicks)
   and, in the round-trip test, by dropping a detail-page detour whose coverage
   the integration suite provides directly.

3. **`getByRole("button", { name: "Nouvel achat" })` is ambiguous.** The sidebar
   carries a permanent "Nouvel achat" button alongside the one on the Achats
   page, so the unscoped locator is a strict-mode violation. Scoped to
   `getByRole("main")`, matching the comment already at `run.mjs:253`. Note
   `run.mjs:493` still uses the unscoped form in a passing test — latent, and
   worth tightening next time that file is touched.

### Recommendations

- **Fix the api/mock rejection divergence (issue 1)** as a standalone change; it
  is the kind of gap the api/mock parity invariant exists to catch, and the
  integration suite only caught it because a new file happened to use idiomatic
  Vitest.
- Prefer in-app navigation over `open()` inside any E2E scenario that has
  already mutated state; reserve `open()` for the initial navigation.
- The manual checks from (c) and (d) are still outstanding: the Arabic RTL pass
  over the scope tabs, the callout and the three-button footer, and one launch
  against a `user_version = 1` database to exercise `m0002` on disk.

---

## 2026-07-27 (d) — Making the "cannot archive, still owes money" refusal visible

### Summary

Follow-up to (c) on presentation only — **no backend file was touched**. The
refusal used to arrive as a bottom-right toast that auto-dismissed after 3.5 s,
by which time the confirm dialog the user had been looking at was already
closed: spatially disconnected, transient, and a dead end.

Three changes:

1. **The archive dialog now opens already blocked.** `ClientsView` has
   `totalOutstanding` on every row, so it knows the archive will be refused
   before the user commits. The dialog shows a danger callout naming the
   formatted amount and renders "Archiver" disabled, replacing the
   confirm → reject sequence entirely.
2. **The blocked dialog offers "Voir ses échéances"**, routing to the client's
   detail page where the unpaid installments are listed — a refusal now has a
   way forward.
3. **Error toasts no longer auto-dismiss** app-wide (success/info keep 3.5 s),
   with the stack capped at the newest 4 so a repeatedly failing action cannot
   grow it without bound.

`ConfirmDialog` gained three optional props (`warning`, `confirmDisabled`,
`secondaryLabel`) and a `secondary` emit, all inert when unset. It has exactly
one consumer (`ClientsView`), so nothing else changed behaviour.

### Test cases run

| Area                 | Cases                                       |
| -------------------- | ------------------------------------------- |
| TS unit (`npm test`) | 110 passed, 7 files (no unit surface added) |

**Gates:** eslint · vue-tsc · vite build · prettier — all clean. Rust gates not
re-run beyond (c); no `src-tauri/` file is in this diff.

### Test cases — written here, executed in the 2026-07-27 (e) pass above

- **`tests/e2e/run.mjs`**: `a client who still owes money cannot be archived`
  was replaced by `archiving a client who owes money is refused before the user
can confirm` — it now asserts the callout text, that the amount is formatted
  with its currency rather than a bare integer, that it is not a raw machine
  code, that the confirm button reports `disabled`, and that the plain confirm
  body is absent. New `the blocked archive dialog routes to the client's
installments` asserts the `/clients/:id` navigation and the client landed on.
  The Salma Jlassi archive → badge → restore round trip is deliberately
  untouched: it is the regression guard that the unblocked path still works.
- Integration and Rust suites need no change — the backend contract
  (`ARCHIVE_HAS_OUTSTANDING`) is unchanged and still covered by
  `client-archive.integration.test.ts` and `error-contract.integration.test.ts`.

### Issues found

1. **The stale-list race had to stop toasting.** `confirmArchive` previously
   caught `ARCHIVE_HAS_OUTSTANDING`, toasted, and closed. It now records the
   backend's figure in `serverOutstanding` and leaves the dialog open, which
   re-renders it blocked with the amount the database reports _now_ rather than
   the one the list loaded with. `confirmPending` no longer closes the dialog
   unconditionally. Same reasoning that governed the old `CLIENT_HAS_PURCHASES`
   re-prompt: the row's `totalOutstanding` is a prediction, the backend is the
   authority.
2. **Persisting error toasts are a click-interception risk in E2E.** Checked:
   the only toast references in `run.mjs` are one negative assertion
   (`.toast--error` count is 0) and a `.field-error` inside PaymentModal, so no
   test interacts with the bottom-right corner after raising an error. Low, but
   worth re-checking when new error-path scenarios are added.
3. **Disabled confirm button.** Normally an antipattern when the reason is
   hidden — here the callout sits directly above it with `role="alert"`, so the
   cause is visible and announced. Noted rather than open.

### Recommendations

- Run the E2E suite before shipping; the two rewritten cases are the only
  coverage of the blocked-dialog presentation.
- **Check the Arabic RTL rendering of the three-button footer** (Annuler / Voir
  ses échéances / Archiver) and the callout's icon-text gap. The callout uses
  flex + `gap` and `margin-block-start`, so it should mirror cleanly, but the
  three-button footer is the widest this modal has ever been.
- Consider applying the same up-front-explanation treatment to the delete path's
  stale-list `CLIENT_HAS_PURCHASES` case, which still toasts. Left alone here
  because the delete button is not rendered at all for a client with history, so
  that toast is genuinely unreachable outside the race.
- Still open from earlier passes: findings 4 and 5 of (c)
  (`errors.archiveHasOutstanding` unformatted in the generic fallback;
  `ClientDetailView` collapsing every load failure into "client missing"), the
  N+1 → `GROUP BY` rewrite (#7), and `rusqlite` 0.32 → 0.40 (#16).

---

## 2026-07-27 (c) — Client archive, and removal of the force-delete cascade

### Summary

Replaced the destructive client delete with an archive. `delete_client` lost its
`force` parameter: a client with any purchase is now refused terminally with
`CLIENT_HAS_PURCHASES:{n}`, and the FK cascade behind it is unreachable from a
client delete. Hard delete survives only for a client with zero purchases.
Clients gained a nullable `archived_at` (migration `m0002_client_archive`),
`archive_client` / `restore_client` commands, and an Actifs / Archivés / Tous
scope filter on the Clients page.

Two guards keep one invariant true — **an archived client always has a zero
balance**: archiving is refused while any installment is unpaid
(`ARCHIVE_HAS_OUTSTANDING:{remaining}`), and `create_purchase` refuses an
archived client (`CLIENT_ARCHIVED`). That invariant is why impayés, the
dashboard and the reports need no `archived_at` filter at all, and it is
asserted directly in the new integration suite.

Unit tests were run; integration and E2E were written but **not executed**, per
the workflow.

### Test cases run

| Area                 | Cases                                                                 |
| -------------------- | --------------------------------------------------------------------- |
| Rust `cargo test`    | 37 passed (was 25; +12 covering archive/restore/delete and migration) |
| TS unit (`npm test`) | 110 passed, 7 files (unchanged — no unit surface added)               |

New Rust cases: `delete_client_is_refused_for_any_client_with_purchases`
(replaces `delete_client_is_gated_then_cascades`, whose force/cascade half no
longer exists), `delete_client_removes_a_client_with_no_purchases`,
`delete_client_reports_a_missing_id_rather_than_succeeding_silently`,
`archive_client_is_refused_while_the_client_owes_money`,
`archive_client_succeeds_once_every_installment_is_paid`,
`archive_client_succeeds_for_a_client_with_no_purchases` (the empty-aggregate
case), `archive_stamp_is_an_iso_date_and_does_not_move_on_a_repeat`,
`restore_client_clears_the_stamp_and_is_idempotent`,
`archive_and_restore_report_a_missing_client`,
`an_archived_client_cannot_take_on_a_new_purchase`,
`list_clients_filters_by_archived_state`,
`list_clients_keeps_clients_with_no_purchases_under_every_scope`, and
`m0002_defaults_existing_clients_to_active`.

**Gates:** eslint · vue-tsc · vite build · prettier · cargo fmt · cargo clippy
(`--all-targets -D warnings`) — all clean.

### Test cases — written here, executed in the 2026-07-27 (e) pass above

- **`tests/integration/client-archive.integration.test.ts`** (new, 16 cases):
  the outstanding-balance guard; the combined lockout (an indebted client
  rejects both delete and archive); archive after settling; the empty-purchase
  client; the scope partition (`active + archived === all`, and the default
  equals `active`); history reachability after archiving (detail page,
  purchases, payments); **money aggregates byte-identical across the archive**
  (`getDashboard`, `listImpayes`, `listSchedule` deep-equal before/after); the
  ISO-date shape of the stamp; `CLIENT_ARCHIVED` on a new purchase, and success
  after restore; restore idempotence; re-archive not moving the stamp;
  `CLIENT_NOT_FOUND` from both.
- **`tests/integration/overdue-dashboard.integration.test.ts`**: the
  force-delete case was removed with the feature; replaced by an outright
  refusal that asserts nothing moved anywhere, a successful zero-purchase
  delete, and `CLIENT_NOT_FOUND` on a stale id.
- **`tests/integration/error-contract.integration.test.ts`**:
  `ARCHIVE_HAS_OUTSTANDING:750` and `CLIENT_ARCHIVED` added to the contract
  list (which forces the key into all three locales and rejects leftover
  placeholders), plus an interpolation assertion.
- **`tests/e2e/run.mjs`**: `delete-client safeguard warns…` rewritten to
  `a client with purchases offers no delete at all`; new `a client who still
owes money cannot be archived` (asserts localized prose and a **formatted**
  amount with its currency, not a bare integer); new archive → Archivés tab →
  badge → detail-page history → restore round trip; new `archived clients are
absent from the new-purchase client picker`. The raw-code leak guard was
  extended with `ARCHIVE_HAS_OUTSTANDING|CLIENT_`.

Run them with `npm run test:integration` and `npm run test:e2e`.

### Issues found

1. **`m0002` would have bricked the app on launch (caught pre-merge, fixed).**
   The first draft used a bare `ALTER TABLE client ADD COLUMN archived_at`.
   SQLite has no `ADD COLUMN IF NOT EXISTS`, and `migrate` replays the whole
   ladder for any database at `user_version = 0`; on a replay the `ALTER` fails
   with "duplicate column name", which fails `Db::open` and therefore startup.
   Now guarded by a `pragma_table_info` check, and
   `migrate_is_versioned_and_idempotent` asserts the column count after a replay.
   This is the ladder's first appended step, so nothing had exercised the path.
2. **`datetime('now')` would have rendered raw on screen (caught pre-merge,
   fixed).** `formatDatePattern` (`src/composables/useFormat.ts:20`) does
   `iso.split("-").map(Number)`; a `"2026-07-27 12:34:56"` stamp makes the day
   component `NaN`, so the guard returns the input verbatim and the user sees a
   timestamp. Changed to `date('now')`, matching the schema's ISO-date
   convention. Asserted by `archive_stamp_is_an_iso_date_…` and by an
   integration regex.
3. **`create_purchase` did not check the archived flag (found in Code Review,
   fixed).** Without it, "archived implies a zero balance" was true only by UI
   convention — the picker filters archived clients, but a `clientId` sent
   straight over IPC would have given an archived client a balance the money
   read models assume cannot exist. Added the `CLIENT_ARCHIVED` guard inside the
   existing transaction, mirrored in the mock, covered by a Rust and an
   integration test.
4. **`errors.archiveHasOutstanding` interpolates an unformatted integer.** Not a
   regression — `errors.overpayment` (`{remaining}`) and `errors.sumMismatch`
   have the same shape today — and not user-visible on the archive path, because
   `ClientsView.confirmArchive` catches the code explicitly and formats with
   `fmt.money`. The generic fallback remains unformatted. Open, low.
5. **`ClientDetailView` still reports every load failure as "client missing".**
   Pre-existing (`src/views/ClientDetailView.vue:53`): the bare
   `catch { notFound.value = true }` collapses a transient IPC fault into
   permanent data loss in the UI, which is exactly the split `missing_row`
   exists to preserve on the Rust side. Untouched by this change. Open, medium.

### Recommendations

- Run the integration and E2E suites before shipping — the E2E archive round
  trip is the only end-to-end check that the scope tabs, the badge and the
  picker exclusion actually work together.
- **Verify the Arabic RTL layout by hand.** The scope tabs are a third copy of
  the `.tabs` block from `EcheancesView`/`AlertesView`, kept deliberately
  direction-agnostic (flex + gap, symmetric padding, no physical margins). Any
  future asymmetric spacing must use logical properties.
- **Exercise the real upgrade path once**: launch against a database created by
  the previous build (`user_version = 1`) and confirm `m0002` applies and every
  existing client comes back active. The unit test simulates this, but the
  on-disk path is worth one manual pass.
- Consider extracting the now-triplicated `.tabs` markup and CSS into a shared
  `ui/SegmentedTabs.vue`, converting all three call sites in one change rather
  than mid-feature.
- Fix finding 5 (`ClientDetailView` error collapsing) as its own change.
- Still open from earlier passes: the N+1 → `GROUP BY` rewrite (#7),
  `rusqlite` 0.32 → 0.40 (#16), the `2/6` Excel date-coercion quirk, and the
  LICENSE placeholder holder.

---

## 2026-07-27 (b) — Remediation of the Low-severity findings in `AUDIT_REPORT.md`

### Summary

Closes the Low findings #20–#29 from the 2026-07-26 audit. #22 was already fixed
by the previous pass; #25 is documented as not-actionable; #26 is partially done
with the runtime majors deferred by decision; #30 was confirmed to need no change.

Unlike the previous pass, **the integration and E2E suites were executed** at the
user's request — 87 integration tests and 31 E2E scenarios, all passing. That
also retroactively validated the two E2E scenarios written but not run last time.

### Test cases run

**TS unit — `npm test`, 110 passed / 0 failed** (was 85). The 25 new tests are
`src/lib/csv.test.ts`:

| Area              | Cases                                                                                                                                                                                                                                                 |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Quoting           | plain strings; empty; embedded `"` doubled; a bare `"`; embedded comma; embedded newline; Arabic and accented text preserved                                                                                                                          |
| Formula injection | `=cmd\|'/c calc'!A1`, `=1+1`, `+1`, `-1+2`, `@SUM(A1)`, leading TAB and leading CR each neutralized; guard applied _before_ escaping so the apostrophe lands inside the field; values that merely _contain_ a trigger (`Ben-Salah`, `a=b`) left alone |
| Numbers           | emitted bare so sheets treat them as numeric; a negative number is **not** apostrophe-prefixed; `NaN`/`Infinity` render empty                                                                                                                         |
| Document          | BOM first, CRLF line endings, trailing newline, header cells escaped too                                                                                                                                                                              |
| `buildImpayesCsv` | one row per overdue installment; several clients/installments flattened; empty list is header-only; a hostile name keeps the row at exactly 7 fields                                                                                                  |

**Rust — `cargo test`, 25 passed / 0 failed** (was 24). The new test is
`payment_rows_resolve_join_columns_from_the_right_table`: it asserts
`purchase_reference`, `purchase_id`, `installment_index`, `client_id` and
`amount` each resolve from the correct table in the four-table payment join —
the property `pay.*` could silently break.

**Integration — `npm run test:integration`, 87 passed / 0 failed** (4 files).

**E2E — `npm run test:e2e`, 31 passed / 0 failed** (was 29). Two new scenarios:

- _deleting a client with no purchases needs a single confirm_ — the path that
  now genuinely sends `force: false`, so the backend gate decides.
- _impayés: the exported CSV is localized and properly quoted_ — captures the
  actual download and asserts the BOM, the dated filename, the **localized**
  header row, and that every data row parses to exactly seven fields.

**Gates:** `eslint` clean · `vue-tsc --noEmit` clean · `npm run build` clean ·
`prettier --check` clean · `npm audit --audit-level=high` 0 vulnerabilities ·
`cargo fmt --check` clean · `cargo clippy --all-targets -- -D warnings` clean ·
**`cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`** (the
licence change was the step most likely to break this) · `cargo audit` unchanged
at 0 vulnerabilities / 17 warnings.

### Issues found

1. **The CSV guard fires on every phone number, and that is correct.** A unit
   test caught that `+216 …` — the format of every Tunisian number — starts with
   a formula trigger and so gets an apostrophe. Kept deliberately: unguarded,
   Excel parses `+216 98 123 456` as a formula and renders `#NAME?`, so the
   guard fixes the phone column as well as securing it. Documented in `csv.ts`
   and pinned by two tests.
2. **The CSP risk flagged in the plan turned out not to exist.** Rather than
   leave it to manual testing, I read `replace_csp_nonce` in `tauri-2.11.5`:
   when it injects nonces it does `csp.entry("script-src").or_default()` and
   **pushes `'self'` if absent**. So the old implicit form and the new explicit
   `script-src 'self'` produce an identical effective policy. `object-src` is
   untouched by Tauri. No runtime behaviour change.
3. **The playwright 1.61 → 1.62 bump needs `npx playwright install`.** The E2E
   suite failed outright until the matching browser was downloaded. CI installs
   browsers already, but anyone pulling this branch locally will hit it.
4. **Pre-existing, not fixed: the "Tranche" column (`2/6`) is coerced to a date
   by Excel on import.** Quoting does not prevent it; only an apostrophe prefix
   would, at the cost of a visible apostrophe on every row. Out of scope for
   this pass and unrelated to the injection finding — flagged rather than
   silently changed. See recommendations.
5. **The stale-list re-prompt branch of the new delete gate is not covered
   end-to-end.** Making the client list stale requires two independent backend
   states, and the browser mock is a per-page-load singleton, so E2E cannot
   express it. The backend half (`force: false` → `CLIENT_HAS_PURCHASES`) is
   covered by the integration suite and by `delete_client_is_gated_then_cascades`
   in Rust; the UI half is covered only by inspection.

### Recommendations

- **Decide on the `2/6` date-coercion quirk** (issue 4). Options: prefix the
  tranche cell with `'`, emit two separate columns (`Tranche` / `Sur`), or leave
  it. Two columns is probably the cleanest and costs one locale key.
- **Add `npx playwright install` to the contributor setup notes** or a
  `postinstall`, so issue 3 doesn't bite the next person.
- **The MSRV is now verified, not just declared.** `rust-version = "1.88"` was
  measured against the locked graph (max dependency MSRV is exactly 1.88.0), and
  a new `MSRV (1.88)` job in `build.yml` runs `cargo +1.88 check --locked` so the
  claim cannot drift back to fiction the way 1.77 did.
- **Deferred: #26 runtime majors** — pinia 4, vue-router 5, vue-i18n 11, Vite 8,
  TypeScript 7, as one PR gated on the E2E suite. Two findings for whoever picks
  it up: `@vitejs/plugin-vue@6.0.8` and `vitest@4.1.10` **already declare
  `vite ^8.0.0`**, so Vite 8 needs no companion bumps; and
  `typescript-eslint@8.65.0` pins `typescript <6.1.0`, which **hard-blocks
  TypeScript 7** until that ceiling lifts — sequence TS last.
- **#25 stays open by design.** 17 RustSec warnings (16 unmaintained, 1 unsound
  `glib`), all transitive through Tauri's GTK3 stack with no fixed versions.
  `deny.toml` now states the policy explicitly (`unmaintained = "workspace"`)
  with the reasoning inline, and `ignore = []` keeps them visible weekly.
- **The LICENSE copyright holder is the placeholder `paymentSchedule`**, taken
  from `Cargo.toml`'s `authors`. Replace it with your legal name or company
  before distributing any build — `build.yml` publishes GitHub Releases.
- **Still open from earlier passes:** the N+1 → `GROUP BY` rewrite (#7) and
  `rusqlite` 0.32 → 0.40 (#16).

---

## 2026-07-27 — Remediation of the High & Medium findings in `AUDIT_REPORT.md`

### Summary

Closed all 5 High and 12 of the 14 Medium findings from the 2026-07-26 audit;
2 Mediums are deliberately deferred (below). Finding #1 (the `fs` plugin grant
over `$APPDATA`) was already fixed in the working tree at the start of this pass
and is now verified to compile and to leave the logo rendering path intact.

The backend went from 3 tests to 24, and gained its first coverage of the code
that owns the money. `npm audit --audit-level=high` — the CI gate that was
**failing** at audit time — now exits 0.

### Test cases run

**Rust — `cargo test`, 24 passed / 0 failed** (was 3 tests).

| Area              | Cases                                                                                                                                                                                                                      |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create_purchase` | even split with remainder last (parts sum to total); manual uneven split; `SUM_MISMATCH` writes **no** purchase and **no** installment rows (proves rollback)                                                              |
| Input validation  | `total_price` 0 and negative; `installment_count` 0 and 121; unknown `interval_kind`; `interval_days` 0 and 400; malformed `purchase_date`; malformed manual `due_date` — 8 codes, and no rejected request writes a row    |
| `record_payment`  | partial leaves `paid_date` NULL; exact payment sets it and flips status to `paid`; **overpayment rejected** with `OVERPAYMENT:150` leaving `paid_amount` at 100; zero/negative amount; malformed date; unknown installment |
| Cascades          | `delete_client` gated at `CLIENT_HAS_PURCHASES:1` then cascading to purchases → installments → payments; `delete_purchase` cascading while the client survives                                                             |
| Settings          | patch applies atomically, `alert_soon_days` 500 clamps to 90, and an explicit language choice clears `language_is_default`                                                                                                 |
| Migrations        | fresh DB stamps `user_version = 1`; a simulated pre-versioning DB (`user_version = 0`, tables present) migrates forward **without losing rows**                                                                            |
| Durability        | live connection reports `journal_mode = wal`, `busy_timeout = 5000`, `foreign_keys = 1`                                                                                                                                    |
| Date math         | `add_interval` saturates instead of panicking for `i64::MAX` / `i64::MIN` / negative `k` across all three interval kinds                                                                                                   |
| Backup            | the snapshot reopens as a valid SQLite database with its rows intact, and a destination that is not a SQLite file is refused rather than clobbered                                                                         |
| Errors            | an internal `rusqlite` error serializes to `"INTERNAL"` while retaining its detail in-process; actionable codes keep their parameters                                                                                      |
| Missing rows      | absent client/purchase report `CLIENT_NOT_FOUND` / `PURCHASE_NOT_FOUND` rather than an opaque internal error                                                                                                               |

**TS unit — `npm test`, 85 passed / 0 failed** (was 56). The 29 new tests are the
cross-language parity suite: `finance.ts` and `db.rs` are both asserted against
`tests/fixtures/finance-parity.json` (8 split cases, 13 interval cases, 7 status
cases), so a change to one implementation without the other now fails a test.

**Gates:** `npx eslint .` clean · `npx vue-tsc --noEmit` clean · `npm run build`
clean (under the new `vue-tsc` 3.x) · `cargo fmt --check` clean ·
`cargo clippy --all-targets -- -D warnings` clean ·
`npm audit --audit-level=high` **exit 0, 0 vulnerabilities**.

**Written but NOT executed** (per the CLAUDE.md constraint — say the word and I
will run them):

- `tests/integration/error-contract.integration.test.ts` — every rejection code
  through the real `api` facade, plus the assertion that all 16 codes resolve to
  real prose in **fr/en/ar** and that raw SQL text / filesystem paths fall back
  to the generic message.
- `tests/e2e/run.mjs` — two new scenarios: a rejected overpayment shows
  localized prose naming the remaining balance (not `OVERPAYMENT`, not SQL), and
  the backup card stays hidden outside the Tauri runtime.

### Issues found

1. **Blocker, found in the Code Review pass and fixed: `backup_database` was an
   arbitrary-file-destruction primitive.** The first implementation called
   `std::fs::remove_file(&dest)` before `VACUUM INTO`, because `VACUUM INTO`
   refuses to overwrite. Two problems: `dest` comes from the renderer, so a
   compromised WebView could delete any file the process can write; and if the
   vacuum then failed, the user had lost whatever was at that path with nothing
   to show for it. Now the command requires a `.db` extension, refuses to
   overwrite anything whose first 16 bytes are not `SQLite format 3\0`, and
   writes to a sibling `.db.part` file renamed into place only on success.
   Covered by `backup_writes_a_readable_snapshot_without_clobbering_other_files`,
   which also reopens the snapshot and asserts the rows survived.
2. **My own first parity assertion was wrong, not the code.** The test fixture
   purchase is dated 2024-01-15, so every tranche is already overdue and the
   rollup correctly reports `late`, not `pending`. Corrected the assertion and
   documented why status is computed against today rather than stored.
3. **A test collided with the demo seed data.** `migrate_is_versioned_and_idempotent`
   inserted a sentinel client named "Ben Salah", which also exists in the
   Tunisian demo seed (on in debug builds), so the row count was 2. Switched to
   a unique sentinel.
4. **`installment.paid_amount` may already exceed `amount` in existing
   databases.** The new guard prevents _new_ overpayment but does not repair
   rows written before it. Any such row still makes `amount - paid_amount`
   negative in the outstanding/overdue aggregates. Not covered by a test because
   there is no migration to fix it — see recommendations.
5. **`get_setting` swallowed query errors**, making a broken settings table
   indistinguishable from a fresh install. Now logs at `warn` before falling
   back. Found while wiring the error type, not listed in the audit.

### Recommendations

- **Run the integration and E2E suites** before shipping this. The error
  contract is asserted end-to-end there and nowhere else.
- **Add a migration (v2) that clamps `paid_amount` to `amount`** for rows
  written before the overpayment guard, and decide whether the excess should
  become a recorded credit or be discarded. Issue 4 above.
- **Deferred by decision, still open:** finding #7's N+1 → `GROUP BY` rewrite of
  `list_purchases` / `get_client_detail` / `get_dashboard` (the `async`
  conversion landed, so they are off the main thread, but they still issue
  ~3N+1 queries), and finding #16, `rusqlite` 0.32 → 0.40. Both now have the
  backend test suite as a safety net, which is what they were waiting for.
- **Findings #20–#30 (Low/Info) were out of scope** and remain open — MSRV
  (`rust-version = "1.77"` is not achievable), CSV quote-escaping and
  formula-injection in the Impayés export, the missing `license` field,
  `*.db` in `.gitignore`, the frontend majors, and CodeQL coverage for Rust.
- **Manual verification still owed on a real desktop run** (`npm run tauri dev`):
  logo still renders through the narrowed `$APPDATA/logo.*` asset scope; a
  non-image or >5 MB file is refused with localized text; the backup file opens
  in `sqlite3`; a second launch focuses the existing window; and the new error
  toasts read correctly in Arabic RTL.

---

## 2026-07-26 — Bug: call/SMS buttons strand the user on a WebView error page

### Summary

**Reported:** on the Impayés (overdue) page, clicking the phone-call button shows
a blank page reading "The URL can't be shown", with no way back.

**Confirmed, and it is not a routing bug.** The call and message buttons are plain
anchors to external URI schemes:

```vue
<a class="contact-btn contact-btn--call" :href="tel(c.phone)">   <!-- tel:21698… -->
<a class="contact-btn contact-btn--msg"  :href="sms(c.phone)">   <!-- sms:21698… -->
```

Nothing intercepts the click. Tauri 2 does not delegate non-`http(s)` schemes to
the OS by default, and this app registers no opener/shell plugin
(`src-tauri/src/lib.rs:15-17` has only `os`, `dialog`, `fs`) and no
`on_navigation` hook. So the click becomes a **top-level navigation of the
WebView itself**; WebKitGTK cannot load `tel:` and replaces the document with its
own error page — the text the user saw.

Severity: **blocker-class UX**. Because the SPA document is destroyed, Vue, the
router and every in-app control are gone. The Tauri window has no browser chrome
(no back button, no address bar), so **the only recovery is quitting and
reopening the app**. Unsaved modal state is lost.

This is why the not-found work from the earlier pass today does not help here:
`vue-router` never sees this navigation. The fix must _prevent_ it, not react to
it — no Vue-rendered back button can exist on a page Vue is no longer running on.

### The fix (implemented this pass)

User approved adding the opener dependency. All four call sites now go through
one path:

- `@tauri-apps/plugin-opener` + `tauri-plugin-opener` (both 2.5.4), registered in
  `src-tauri/src/lib.rs`.
- `src-tauri/capabilities/default.json` grants `opener:allow-open-url` scoped to
  exactly `{ "url": "tel:*" }` and `{ "url": "sms:*" }`. Deliberately **not**
  `opener:default`, which additionally grants `http://*`, `https://*`,
  `mailto:*` and `reveal-item-in-dir` — and which, despite that breadth, does not
  cover `sms:`. Verified against Tauri's generated ACL
  (`src-tauri/gen/schemas/capabilities.json`).
- `api.openExternal(url)` added to the gateway with a matching `mockDb`
  implementation that records the URI and does not navigate.
- New `src/composables/useContactActions.ts` — `contactUri()` validates the
  number and builds the URI; `call()`/`message()` await the gateway and toast on
  failure. `console.error` keeps the underlying plugin error for diagnostics while
  the toast stays free of internals.
- The four anchors became `<button type="button">` in `ImpayesView.vue` and
  `ImpayesPanelCard.vue`. With no `href`, navigating away is now impossible by
  construction rather than by interception.

### Test cases run

**Unit — RUN, 56/56 passing** (`npm test`; 9 new for `contactUri`). Frontend gates
clean: `npm run lint`, `npm run build`. Rust gates clean (`src-tauri/` changed):
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
(3 passing).

`contactUri` — accepts and normalizes:

- `tel:` for calls, `sms:` for messages.
- Strips presentational separators: `98 123 456`, `(216) 98-123.456`, padded
  input.
- Keeps a leading `+` for international numbers.

`contactUri` — rejects, so it never reaches the OS handler:

- Empty/whitespace-only, free text (`appeler le bureau`, `N/A`).
- Other schemes and URI syntax: `javascript:alert(1)`, `file:///etc/passwd`,
  `?foo=bar`, `#frag`, `,999`.
- Too short (`12`, `+`), implausibly long (40 digits), separators-only (`()-.`).
- A `+` that is not the international prefix — refused rather than normalized.

**E2E — RUN, 27/27 passing** (`npm run test:e2e`; 2 new, 1 rewritten):

- The inverted assertion: call/message/view are all `<button>`, and
  `a[href^='tel:'], a[href^='sms:']` count is **0**. This replaces the old
  assertion that required `href` to start with `tel:` — the one that kept the
  suite green while the defect was live.
- Impayés: clicking call then message leaves the URL unchanged, `.app-shell`
  present, cards still rendered, and **no error toast** for a valid seeded number
  (proving the click → composable → gateway → mock path works end to end).
- Dashboard: the overdue panel's actions are buttons, no scheme anchors anywhere
  on the page, and the dashboard still renders 5 KPI cards after a call click.

**Not verified in the real WebView.** This is the one gap. Playwright drives
Chromium, which does not reproduce WebKitGTK's failure mode, so the automated
suite proves the markup and the gateway path but not the OS hand-off. A
`npm run tauri dev` attempt was made and the binary did build with the plugin
compiled in, but the launch aborted because port 5173 was already occupied by a
running dev server, and no screenshot/input-automation tooling is available here.
Manual confirmation steps are in Recommendations.

### Diagnosis evidence

- Traced the click path from the anchor markup to the absent handler: no
  `@click.prevent`, no `target`, no navigation guard, no opener plugin.
- Enumerated every external-scheme surface in `src/` — 4 affected call sites in 2
  files (below).
- Probed the host for a handler: `xdg-mime query default x-scheme-handler/tel`
  returns **empty** on this Linux box — nothing is registered for `tel:` at all.
- Confirmed CSP is not the cause: `default-src 'self'` does not govern link
  navigation, and the failure is the WebView being unable to load the scheme.

### Issues found

1. **Call button strands the user** (blocker, **fixed**) —
   `src/views/ImpayesView.vue:167`. Repro before the fix: `npm run tauri dev` →
   sidebar **Impayés** → click the green phone button on any overdue client card →
   WebView shows "The URL can't be shown"; no back affordance; must quit the app.
2. **SMS button had the identical defect** (blocker, **fixed**) —
   `src/views/ImpayesView.vue:174`. Same repro via the message button. One click
   away from the reported path, same root cause.
3. **The dashboard reproduced both** (blocker, **fixed**) —
   `src/components/dashboard/ImpayesPanelCard.vue:72` (tel) and `:75` (sms). The
   overdue panel on the **home screen** carried the same anchors, so a user could
   brick the window without ever visiting Impayés. Not in the original report.
4. **The E2E suite enshrined the bug** (should-fix, **fixed**) —
   `tests/e2e/run.mjs`, test "impayés: export button is present and each card
   exposes call/SMS/view actions", asserted `href` starts with `tel:` / `sms:`. The
   suite was green _because_ the broken markup was present, which is why 25/25
   passed earlier today while this defect was live. Playwright never follows the
   scheme, so it cannot observe the stranding. Assertion now inverted.
5. **`tel:` has no handler on desktop Linux** (design constraint, not a defect,
   **accommodated**). Even a correct hand-off has nothing to hand off to here, so
   the fix treats rejection as a first-class path: an error toast naming the number
   (`impaye.callFailed` / `impaye.messageFailed`, all three locales) rather than a
   silent no-op.
6. **Phone field was unvalidated** (should-fix, **fixed**) — found while
   implementing. `c.phone` is free text and the old helpers only stripped
   whitespace, so whatever was typed went straight into a URI. `contactUri` now
   rejects on character set before touching digits and rebuilds the URI from
   digits plus an optional leading `+`.
7. **CSV export is a related but distinct risk** (should-fix, **open — tracked at
   user's request**) — `src/views/ImpayesView.vue:112-118` downloads via
   `URL.createObjectURL` + `a.download`. `a.download` on a `blob:` URL is
   unreliable in WKWebView/WebKitGTK. Failure mode is a silent no-op rather than
   stranding, so it is not urgent, but it is the same class of problem: browser
   affordances assumed to work inside a WebView. Not investigated this pass.

### Recommendations

- **Confirm in the real app** — the one thing automation could not cover. Stop any
  dev server on port 5173, then `npm run tauri dev` → **Impayés** → click the
  phone button. Expected: the app stays put and an error toast names the number
  (this desktop has no `tel:` handler). Repeat on the dashboard's overdue panel,
  and on a machine that _does_ have a handler to confirm the hand-off itself.
- Verify the toast in Arabic too — the new strings are RTL and untested visually.
- Sweep for the same anti-pattern before it reappears: any `<a href>` to a
  non-`http(s)` scheme, `target="_blank"`, or `window.open` inside this WebView
  will fail the same way. Currently there are none outside the CSV export.
- Address issue #7 when convenient; a Rust command writing the CSV via the
  existing `fs`/`dialog` plugins needs no new dependency.
- Consider `aria-label` instead of `title` on the icon-only contact buttons — the
  accessible name currently comes from `title`, matching the pre-existing
  `contact-btn--view` pattern, but `aria-label` is more reliable.
- Replace the assertion in issue #4 with one that proves the click does **not**
  navigate away (e.g. the document still has `.app-shell` afterwards).

---

## 2026-07-26 — Feature QA: not-found recovery (back navigation)

### Summary

Audited the "let the user get back when a page isn't found" behaviour and closed
four gaps around it. The affordance itself already existed — the router's
catch-all (`name: "not-found"`) renders `NotFoundView.vue` with a Back button
(`useBack`) plus a dashboard link, and the detail views show a recoverable
message for a missing/deleted id. What was missing: correct RTL presentation, a
guard against Back landing on a second not-found page, a page title, and any
test coverage at all.

Changes under test:

- `src/composables/useBack.ts` — the back/fallback decision is extracted into a
  pure `shouldGoBack(back, resolveName)` helper, which now also refuses a stored
  history entry that resolves to the `not-found` route.
- `src/style.css` — new opt-in `[dir="rtl"] .icon-flip { transform: scaleX(-1) }`
  utility; applied to the back arrow in `NotFoundView`, `ClientDetailView` and
  `PurchaseDetailView` (both the missing-record and normal branches).
- `src/components/layout/AppHeader.vue` — `not-found` added to `NAV_KEY`, reusing
  the existing `notFound.title` key (no new strings, so all three locales stay at
  261 identical keys).
- `src/router/index.ts` — comment recording that the `"not-found"` route name is
  string-matched by `useBack` and `AppHeader`.

### Test cases run

**Unit — RUN, 46/46 passing** (`npm test`; 9 new in
`src/composables/useBack.test.ts`). `npm run lint` and `npm run build`
(`vue-tsc --noEmit`) also clean.

`shouldGoBack` — no usable history:

- `null` back entry (fresh document load, the deep-link case) → fallback.
- `undefined` / empty-string entry → fallback.
- Non-string history values (number, object) → fallback rather than trusted.

`shouldGoBack` — history points at a real page:

- List, detail, and dashboard paths → `router.back()`.

`shouldGoBack` — history points at another unknown URL:

- `/nope` and `/achats/12/nope` → fallback, so Back never swaps one not-found
  screen for another.
- A resolver that throws → fallback, not a propagated exception.

**E2E — RUN at the user's request, 25/25 passing** (`npm run test:e2e`; 5 new
scenarios in `tests/e2e/run.mjs`, plus the 20 pre-existing ones, no regressions).
The Playwright browser had to be provisioned first with
`npx playwright install chromium` — a fresh checkout will need that before the
suite can run.

- Unknown route renders the localized card, the header reads "Page introuvable"
  (not the app name), and two ways out are offered.
- Back on a deep-linked not-found page falls back to the dashboard.
- The dashboard link reaches a genuinely rendered dashboard (5 KPI cards).
- Switching to Arabic flips the document to RTL and the back arrow's computed
  transform becomes `matrix(-1, 0, 0, 1, 0, 0)`.
- A deleted record's detail page (`/clients/999999`) shows the recoverable
  message and its Back button falls back to the clients list.

### Issues found

All four were found by this audit and fixed in this pass.

1. **Back arrow not mirrored in RTL** (should-fix, fixed). The three back buttons
   hardcoded `<AppIcon name="arrow-left" />` and `style.css` had essentially no
   `[dir="rtl"]` rules, so in Arabic the arrow pointed away from where "back" is.
   Repro: open any client detail page, switch to العربية, observe the arrow.
2. **Back could loop into a second not-found page** (should-fix, fixed).
   `useBack` tested `history.state.back` for truthiness without checking where it
   pointed. Repro (pre-fix): in-app navigate to one unknown URL, then another,
   then press Retour — you land on a not-found page again.
3. **Header showed the app name on the 404 page** (nit, fixed). `not-found` was
   absent from `AppHeader`'s `NAV_KEY`, so `title` fell through to `t("app.name")`.
4. **No test coverage** (should-fix, fixed). No unit test for `useBack`, no E2E
   scenario touching an unknown route, no QA record.

### Recommendations

- The computed-transform assertion (`matrix(-1, 0, 0, 1, 0, 0)`) is the only
  automated check on the RTL mirroring, and it passes. Keep it if `.icon-flip`
  is ever refactored — a plain screenshot diff would not catch a silent regression
  here.
- CI provisioning: the E2E stage needs `npx playwright install chromium` (or the
  Playwright Docker image) before `npm run test:e2e`; the binary is not vendored.
- **Known limitation, accepted:** backing into a _valid_ route whose record was
  since deleted (`/achats/999` → `achat-detail`) still renders the in-page
  missing state; the router cannot know the row is gone. That screen carries the
  same Back button with a list fallback, so the user is never stranded. Documented
  in `useBack.ts`.
- **Coverage limitation:** `open()` does a full document load and vue-router
  `replaceState`s fresh history state on initial navigation, so `state.back` is
  always null in E2E — the suite can only exercise the _fallback_ branch. There is
  no UI path that router-navigates to an unknown URL. The genuine `router.back()`
  branch and the 404-skip are covered by the unit tests instead.
- `.icon-flip` is opt-in and currently used only on back arrows. Any future
  directional icon (`chevron-left`/`chevron-right` in pagination, for example)
  needs the class added explicitly — worth a sweep if pagination lands.
- Unrelated, noted while auditing: `architecture.md` references a
  `ui/DateRangeFilter` component twice, but no such file exists — that UI lives in
  `ListFilterBar.vue` / `DatePicker.vue`. Stale doc reference, not fixed here.

---

## 2026-07-24 — Tooling QA: clean & secure code baseline

### Summary

Validation pass for the new clean/secure tooling baseline (ESLint + Prettier +
security plugins, rustfmt/clippy/cargo-audit/cargo-deny, husky/lint-staged, and
the CI/Dependabot/CodeQL workflows), plus the vite 7 / vitest 4 upgrade done to
clear pre-existing dev-tooling advisories. Existing behavior was re-verified
against the upgraded toolchain.

### Test cases run

Executed (all green):

- **Unit** — `npm test`: 36/36 passing on vitest 4.
- **Integration** — `npm run test:integration`: 19/19 passing on the upgraded
  separate vitest config (validates the vite 7 / vitest 4 upgrade end-to-end).
- **Rust unit** — `cargo test` (src-tauri): 3/3 passing after the two clippy
  refactors in `commands.rs`/`db.rs`.
- **Typecheck/build** — `npm run build` (vue-tsc + vite build): passes.

Quality/security gates (all green):

- `npm run lint` (ESLint) — 0 problems.
- `npm run format:check` (Prettier) — clean.
- `cargo fmt --check` — clean; `cargo clippy --all-targets -- -D warnings` — clean.
- `npm audit --audit-level=high` — 0 vulnerabilities (was 1 critical + 1 high
  before the vite/vitest upgrade).

Not run locally (execute in CI — tools not installed in this environment):
`cargo audit`, `cargo deny check`, CodeQL analysis.

### Issues found

- None outstanding. The pre-existing vite/vitest/esbuild advisories (1 critical,
  1 high, 3 moderate — dev-tooling only) were resolved by upgrading to vite 7 /
  vitest 4. The `v-html` in `AppIcon.vue` was reviewed and confirmed XSS-safe
  (static SVG map, key-only selection) and annotated accordingly.

### Recommendations

- After first push, confirm the `security.yml` (`cargo audit` / `cargo deny`) and
  `codeql.yml` runs are green; triage any RustSec advisory they surface.
- Commit `src-tauri/Cargo.lock` (now un-ignored) so the Rust audit scans a pinned
  dep set.
- Keep Node ≥ 20.19 locally (CI uses 22) — required by Vite 7.

---

## 2026-07-22 — Feature QA: Alertes (alerts center) page

### Summary

QA pass for the newly implemented **Alertes** page (`src/views/AlertesView.vue`),
which replaced the styled placeholder. The page consolidates every _actionable_
installment — overdue, due today, or due within 7 days — derived from
`api.listSchedule()` through the pure `buildAlerts` classifier
(`src/lib/alerts.ts`). It renders three summary tiles (count + total per kind,
clickable to filter), status tabs, the shared `ListFilterBar`, and a sortable
table with a days-late / due-in "timing" column; rows link to the purchase.

Added integration and E2E coverage and **executed all suites** (unit,
integration, E2E) — the user requested execution. All green.

### Test cases — RUN

Unit — `src/lib/alerts.test.ts` (9 cases, `npm test`): **36/36 passed** overall.

- `classifyAlert` boundaries against a fixed `today`: overdue (positive days late), due-today (0 days), due-soon (days remaining).
- Horizon edge: last day inside the window is `dueSoon`, the day after is dropped; custom horizon respected.
- Fully-paid past-due rows are ignored; partially-paid overdue rows still alert with the correct `remaining`.
- `buildAlerts` keeps only actionable rows in input order and returns `[]` when nothing qualifies.

Integration — `tests/integration/alerts.integration.test.ts` (4 cases, `npm run test:integration`): **17/17 passed** overall.

- Derived alerts are all unpaid, in-window, and their `days`/`kind` match `dayDiff(dueDate, today)`.
- Overdue-alert count equals `dashboard.stats.overdueCount`.
- Overdue alerts match `listImpayes` exactly — same installment-id set and same summed remaining total.
- Settling an overdue tranche in full removes it from the model and shrinks the overdue set by one.

E2E — `tests/e2e/run.mjs`, 4 new Alertes scenarios (`npm run test:e2e`): **20/20 passed, 0 console errors**.

- Three summary tiles render; on the default "all" tab the table row count equals the summed tile counts.
- The overdue tile value equals the sidebar warning badge (both = overdue installment count).
- Clicking the Overdue tile activates the "En retard" tab, narrows rows to the overdue count, and every visible row shows an overdue timing + late-row highlight.
- Clicking a row navigates to the matching purchase-detail page (header title = purchase reference).

### Issues found

None. No product defects surfaced; type-check (`vue-tsc --noEmit`) is clean.

### Recommendations

- The 7-day "due soon" horizon is currently a constant (`DEFAULT_SOON_DAYS`). If shop owners want a configurable window, promote it to a setting later — not needed now.
- The E2E table-total invariant (`rows === overdue + dueToday + dueSoon`) is only meaningful while the seed reliably produces overdue rows; the due-today / due-soon buckets may be empty depending on the seed's relative dates, which is expected and not asserted as non-zero.
- `alerts.integration.test.ts` cross-checks the derived model against the dashboard and impayés; if the Rust backend's overdue definition ever diverges from the TS `dayDiff` logic, this suite will catch it.

---

## 2026-07-22 — Bug: overdue (Impayés) page empty under `tauri dev`

### Summary

Investigated a report that the overdue page renders correctly under `npm run dev`
but shows nothing under `npm run tauri dev`. Root cause found and fixed: a
parameter-binding bug in the Rust `build_impayes` command that made the SQL
query fail at runtime whenever **no filter** was applied — which is the default
state on page load. The browser build was immune because it uses the in-memory
mock (`src/api/mock.ts`) instead of the SQLite-backed Tauri command.

### Test cases run

- **Root-cause reproduction** (temporary diagnostic test against the live DB at
  `~/.local/share/tn.paymentschedule/payment_schedule.db`): `build_impayes` with
  the default filter returned `Err("Wrong number of parameters passed to query.
Got 2, needed 1")` — confirming the command rejected the query and the view
  swallowed it into a blank page. After the fix the same call returned 6 client
  groups / 20 overdue installments, fully serialized.
- **Rust unit suite** (`cargo test`): 3/3 passed, including the new regression
  `commands::tests::build_impayes_binds_params_for_every_filter_combo`, which
  exercises all five filter combinations (none / date_from / date_to / client_id
  / all three) and asserts none error, plus that a seeded DB reports overdue rows.
- **Frontend unit suite** (`npm test`): 27/27 passed (unchanged).

### Issues found

1. **`build_impayes` bound a fixed 4 parameters regardless of the query built** (product bug, fixed).
   - **File:** `src-tauri/src/commands.rs` (`build_impayes`).
   - **Root cause:** the `?2`/`?3`/`?4` placeholders were appended only when the
     matching optional filter was present, but the params vector was always
     built with four entries (`today`, `date_from`, `date_to`, `client_id`), so
     the bound-parameter count didn't match the query's declared placeholders.
     With no filter the query declares only `?1`, so SQLite rejected it.
   - **Symptom:** `list_impayes` (and by extension the dashboard's overdue panel)
     returned an error; `ImpayesView.onMounted` awaits `api.listImpayes()` with no
     `try/catch`, so on rejection `loading` stays `true` and the page renders the
     empty card list with no data and no error — a silent blank.
   - **Fix:** build the params vector in lockstep with the placeholders, pushing a
     value only when its clause is added and numbering `?n` sequentially.
   - **Reproduce (before fix):** `npm run tauri dev` → open **Impayés** → blank
     page despite overdue installments existing in the DB. `npm run dev` shows
     them correctly (mock path).

### Recommendations

- **Surface command errors in the UI.** `ImpayesView` (and any view calling the
  API in `onMounted` without a `catch`) should trap rejections and show an error
  state / toast instead of hanging on `loading = true`. This bug was invisible
  precisely because the error was swallowed. Other views should be audited for
  the same pattern.
- **Add integration coverage against the real command path.** Existing
  `src/views/impayes-overdue.test.ts` and the integration suite exercise the mock
  (`src/api/mock.ts`), so they could not catch a Rust-side SQL defect. Consider a
  Rust-level integration test (like the regression added here) for each command
  that assembles SQL dynamically — `list_impayes`, `list_clients`, and any other
  builder that conditionally appends clauses/params.
- **Prefer named parameters** (`:from`, `:to`, `:client`) over positional `?n`
  for dynamically-assembled queries so a missing clause can't desynchronize the
  binding count.

---

## 2026-07-21 — Full test-suite execution (unit + integration + E2E)

### Summary

Executed all three test layers. Unit and integration suites were green on the
first run. The E2E suite surfaced **one pre-existing test defect** (not a product
bug), which was fixed; the suite is now fully green.

### Test cases run

- **Unit** (`npm test`): 27/27 passed (2 files).
- **Integration** (`npm run test:integration`): 13/13 passed (2 files) — purchase lifecycle + overdue/dashboard/cascade flows.
- **E2E** (`npm run test:e2e`): 16/16 passed after the fix below (initially 15/16).

### Issues found

1. **E2E locator ambiguity — `new purchase: auto-split installments and sum-mismatch validation`** (test defect, fixed).
   - **Symptom:** Playwright strict-mode violation — `getByRole('button', { name: 'Nouvel achat' })` resolved to 2 elements.
   - **Root cause:** the `/achats` page shows two legitimate "Nouvel achat" buttons — a permanent one in the sidebar (`AppSidebar.vue`) and one in the Achats view. The app is correct; the test's locator was under-scoped. Pre-existing (its failure screenshot was already present at session start), not a regression from relocating `e2e/` → `docs/e2e/`.
   - **Fix:** scoped the click to the main region — `page.getByRole("main").getByRole("button", { name: "Nouvel achat" })` in `tests/e2e/run.mjs`.
   - **Reproduce (before fix):** `npm run test:e2e` → the named test fails on the button click.

No product defects found.

### Recommendations

- Wire `npm test`, `npm run test:integration`, and `npm run test:e2e` into CI (in that order) so the E2E stage runs headless on each PR.
- Consider a shared helper for "open the new-purchase modal" so future callers can't re-introduce the ambiguous-locator class of bug.

---

## 2026-07-21 — Integration test suite for the api/backend flows

### Summary

Added an opt-in **integration** test layer that sits between the fast unit tests
(`src/**`) and the browser E2E suite (`tests/e2e/run.mjs`). The new suites drive
the real `api` facade (`src/api/index.ts`) against the in-memory backend
(`src/api/mock.ts`) across multi-command flows, verifying that the api → mockDb →
finance layers stay consistent with one another. The E2E directory was also
relocated from `e2e/` to `docs/e2e/`, and the delivery workflow now mandates
maintaining this report.

Each integration test re-seeds a fresh backend (via `vi.resetModules()`), so the
6-client / 8-purchase seed is identical and isolated per case.

### Test cases — written, NOT run (awaiting confirmation)

Per the QA workflow, integration tests are not executed automatically. Run them
with `npm run test:integration`.

`tests/integration/purchase-lifecycle.integration.test.ts`

- Auto-split of a total across installments matches `splitAmounts` (1000/3 → 333/333/334) and starts fully `pending`.
- Caller-supplied uneven split is honoured when the amounts sum to the total.
- Explicit split whose amounts don't sum to the total is rejected (`SUM_MISMATCH`).
- Installment status transitions pending → partial → paid; purchase moves pending → in_progress.
- Purchase flips to `paid` once every installment is settled; payment ledger totals reconcile.
- Non-positive payment amount is rejected (`INVALID_AMOUNT`).
- Creating a purchase and paying it bumps the dashboard's purchase/sales/collected/outstanding aggregates correctly.

`tests/integration/overdue-dashboard.integration.test.ts`

- Dashboard aggregates reconcile with `listImpayes`, `listClients`, `listPurchases`, `listAllPayments` on the seed.
- `ImpayeFilter` by `clientId` narrows to one client and preserves that client's installments.
- Impossible date window yields an empty overdue list.
- Paying an overdue installment in full removes it from impayés and decrements the dashboard overdue count.
- Unforced delete of a client with purchases is refused (`CLIENT_HAS_PURCHASES:n`) and mutates nothing.
- Forced delete cascades: client, its purchases, and its overdue rows disappear; dashboard purchase count updates.

### Issues found

None — this pass added coverage; no product defects were surfaced (tests not yet executed).

### Recommendations

- Execute `npm run test:integration` to confirm the suites pass, then wire both `npm test` and `npm run test:integration` into CI ahead of the Playwright E2E stage.
- Consider a component-level integration layer (mounting filterable list views with Pinia + i18n) if UI-wiring regressions become a concern; current integration coverage stops at the api facade.
