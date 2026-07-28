# QA Report — paymentSchedule

This file is the durable QA record for the project. Each QA pass appends a new
dated section (most recent first). Format per entry: **Summary → Test cases →
Issues found → Recommendations**. See `CLAUDE.md` (Phase 3: QA) for the workflow.

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
