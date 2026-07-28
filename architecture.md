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

Editing follows the same logic. `update_purchase` always accepts the product
label; changing the total, count, interval or the **purchase date** regenerates
the installment rows — and those rows own the payments through an
`ON DELETE CASCADE` — so it is refused once a payment exists.
`schedule_changed` compares the _resolved_ schedule against what is stored
rather than trusting the presence of `input.installments`, because the editor
always sends the rows it is displaying. `client_id` is ignored: a purchase
cannot change hands.

### Editing one installment: the path that survives a payment

`update_purchase` going hard-locked at the first payment leaves a real gap —
pushing one due date back a week, or re-cutting the tranches a client
renegotiated, only ever happens _after_ payments have started.
`update_installment` fills it by updating rows in place. It regenerates nothing,
so it never destroys the payments hanging off those rows, and it is the **only**
installment editor: it absorbed the payment modal, so both what is owed and what
has been collected move through it.

Its fields split into two halves under opposite rules, and neither half's rule
looks at the other's:

- **The schedule** — `amount` and `due_date` — is editable until the installment
  settles, after which it is history (`AMOUNT_LOCKED`, `DUE_DATE_LOCKED`).
  Nothing about the neighbouring installments gates it.
- **The money** — `paid_amount`, `payment_date`, `note` — is editable only once
  installment `N-1` is fully paid (`PREVIOUS_UNPAID:{index}`). Cash is collected
  in order, so it cannot be recorded out of order. Nothing about _this_
  installment's own status gates it.

Two invariants survive it, and each is the reason for one of the guards.

**`SUM(amount) == purchase.total_price`.** The total is never written; a changed
amount is absorbed by the other unsettled installments (`rebalance_amounts` in
`db.rs`, mirrored by `rebalanceAmounts` in `finance.ts` and covered by the shared
parity fixture). The delta lands on the installments _after_ the edited one and
falls back to the earlier unsettled ones only when there is nothing later; a
fully-paid installment is never an absorber, since its amount is settled history.
When no absorber set can take the change the edit is refused with
`NO_REBALANCE_ROOM` rather than the total quietly moving. The deliberate
consequence: once every _other_ installment is settled, this one's amount is
locked.

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
correction to carry it amends the row's latest entry instead, and is refused with
`NO_PAYMENT_TO_DATE` when there is no entry at all — never silently dropped.

The visible cost is that a downward correction shows as a **negative line in the
Paiements log** and inside its amount-range filter. That is the honest reading:
the money came back.

`paid_amount` is additionally capped at the installment's amount
(`PAID_ABOVE_AMOUNT:{amount}` / `BELOW_PAID:{paid}` — the same constraint from
either side, reported against whichever field the user moved). Both are checked
against the values the edit _lands_ on, not the stored ones, so lowering the
amount and the collected figure together is not refused for a conflict the
request itself resolves. It is the same invariant `record_payment`'s
`OVERPAYMENT` guard protects: `SUM(i.amount - i.paid_amount)` feeds the
outstanding and overdue aggregates, and one negative row cancels out another
client's real debt.

Finally, a due date is clamped to `[prev.due_date, next.due_date]`
(`DUE_DATE_OUT_OF_ORDER`; the outer installments are unbounded on their missing
side). `idx` is what orders installments, but the sequential money rule is
naturally stated in terms of due dates — the clamp makes position order and
chronological order provably the same thing, so "the previous installment" means
one thing however the dates are edited.

Nothing writes `status`: it is derived on read, so zeroing an untouched
installment reads as "paid" and lowering a settled one's collected figure puts it
back in debt with no extra bookkeeping.

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
