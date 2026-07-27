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
  `SortHeader`) and feature components (`dashboard/*`, `PaymentModal`,
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
  delegates to a `*_impl` free function taking `&Connection`, which is what
  makes them testable without a Tauri `State`.
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
  `installment.due_date`, `payment.installment_id`.
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
