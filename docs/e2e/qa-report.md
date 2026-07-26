# QA Report — paymentSchedule

This file is the durable QA record for the project. Each QA pass appends a new
dated section (most recent first). Format per entry: **Summary → Test cases →
Issues found → Recommendations**. See `CLAUDE.md` (Phase 3: QA) for the workflow.

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
