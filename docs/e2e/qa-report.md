# QA Report — paymentSchedule

This file is the durable QA record for the project. Each QA pass appends a new
dated section (most recent first). Format per entry: **Summary → Test cases →
Issues found → Recommendations**. See `CLAUDE.md` (Phase 3: QA) for the workflow.

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
