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
  entry is itself an unknown URL), and
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

- **`lib.rs`** — Tauri builder: registers plugins (os, dialog, fs), opens/seeds
  the DB into managed state, and registers every command.
- **`commands.rs`** — the full API surface (`#[tauri::command]`): clients,
  purchases, installments, payments, impayés, schedule, dashboard, settings,
  logo. Each locks the shared connection and returns serde models.
- **`db.rs`** — connection wrapper (`Mutex<Connection>`), schema migration,
  and shared date/status/split helpers.
- **`models.rs`** — serde structs (camelCase payloads) shared with the frontend.
- **`seed.rs`** — first-run Tunisian demo data.

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

## Key decisions

- **`rusqlite` behind commands** (not `tauri-plugin-sql`) so the requirement
  "all persistence through Rust commands, never direct frontend access" holds.
- **Browser mock backend** keeps the app fully functional without Tauri, which
  enables headless UI verification (Playwright screenshots) and unit tests.
- **Design tokens** (`src/style.css` CSS variables) extracted from the reference
  mockup drive every screen — including the mirrored Arabic RTL layout — for
  visual consistency.
