# Features

Status legend: ✅ done · 🟡 partial / placeholder · ⬜ planned

| Feature | Status | Description |
|---------|:------:|-------------|
| **Dashboard (Tableau de bord)** | ✅ | KPI row (purchases, sales, collected, outstanding, late), recent purchases, featured purchase detail, due-date alerts, overdue panel. Pixel-matched to `docs/intsallment.png`. |
| **Achats (Purchases)** | ✅ | Searchable list; new-purchase form with inline client creation, auto-split installments, per-line amount override with live sum check, configurable interval (weekly/monthly/custom). |
| **Purchase detail** | ✅ | Full installment schedule, per-tranche payment recording, payment history, running balance. |
| **Clients** | ✅ | CRUD with validation; overdue badges; delete safeguard when purchases exist; client detail page (purchases, payment history, outstanding). |
| **Paiements (Payments)** | ✅ | Per-installment recording with partial-payment support; global payment log; per-purchase and per-client history. |
| **Échéances (Due dates)** | ✅ | Full schedule with all/overdue/upcoming/paid filters; overdue rows highlighted. |
| **Impayés (Overdue)** | ✅ | Clients with overdue installments; date-range + client filters; per-client contact shortcuts (call/SMS/view); CSV export. |
| **Settings (Paramètres)** | ✅ | Language, currency, date format, shop logo upload, shop name/info — persisted in SQLite, applied live. |
| **Internationalization** | ✅ | Arabic (RTL), French, English. OS-locale default → French fallback. Full layout mirroring for Arabic. |
| **Localized formatting** | ✅ | Currency (default TND) and date format (default dd/MM/yyyy) applied everywhere; locale-aware number grouping. |
| **Logo management** | ✅ | Upload/replace/remove; stored in app-data dir; shown in sidebar/header. |
| **Offline SQLite storage** | ✅ | All data local via `rusqlite` behind Tauri commands; seeded demo data on first run. |
| **Empty states & validation** | ✅ | Localized empty states and form validation across all modules. |
| **Alertes** | 🟡 | Sidebar entry + styled placeholder page. Live alerts already surface on the dashboard; dedicated page logic deferred. |
| **Rapports (Reports)** | 🟡 | Sidebar entry + styled placeholder page. Exportable reporting deferred. |
| **Windows `.msi`/`.exe` build** | 🟡 | Bundler configuration present; must be built on Windows (see README). |

## Cross-cutting

- **Keyboard accessibility:** forms are tab-navigable, Enter submits, Esc closes modals.
- **Responsive:** grid layouts collapse below ~1200px; minimum window size enforced (1024×680).
- **Toasts:** success/error feedback on all mutations.

## Testing

- **Unit tests:** `npm test` (Vitest) — finance helpers in `src/lib/finance.test.ts`.
- **End-to-end tests:** `npm run test:e2e` — self-contained Playwright suite (`e2e/run.mjs`) drives the real app in headless Chromium against the in-memory mock backend. Spawns its own Vite server on port 5199 and pins the browser locale to French for determinism. Covers app-shell render, dashboard KPIs, full sidebar navigation, client list + create flow, purchase list + search, and the overdue (impayés) page. Failures capture a full-page screenshot under `e2e/artifacts/`.
