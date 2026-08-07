# Architecture

## Overview

paymentSchedule is a **Tauri 2** desktop app: a Rust **core process** owns all state
and persistence, and a **Vue 3 WebView** renders the UI. The two communicate
only through typed Tauri **commands** (request/response IPC). The frontend has no
direct database or filesystem access.

```
┌───────────────────────────────────────────────┐
│                WebView (Vue 3)                 │
│  Views ── Pinia stores ── composables          │
│                 │                              │
│         src/api/index.ts (gateway)             │
│         ├─ Tauri: invoke("command", args)      │
│         └─ Browser: src/api/mock.ts (in-mem)   │
└───────────────────│───────────────────────────┘
                    │  IPC (invoke)
┌───────────────────▼───────────────────────────┐
│               Core process (Rust)              │
│  lib.rs ── commands.rs ── db.rs ── seed.rs     │
│                 │                              │
│    rusqlite  →  payment_schedule.db (SQLite)   │
│         app-data dir  →  logo.<ext>            │
└────────────────────────────────────────────────┘
```

## Frontend (`src/`)

- **`main.ts`** — bootstraps Pinia, vue-i18n, vue-router; loads settings and
  applies locale/direction before mount.
- **`App.vue` + `components/layout/`** — the shell: `AppSidebar`, `AppHeader`,
  content area, and toasts.
- **`views/`** — one component per route (Dashboard, Achats, PurchaseDetail,
  Clients, ClientDetail, Paiements, Echeances, Impayes, Settings, Alertes,
  Rapports, NotFound — the router's catch-all).
- **`components/`** — reusable UI (`ui/`: buttons via CSS, `BaseModal`,
  `StatusBadge`, `KpiCard`, `EmptyState`, `ConfirmDialog`, `AppIcon`,
  `SortHeader`) and feature components (`dashboard/*`, `EditInstallmentModal`,
  `NewPurchaseModal`, `ClientForm`).
- **`stores/`** — Pinia: `settings` (language/currency/date/logo, OS-locale
  detection), `stats` (sidebar badge counters), `ui` (toasts, sidebar toggle,
  header-title override).
- **`composables/`** — `useFormat` (locale-aware money/date/number formatting,
  reactive to the settings store), `useSort` (client-side, direction-toggling
  table sorting driven by `SortHeader`), `useBack` (returns to the real
  previous page, falling back to a list route on a deep link or when the previous
  entry is itself an unknown URL), `useContactActions` (validates a client phone
  number and hands a `tel:`/`sms:` URI to the OS, toasting on failure), and
  `useClickOutside` (dismiss popovers/menus — used by the header language menu,
  `DatePicker`, and `DateRangeFilter`). UI filter pieces live in `ui/`:
  `DatePicker` (calendar), `DateRangeFilter` (date popover), `ListFilterBar`
  (reference/client/amount/date bar).
- **`lib/finance.ts`** — pure, unit-tested installment/payment math (the TS
  mirror of `db.rs`), reused by the browser mock.
- **`i18n/`** + **`locales/{ar,fr,en}.json`** — all UI strings; RTL applied via
  `dir="rtl"` on `<html>` for Arabic.
- **`api/`** — `index.ts` is the single typed gateway. It calls Tauri `invoke`
  in the desktop app, or `mock.ts` (a faithful in-memory reimplementation) in a
  plain browser, so previews and tests run without the Rust runtime.

## Backend (`src-tauri/src/`)

- **`lib.rs`** — Tauri builder: registers plugins (single-instance, log, os,
  dialog, opener), opens/seeds the DB into managed state, and registers every
  command. `single-instance` is registered **first** and must stay there — it
  has to win before anything opens the database, since two processes on one
  SQLite file is the classic corruption window. The
  `opener` plugin exists so `tel:`/`sms:` URIs go to the OS default handler —
  navigating the WebView to them instead destroys the SPA. Its capability is
  scoped to those two schemes only. There is deliberately **no `fs` plugin and
  no `fs:*` permission**: the database lives in the app-data dir, so granting
  the WebView filesystem access there would contradict the "no direct frontend
  access" rule. The asset protocol is scoped to `$APPDATA/logo.*` — the single
  file the renderer is allowed to read.
- **`commands.rs`** — the full API surface (`#[tauri::command]`): clients,
  purchases, installments, payments, impayés, schedule, dashboard, settings,
  logo, backup. Every command is `async` so Tauri runs it on the async runtime
  instead of inline on the IPC/main event-loop thread; none of them `await`, so
  the connection guard never spans a suspension point. Each validates its
  arguments, locks the shared connection, and — for the mutating commands —
  delegates to a `*_impl` free function taking `&Connection` — or
  `&mut Connection` where the command needs `conn.transaction()` — which is what
  makes them testable without a Tauri `State`. A read gets the same treatment
  when its query is easy to get subtly wrong (`list_clients_impl` and its scope
  predicate).
- **`error.rs`** — `AppError`, the typed error surface. See "Error contract".
- **`db.rs`** — connection wrapper (`Mutex<Connection>`), the migration ladder,
  the validation bounds, and shared date/status/split helpers. `Db::lock()`
  tolerates a poisoned mutex (`into_inner`) so one panicking command cannot
  brick every later one under `tauri dev`.
- **`models.rs`** — serde structs (camelCase payloads) shared with the frontend.
- **`seed.rs`** — first-run Tunisian demo data.
- **`license.rs`** — offline licence validation. See "Licensing".

## Licensing

`license.rs` reads a signed licence from `$APPDATA/license.json` — beside the
database and the logo — and reports a `LicenseStatus` of `Valid` / `Expired` /
`MachineMismatch` / `InvalidSignature` / `Malformed` / `Missing`. The wire format
and the signing recipe are specified in `docs/license-format.md`; the module doc
in `license.rs` is the authoritative copy.

Design points that constrain later work:

- **The gate is in Rust, not the UI.** `require_license` in `commands.rs` refuses
  20 of the 29 commands with `LICENSE_REQUIRED` when the install is unlicensed.
  The frontend mirrors that state, but a check that lived only in the renderer
  would be decoration: the WebView is the user's, and a `v-if` is not a control.
- **The unlicensed baseline is narrow but genuinely usable**: reading clients and
  purchases — list and detail — plus `get_settings`, a language-only
  `update_settings`, and `backup_database`. Losing a licence must never hold a
  shop keeper's own ledger hostage, and language has to stay editable or the
  licence screen could become unreadable. Backup belongs in that same safety
  baseline: it only snapshots records the baseline already lets the user read,
  and gating it withheld the copy a shop most needs — the one taken before
  troubleshooting an expiry. `list_clients`/`list_purchases` **degrade** rather than refuse:
  an unlicensed caller is pinned to the active scope with no server-side search.
- **Filters and sorting cannot be enforced here.** `useSort.ts` reorders rows
  already in the browser and `ListFilterBar.vue` filters in the parent component,
  so the backend never sees either. Disabling them in the UI communicates the
  licence boundary; it does not enforce it. `scope` is the one real exception.
- **The clock watermark** (`license_clock_watermark` in the `setting` table)
  records the latest date the install has seen; a system clock behind it yields
  `ClockTampered` instead of reviving an expired licence. It is deliberately kept
  out of `Settings`/`SettingsPatch`, which are serialized to and written by the
  renderer — the code it defends against.
- **The shop name is licence-attested, not configuration.** `AppSidebar`'s brand
  block shows `licensee`, falling back to the stored `shop_name` setting and then
  to the generic app title. The setting stays in `Settings`/`SettingsPatch` as
  that fallback but is no longer editable from Paramètres: a name the user can
  type is branding, a name the vendor signs is identification. Note the fallback
  chain covers `clockTampered`, `invalidSignature` and `missing` — the statuses
  where no verified payload exists.
- **`LicenseStatus` never crosses IPC**; `LicenseInfo` does. The projection drops
  `Malformed { reason }`, which is parser detail for the log, exactly as
  `AppError::Internal` collapses to an opaque code.
- **The trust anchor is compiled in.** The Ed25519 public key is a constant,
  overridden at build time by `PAYMENT_SCHEDULE_LICENSE_PUBKEY`. It is never
  fetched, read from disk or taken from configuration — a licence check whose key
  is editable is not a check. The default is a development key whose seed is
  published in the module docs; a release build still using it logs a warning.
- **Signature before parse.** The signature covers the base64 payload text
  exactly as it appears in the file, so it is verified before any untrusted JSON
  is decoded, and there is no JSON canonicalization for a signer and a verifier
  to disagree about.
- **First cryptography in the tree**: `ed25519-dalek`, `sha2`, `base64` and
  `machine-uid`. Pure Rust on purpose — `ring` and `sodiumoxide` need a C/asm
  toolchain the Windows build does not have. `sha2` is pinned to the 0.10 already
  present transitively so `cargo deny`'s duplicate-version check stays quiet.
- **Machine binding** hashes the per-OS machine identifier with an app-specific
  salt, so the raw OS UUID never lands in a file the customer can forward.
  `machine_fingerprint()` is public because a bound licence cannot be issued until
  the customer can read their fingerprint off the screen.
- Validation is **fail-closed** and non-panicking throughout: `panic = "abort"` in
  release means a panic on a hostile licence file would take down the whole app.

## Error contract

Commands never send prose across IPC. `AppError` (`error.rs`) splits errors in
two:

- **Actionable** — `Validation` / `Conflict` / `NotFound`. These serialize to a
  stable machine code, optionally with colon-separated parameters:
  `INVALID_AMOUNT`, `CLIENT_HAS_PURCHASES:3`, `SUM_MISMATCH:900:1000`.
- **Internal** — anything the user cannot act on. The detail is written to the
  log and the wire only ever sees the opaque `INTERNAL`. This is the single
  point that stops SQL text, schema names and filesystem paths reaching the
  renderer.

`src/lib/errors.ts` (`toUserMessage`) parses the code, looks up
`errors.<camelCase>` in the active locale, interpolates the parameters, and
falls back to `errors.generic` for anything it does not recognise. The raw error
always goes to `console.error` first.

**Adding a command that can fail means touching all four:** the Rust guard, the
code table in `error.rs`, the `errors.*` key in **all three** locale files, and
the matching `throw` in `src/api/mock.ts` — the mock has to reject the same
inputs or the integration and E2E suites validate behaviour the real backend
does not have.

## Logging

`tauri-plugin-log` writes to stdout and the platform log directory (`Debug` in
dev builds, `Info` in release). Command failures are logged where the error is
constructed (`AppError::internal`), so nothing fails silently.

**Logs carry ids and error codes only.** Client names, phone numbers, addresses,
emails and payment notes are PII under Tunisian loi 2004-63 and must stay out of
them.

## Data model (SQLite)

```
client (1) ──< purchase (1) ──< installment (1) ──< payment
setting (key/value)
```

- FK cascades: deleting a client cascades to its purchases → installments →
  payments. Indices on `purchase.client_id`, `installment.purchase_id`,
  `installment.due_date`, `payment.installment_id`, `client.archived_at`,
  `purchase.archived_at`.

### Retiring a client: archive, never delete

**A client with any purchase can never be deleted.** `delete_client` takes no
`force` and has no escape hatch: it refuses with `CLIENT_HAS_PURCHASES:{n}`,
terminally. The FK cascade above is still in the schema (`delete_purchase`
relies on the chain below it) but is no longer reachable from a client delete.
Hard delete survives only for a client with zero purchases — a typo or a
duplicate. `ClientsView` does not even render the button otherwise, so the
policy is visible rather than something the user discovers by being refused.

The reversible path is `archived_at` on `client`: `NULL` while active, an ISO
`YYYY-MM-DD` stamp once archived. `list_clients` takes a `ClientScope`
(`active` | `archived` | `all`, defaulting to `active`) and applies the
predicate in the `WHERE`. The predicate names `c.` only — putting it on `p.` or
`i.` would degrade both `LEFT JOIN`s into inner joins and silently drop every
client with no purchases.

Two guards keep one invariant true: **an archived client always has a zero
balance.**

- `archive_client` refuses while any installment is unpaid
  (`ARCHIVE_HAS_OUTSTANDING:{remaining}`).
- `create_purchase` refuses an archived client (`CLIENT_ARCHIVED`), so an
  archived client cannot re-acquire a balance over IPC even though the UI's
  picker already only offers active clients.

That invariant is load-bearing: it is the entire reason impayés, the dashboard
and the reports need **no** `archived_at` filter. An archived client contributes
0 outstanding and 0 overdue whether they are filtered out or not, so archiving
can never make money quietly leave a total. Their history stays fully visible in
Achats / Paiements / Échéances and on their detail page — archiving hides the
client from the client list and the new-purchase picker, nothing more.

The deliberate consequence: a client with unpaid installments can be neither
deleted nor archived. Someone who owes you money cannot be made to disappear.

### Retiring a purchase: the opposite rule

Purchases are archivable too (`purchase.archived_at`, `m0003`), but the money
rule is **inverted**. An archived client is settled, so leaving them in the
aggregates changes nothing. An archived purchase has been removed from the
books, so it must **leave every total** — a purchase you deleted is not still
owed. `list_purchases` takes a `PurchaseScope`, and nine read models filter on
`archived_at`: the dashboard's counts and sums, its outstanding/overdue/upcoming
aggregates, `build_impayes`, `list_schedule`, `list_clients_impl` and
`client_outstanding`.

Three of those aggregates query `installment` without ever naming `purchase`,
so they carry an `EXISTS (SELECT 1 FROM purchase pu WHERE pu.id = i.purchase_id
AND pu.archived_at IS NULL)` rather than a join. Miss one and a headline figure
silently disagrees with the list it links to.

**The invariant: an archived purchase carries zero payments.** `archive_purchase`
refuses once any payment exists (`PURCHASE_HAS_PAYMENTS:{n}`) and
`record_payment` refuses an archived purchase (`PURCHASE_ARCHIVED`); there is no
delete-payment command, so this cannot be worked around. It is what lets
`total_collected` — `SELECT SUM(amount) FROM payment`, the hottest aggregate in
the app — skip the filter entirely instead of joining payment → installment →
purchase. It is guarded by a test, not by a redundant join.

Consequences, both intended:

- A purchase with recorded payments is **permanent**: neither archivable nor
  deletable. Real cash was taken against it.
- Deleting a purchase is a two-step. `delete_purchase` refuses anything not
  already archived (`PURCHASE_NOT_ARCHIVED`), so the destructive cascade is only
  reachable from the archive tab, and only deliberately.

### The two editors: one owns the schedule, the other owns the money

Editing is split by _which fields_ it may touch, not by how much has been paid.

**`update_purchase` is the only writer of `amount` and `due_date`.** It always
accepts the product label; everything the schedule derives from — total, count,
interval, and the **purchase date** that anchors it — is resolved into a full
schedule and handed to `apply_schedule_in_place`. `schedule_changed` compares
the _resolved_ schedule against what is stored rather than trusting the presence
of `input.installments`, because the editor always sends the rows it is
displaying, so a label-only edit must not read as a reschedule. `client_id` is
ignored: a purchase cannot change hands.

`apply_schedule_in_place` updates the rows position by position instead of
regenerating them, which is what lets a purchase carrying payments still be
rescheduled — the `payment` ledger hangs off `installment` by an
`ON DELETE CASCADE`, so keeping the rows keeps the history. Three rules decide
whether a schedule is acceptable, all checked before anything is written:

- A **settled** row (`paid_amount >= amount`) is history: the incoming schedule
  has to agree with it (`AMOUNT_LOCKED`, `DUE_DATE_LOCKED`). This is what makes
  a paid installment's amount and due date immutable from anywhere, since no
  other command writes them.
- No row may fall below what it has collected (`BELOW_PAID:{paid}`), because
  `amount - paid_amount` feeds every outstanding aggregate and must not go
  negative.
- A row may only be **dropped** — by shortening the schedule — while no cash has
  landed on it (`PURCHASE_HAS_PAYMENTS:{n}`).

Note what this leaves open on purpose: a _partially_ paid installment is still
reschedulable, bounded below by its `paid_amount`. Only settling it freezes it.

Because the anchor fields regenerate every row's due date, settled rows
included, changing the count, cadence or purchase date on a purchase with a
settled installment is refused in practice — the UI disables those fields rather
than letting the user discover it. The total price stays open: the difference is
absorbed by the installments still owed.

`resolve_schedule` additionally requires the due dates to run in position order
(`DUE_DATE_OUT_OF_ORDER`). `idx` is what orders installments, but the sequential
money rule below is naturally stated in terms of due dates; the check makes
position order and chronological order provably the same thing, so "the previous
installment" means one thing however the dates are edited. It is shared by
create and update, so the two cannot drift.

### Editing one installment: money only

`update_installment` updates one row in place and deals **only** in money —
`paid_amount`, `payment_date`, `note`. An `amount` or `due_date` sent here is
refused with `SCHEDULE_VIA_PURCHASE` whatever its value, and whatever the
installment's state. Refusing on _presence_ rather than on "differs from what is
stored" is deliberate: a caller sending the field still believes this command
owns one, and a no-op today is a real edit after the next keystroke.

That refusal is what makes "the schedule is edited in one place" a property of
the backend rather than a habit of the UI. It also means a schedule change is
always judged against the _whole_ schedule — the sum, the ordering and every
settled row at once — instead of one row at a time.

Two rules govern what is left:

- **`paid_amount`** is editable only once installment `N-1` is fully paid
  (`PREVIOUS_UNPAID:{index}`). Cash is collected in order, so it cannot be
  recorded out of order. Nothing about _this_ installment's own status gates it:
  a settled row's collected figure stays correctable, which is the half of the
  immutability rule that stays open on purpose.
- **A recorded payment date is history** (`PAYMENT_DATE_LOCKED`). A date may only
  be supplied to date the ledger entry this edit is about to create, so an entry
  already on record can never be re-dated. Setting one the first time is exactly
  what recording a payment is, so that path is untouched. A note carries no such
  history and may still amend the latest entry — but with no entry at all,
  either is refused with `NO_PAYMENT_TO_DATE` rather than silently dropped.

One invariant survives it, and it is the reason for the correction entry.

**`SUM(payment.amount) == SUM(installment.paid_amount)`.** `paid_amount` is a
denormalised cache of the ledger — `record_payment` only ever increments it, in
the same transaction as its `INSERT INTO payment`. The dashboard's
`total_collected` is the single money figure in the app read from the ledger
itself; every other paid/remaining/outstanding figure reads `paid_amount`. So an
edit that moved `paid_amount` alone would make that one tile disagree with every
purchase and client total. Instead the editor writes a **correction entry**: one
`payment` row for the difference, carrying the caller's date and note, negative
when the figure comes down. `paid_date` then stays derived (`sync_paid_date`,
`MAX(payment_date)`), re-run for the edited row and for every absorber a
rebalance pushed across its settled threshold. A date or a note with no
correction to carry it is refused with `NO_PAYMENT_TO_DATE` — never silently
dropped.

The visible cost is that a downward correction shows as a **negative line in the
Paiements log** and inside its amount-range filter. That is the honest reading:
the money came back.

`paid_amount` is additionally capped at the installment's amount
(`PAID_ABOVE_AMOUNT:{amount}`). It is the same invariant `record_payment`'s
`OVERPAYMENT` guard protects: `SUM(i.amount - i.paid_amount)` feeds the
outstanding and overdue aggregates, and one negative row cancels out another
client's real debt. The mirror-image constraint from the schedule side is
`BELOW_PAID:{paid}`, raised by `apply_schedule_in_place`.

`sync_paid_date` is reached from both editors and re-derives `paid_date` as
`MAX(payment_date)` whenever a row crosses its settled threshold — from either
direction, since `update_installment` moves the collected figure under a fixed
amount and `apply_schedule_in_place` moves the amount under a fixed figure.

Nothing writes `status`: it is derived on read, so zeroing an untouched
installment reads as "paid" and lowering a settled one's collected figure puts it
back in debt with no extra bookkeeping.

One asymmetry worth knowing: `rebalance_amounts` in `db.rs` no longer has a Rust
caller, because a schedule now arrives whole and its sum is checked outright
rather than a single-row delta being absorbed. It stays as the parity anchor for
`rebalanceAmounts` in `finance.ts`, which the purchase editor still runs to
redistribute its rows as they are typed — the shared fixture is what proves the
two agree, and deleting the Rust half would leave it checking nothing.

- **Queries name their columns.** No `SELECT *`: the payment queries join four
  tables and `map_payment` resolves columns by name, so a star would let a new
  `payment.reference` or `payment.purchase_id` column silently shadow the
  purchase's value — wrong data on screen with no error anywhere.
- **Money** is stored as whole currency units (`INTEGER`) so the installment
  split is exact. **Dates** are ISO `YYYY-MM-DD` text.
- **Installment status** is derived on read (`paid`/`partial`/`late`/`pending`)
  from `paid_amount`, `amount`, and `due_date` vs today — no scheduled job needed
  to flip installments to "late".
- **Connection PRAGMAs** (`Db::open`): `journal_mode = WAL`,
  `busy_timeout = 5000`, `synchronous = NORMAL`, `foreign_keys = ON`. The last
  one is per-connection and the cascades depend on it.
- **`paid_amount` can never exceed `amount`** — `record_payment` rejects
  overpayment with `OVERPAYMENT:{remaining}`. The invariant matters because
  `amount - paid_amount` is summed directly into the outstanding and overdue
  aggregates, so a negative row would cancel out another client's real debt.

### Schema versioning

`db.rs` holds an **append-only** `MIGRATIONS` slice; the index in it is the
version, recorded in `PRAGMA user_version`. Each step runs in its own
transaction together with the version bump, so a failure cannot leave a
half-applied schema recorded as complete.

Databases created before versioning existed sit at `user_version = 0` with the
v1 tables present. That is safe only because `m0001_initial_schema` is the
historical `CREATE TABLE IF NOT EXISTS` batch verbatim — re-running it is a
no-op that stamps the version on. **Never reorder or edit a step that has
shipped**; append a new one.

**Shipped steps must also be additive** — add tables, add columns, add indexes;
no `DROP`, no rename, no 12-step table rebuild. Both steps to date are, and this
is why it matters: `migrate` refuses to open a database whose `user_version` is
ahead of the binary, and `Db::open` is propagated with `?` from `setup`, so the
previous installer does not merely misread a rebuilt schema — it will not
launch. A destructive step therefore makes "reinstall the old version" stop
being a recovery option for every user who has already upgraded. Nothing in CI
enforces this; it is a review rule.

**Every step must be replay-safe, not just `m0001`.** `m0002_client_archive` was
the first appended step, and it has to check `pragma_table_info` before its
`ALTER TABLE ADD COLUMN` — SQLite has no `ADD COLUMN IF NOT EXISTS`, and a blind
`ALTER` against a database that already has the column fails the whole
`Db::open`, taking the app down on launch. `migrate_is_versioned_and_idempotent`
replays the ladder from zero and asserts the column count, which is what catches
this.

### Backup

`backup_database` uses `VACUUM INTO`, not a file copy: a copy would race an
in-flight write and, in WAL mode, miss everything still in the `-wal` file.
It is the only recovery path in the app — client deletes cascade through
purchases, installments and payments and cannot be undone.

Three things follow from it being the _only_ recovery path:

- **The snapshot is verified before it is accepted.** `VACUUM INTO` returning
  `Ok` proves the statement ran, not that the file it wrote is a usable
  database; a full disk or a failing drive lands here as success. The staged
  file is opened and put through `PRAGMA integrity_check` and
  `PRAGMA foreign_key_check` before the rename, so a bad snapshot fails loudly
  now instead of silently at restore time.
- **It carries no licence gate**, unlike every other write. See the licensing
  section: a snapshot of records the unlicensed baseline already displays
  protects nothing, and the gate withheld the copy a shop most needs.
- **Success writes `last_backup_at` to the `setting` table**, and the command
  returns the updated `Settings` so the Settings page can show when the last
  backup happened and nudge after `BACKUP_STALE_DAYS`. Nothing schedules a
  backup, so that nudge is the only thing that ever raises the subject.

Restoring is a manual file operation, documented in `README.md` — there is no
`restore_database` command.

### Automatic snapshots

`autobackup.rs` takes two snapshots at launch, both through
`backup_database_impl` unchanged, so both inherit its verification:

- **Before a pending migration.** `db::pending_migration` is consulted _before_
  `Db::open`, and answers `Some` only when the file exists, carries a `client`
  table and is behind `MIGRATIONS.len()` — so a fresh install is never affected.
  On `Some`, `backups/payment_schedule.pre-v{n}.db` is written first. **If that
  write fails the app refuses to migrate**: it records the reason, hides the
  window, and `RunEvent::Ready` shows a native dialog before exiting. The user
  can free disk space; they cannot undo a bad migration.
- **On a schedule the shop sets** — 17:00 daily out of the box, with frequency
  (daily/weekly/monthly) and time in Settings. `backups/auto-{date}.db`, guarded
  by `last_auto_backup_at`. Never fatal: a failure costs a backup, not the
  working day, because no irreversible step is waiting behind it.

`autobackup::due` is the single predicate behind both the scheduler tick and the
launch pass, so no second rule can disagree with it. It has two branches, and
the first is what makes a time of day mean anything on a desktop app:

```
due = enabled AND ( elapsed >  interval                       // overdue → now
                    OR (elapsed >= interval AND now >= time) )// today's window
```

A shop that runs the app 09:00–16:00 is never open at 17:00. Under a plain
"fire at the scheduled time" rule they would never be backed up at all, and the
failure would be silent until the day they needed the copy; the overdue branch
catches them the next morning instead. The same branch is the launch catch-up.

The scheduler is a plain `std::thread` polling every 60 s, not a computed sleep
until the next occurrence: a poll is the only shape that survives the user
changing the time, the machine suspending, and the clock moving. It re-reads the
settings every pass, so an edit takes effect within a minute with no restart. A
failed attempt is held off for an hour — otherwise a full disk means a
`VACUUM INTO` attempt and a log line every minute, none of which can succeed.

The scheduled snapshot opens **its own connection** rather than taking the
shared `Mutex<Connection>`. At launch the mutex was free, but 17:00 lands while
the shop is typing; in WAL mode a second connection reads a consistent snapshot
without blocking writers.

The schedule is licensed configuration, like the currency or the alert window —
an expired install cannot re-time it, but the backups keep running on whatever
is stored and the manual button carries no gate at all.

Pruned as two pools, `auto-*` to 5 and `pre-v*` to 2, so daily snapshots can
never evict the copy taken before a schema change.

The dialog is the one piece of user-facing text this crate owns, in `lib.rs`
rather than `src/locales/*.json`: it fires before the WebView exists, so
vue-i18n cannot supply it. It reads `language` from `setting` and falls back to
French.

**These are not a substitute for the manual backup and never clear its staleness
nudge.** `backups/` sits beside `payment_schedule.db` on the same disk: one
failed drive or one stolen machine takes both. They defend against a bad
migration and a mistaken delete, not against losing the computer, which is why
`last_auto_backup_at` is reported separately from `last_backup_at` and why
`backupIsStale` reads only the latter.

## Key decisions

- **`rusqlite` behind commands** (not `tauri-plugin-sql`) so the requirement
  "all persistence through Rust commands, never direct frontend access" holds.
- **Browser mock backend** keeps the app fully functional without Tauri, which
  enables headless UI verification (Playwright screenshots) and unit tests.
- **Design tokens** (`src/style.css` CSS variables) extracted from the reference
  mockup drive every screen — including the mirrored Arabic RTL layout — for
  visual consistency.
- **Tables are wrapped in `.table-scroll`** (`src/style.css`). `.card` sets no
  `overflow` and `.app-content` scrolls the whole page, so an unwrapped table
  wider than its card paints through the card border and drags the page into
  horizontal scroll. The wrapper keeps the overflow local; every new table needs
  it.
