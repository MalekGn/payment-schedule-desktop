# paymentSchedule — Development Audit Report

**Date:** 2026-08-04
**Commit audited:** `b38a402` (branch `dev`, working tree clean)
**Scope:** architecture, security, dependencies, data layer, frontend, build config, hygiene
**Nature:** read-only review. **No code was changed.** The only file written is this report.

> **Update:** findings **H2**, **M5**, **H3** and **M1** were closed after this
> report was written — commits `ce038d1`, `9fb54f4`, `06b9165` and the
> schedule-bounds commit that follows them.
> Their rows below are annotated inline; everything else stands as audited.
>
> This report **replaces** the 2026-07-26 audit (commit `641c7ff`, which audited
> `eb86a6a`). That version is recoverable with
> `git show 641c7ff:AUDIT_REPORT.md`. Its findings were closed by `1a474a1` and
> `37187e2`; §0 records what actually happened to each one, so the history is
> not lost by rewriting.

**Stack as actually found**

| Layer    | What is there                                                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Shell    | **Tauri 2** (`tauri 2.11.5`, `tauri-build 2.6.3`, config `$schema: schema.tauri.app/config/2`) — current with crates.io               |
| Backend  | Rust, edition 2021, MSRV **1.88** (CI-proven), `src-tauri/src/{lib,main,commands,db,models,error,seed,license}.rs`                    |
| Database | **`rusqlite 0.39.0`, `features = ["bundled"]`** (SQLite compiled into the binary). No SQL plugin, no ORM, no `@tauri-apps/plugin-sql` |
| Frontend | **Vue 3.5** + TypeScript, `<script setup>` in **34/34** SFCs, Pinia 2 (4 stores), vue-router 4, vue-i18n 10                           |
| Build    | Vite 7.3.6, `vue-tsc --noEmit` typecheck, ESLint 10 flat config, Prettier 3, husky + lint-staged                                      |
| CI       | 3 GitHub Actions workflows (release bundles, CodeQL, security audit), Dependabot on npm/cargo/actions, all actions SHA-pinned         |
| Tests    | Vitest unit (10 files / 147 cases), Vitest integration (8 files / 99 cases), Playwright E2E (50 scenarios), **126 Rust tests**        |

**Local gate status at audit time** (every one actually run, results real):

| Gate                                        | Result                                                                                                    |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `npm run lint`                              | ✅ clean                                                                                                  |
| `npm run build` (`vue-tsc --noEmit` + vite) | ✅ clean                                                                                                  |
| `npm test`                                  | ✅ 10 files, 147 cases passed                                                                             |
| `cargo fmt --check`                         | ✅ clean                                                                                                  |
| `cargo clippy --all-targets -- -D warnings` | ✅ clean                                                                                                  |
| `cargo test`                                | ✅ 126 passed — CI job added since (H2 closed)                                                            |
| `cargo audit`                               | ✅ **0 vulnerabilities**, ⚠️ 18 unmaintained/unsound warnings (all transitive via GTK3)                   |
| `npm audit --audit-level=high`              | ❌ **exit 1** — 1 high advisory. _This is the gate in `.github/workflows/security.yml`; it is red today._ |

---

## 0. Status of the 2026-07-26 audit

30 findings were raised then. Verified against the current tree:

| Old # | Finding                                   | Status today                                                                                                                    |
| ----- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| 1     | `fs:*` over `$APPDATA/**`, DB in scope    | ✅ **Fixed.** No `fs` plugin; no `fs:*` permission; `assetProtocol.scope` narrowed to `$APPDATA/logo.*`                         |
| 2     | `upcoming_days` → `Duration::days` panic  | ✅ **Fixed.** Clamped to `1..=365` (`commands.rs:1660-1662`), rationale recorded in-comment                                     |
| 3     | `interval_days` overflow                  | ✅ **Fixed.** `INTERVAL_DAYS_RANGE 1..=365` + `add_interval` fully overflow-saturating (`db.rs:339-358`)                        |
| 4     | `installment_count` unbounded             | ⚠️ **Partial.** Range check added — but the `installments` array bypasses it. See **H3**                                        |
| 5     | Raw `rusqlite` error text shown to users  | ✅ **Fixed.** `AppError` + stable codes; `Internal` collapses to `"INTERNAL"` (`error.rs:120-127`); frontend maps codes to i18n |
| 6     | `set_logo` unvalidated                    | ✅ **Fixed.** Extension allow-list, `is_file()`, 5 MiB cap, magic-byte sniff (`commands.rs:1990-2023`)                          |
| 7     | Sync commands on IPC thread; N+1 queries  | ⚠️ **Partial.** All 27 commands are now `async` ✅; the N+1 pattern remains (`commands.rs:564-574`). See **I3**                 |
| 8     | No WAL / `busy_timeout` / single-instance | ✅ **Fixed.** All four PRAGMAs set and test-asserted (`db.rs:29-34`); `tauri-plugin-single-instance` registered first           |
| 9     | No schema versioning                      | ✅ **Fixed.** `PRAGMA user_version` ladder, transactional, forward-refusing (`db.rs:96-150`); `backup_database` added           |
| 10    | Manual `due_date` written unparsed        | ✅ **Fixed.** Every manual date goes through `parse_date` (`commands.rs:646-655`)                                               |
| 11    | `update_settings` non-transactional       | ✅ **Fixed.** One transaction over the whole patch (`commands.rs:1903+`)                                                        |
| 12    | Unlimited overpayment                     | ✅ **Fixed.** `OVERPAYMENT:{remaining}` guard (`commands.rs:1332-1335`)                                                         |
| 13    | `total_price` never validated             | ⚠️ **Partial.** `> 0` enforced; still no **upper** bound. See **M1**                                                            |
| 14    | `lock().unwrap()` poisoning               | ✅ **Fixed.** `unwrap_or_else(\|e\| e.into_inner())` (`db.rs:52-54`)                                                            |
| 15    | 8 high npm advisories                     | ⚠️ **Now 1, different package.** Same gate still red. See **H1**                                                                |
| 16    | `rusqlite` 0.32.1 vs 0.40.1               | ✅ **Fixed since.** Now 0.39.0; bundled SQLite 3.46.0 → 3.51.3. See **M5**                                                      |
| 17    | Zero backend logging                      | ✅ **Fixed.** `log` + `tauri-plugin-log`; PII-free by policy (`lib.rs:34-37`), verified                                         |
| 18    | Unhandled load paths                      | ✅ **Fixed.** `useLoader` composable + `LoadError` across all list views                                                        |
| 19    | 3 Rust tests, no backend coverage         | ✅ **Fixed.** 126 Rust tests, and CI now runs them. See **H2**                                                                  |
| 20    | MSRV wrong (1.77 vs 1.88)                 | ✅ **Fixed.** `rust-version = "1.88"` + a dedicated CI job proving it; `engines.node >= 22`                                     |
| 21    | CSV quoting + formula injection           | ✅ **Fixed.** Quote-doubling + `FORMULA_TRIGGER` guard (`src/lib/csv.ts:32,57-58`), 19 unit tests                               |
| 22    | `clear_logo` discards `remove_file` error | ✅ **Fixed.** Now `log::warn!`s                                                                                                 |
| 23    | Unreachable `delete_client` `force` guard | ✅ **Fixed.** Parameter removed; the refusal is terminal                                                                        |
| 24    | `SELECT *` + name-based mapping           | ✅ **Fixed.** Queries name their columns                                                                                        |
| 25    | 17 RustSec unmaintained warnings          | ➖ **Accepted, now 18.** All transitive via GTK3; policy recorded in `deny.toml`                                                |
| 26    | Frontend majors behind                    | ❌ **Still open.** See §4.2                                                                                                     |
| 27    | No licence declared                       | ✅ **Fixed.** `LICENSE` + `license-file` + `"UNLICENSED"` + `deny.toml`                                                         |
| 28    | `.gitignore` missing `*.db`               | ✅ **Fixed.** Covers `*.db`, `-wal`, `-shm`, `-journal`, `.part`, `*.sqlite*`, with a PII rationale                             |
| 29    | CSP `style-src 'unsafe-inline'`           | ➖ **Accepted.** Unchanged and still correct for Vue                                                                            |
| 30    | One `v-html` in `AppIcon.vue`             | ➖ **No change needed.** Re-verified: static `ICONS` map, key lookup only                                                       |

**Score: 22 fixed, 3 partial, 2 still open, 3 accepted.** That is a strong
remediation record. Every finding below is either new, or the residue of a
partial fix.

---

## 1. Executive summary

Ordered most-important first.

1. **The `npm audit --audit-level=high` CI gate is red right now** — one high
   advisory (`brace-expansion` 5.0.8, GHSA-rgw5-rvv9-x895) reached through
   `eslint@10.8.0 → minimatch@10.2.5`. `npm audit fix` resolves it within the
   declared ranges. One command, and the highest-value action in this report.
2. **`cargo test` runs in no CI workflow.** _(Closed in `9f9ad6c`.)_ 126 Rust tests exist — covering the
   money invariants, the licence gate, the migration ladder and most of the
   guards discussed below — and not one is exercised before a release is cut.
   `release` depends on `[test, rust-lint]`, where `test` is JS-only and
   `rust-lint` is fmt + clippy. A regression in `commands.rs` reaches an
   installer unchallenged.
3. **The `installments` array bypasses `INSTALLMENT_COUNT_RANGE`.** _(Closed.)_ The 1..=120
   cap exists, in the code's own words, so a hostile count "cannot drive an
   unbounded `Vec` allocation and insert loop" — but `resolve_schedule` derives
   the row set from `list.len()` and checks only the _sum_. `installmentCount: 1`
   with a million-element array passes validation and reaches a per-element
   `INSERT` loop. Old finding #4 re-opening through a different door.
4. **Money arithmetic can wrap silently in release.** _(Closed.)_ `[profile.release]` sets
   `panic = "abort"` but not `overflow-checks`, and `total_price` / per-line
   `amount` have no upper bound. A crafted amount set can wrap `i64` to equal
   `total_price` and defeat the `SUM_MISMATCH` check outright.
5. **`backup_database` deletes an arbitrary `*.db.part` file with no content
   check.** The careful "must already be a SQLite file" guard protects
   `dest_path` but not the sibling temp path derived from it.
6. **`tests/**` is never typechecked.** `tsconfig.json` includes only `src/**`,
   so `vue-tsc --noEmit` checks 31 files and no test file. The 99-case
   TypeScript integration suite is linted but type-checked by nothing.
7. **Integration and E2E remain outside CI** — 99 integration cases and 50 E2E
   scenarios that run only when someone remembers to.
8. **`rusqlite` is 8 minor versions behind (0.32.1 → 0.40.1)** _(closed in
   `7e592ac`: now 0.39.0, SQLite 3.46.0 → 3.51.3)_, so an older
   SQLite C library is statically compiled into every shipped binary. The one
   dependency gap with a security dimension, and unchanged since the last audit.
9. **No length bound on any user-supplied string.** `ClientInput` and five
   `SettingsPatch` fields are written with `.trim()` and nothing else.
10. **Positives worth stating so they are not "fixed" away**: zero `unsafe`;
    exactly one `expect()` in the whole non-test backend, at startup; no SQL
    injection anywhere; no shell, no network client, no updater; errors cannot
    leak SQL or paths across IPC by construction; licence validation is
    signature-before-parse with `verify_strict`; migrations are transactional and
    forward-refusing; all four SQLite PRAGMAs including `foreign_keys` are set
    _and asserted_; zero TODO/FIXME, zero `any`, zero `@ts-ignore`, zero secrets.

---

## 2. Findings table

| #      | Area              | Severity              | Finding                                                                                                                                                                                                         | Recommendation                                                                                              | File / Location                                                  |
| ------ | ----------------- | --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| **H1** | Dependencies / CI | **High**              | `npm audit --audit-level=high` exits 1: `brace-expansion` 5.0.8 DoS (GHSA-rgw5-rvv9-x895) via `eslint@10.8.0 → minimatch@10.2.5`                                                                                | `npm audit fix` — resolves in-range, no major bump                                                          | `.github/workflows/security.yml:52-53`                           |
| **H2** | CI / Testing      | ~~High~~ **CLOSED**   | ~~No workflow runs `cargo test`. 126 Rust tests never gate a release~~ — fixed in `9f9ad6c`: a `rust-test` job runs `cargo test --locked` and `release` depends on it                                           | Add a `cargo test` job to `build.yml`; add it to `release.needs`                                            | `.github/workflows/build.yml:26-52,54-79,133`                    |
| **H3** | Data integrity    | ~~High~~ **CLOSED**   | ~~`installments` array length is unbounded and never compared to `installment_count`~~ — fixed: `resolve_schedule` refuses with `INSTALLMENT_COUNT_MISMATCH:{sent}:{declared}`                                  | Done                                                                                                        | `commands.rs:605-607`, `632-644`, `686-702`; `db.rs:283-287`     |
| **M1** | Data integrity    | ~~Medium~~ **CLOSED** | ~~`total_price` and `InstallmentInput::amount` have no upper bound~~ — fixed: both bounded by `MONEY_RANGE` (`0..=1e9`) before the sum is taken, which makes the wrap unreachable                               | Done; `overflow-checks` deliberately left off                                                               | `commands.rs:602`, `634`, `350-351`, `1576`; `Cargo.toml:70-75`  |
| **M2** | Tauri security    | **Medium**            | `backup_database` unconditionally `remove_file`s `dest.with_extension("db.part")` with no content check; TOCTOU between `exists()` and `rename`                                                                 | Stage the temp file in app-data and move it, or apply the same SQLite-magic check to `tmp`                  | `commands.rs:2104-2121`, `2129`, `2133-2135`                     |
| **M3** | Build config      | **Medium**            | `tests/**` is in no TypeScript project — `vue-tsc --noEmit` covers 31 files, none of them tests. `vitest.integration.config.ts` also uncovered                                                                  | Add a `tsconfig.test.json` (or extend `include`) and typecheck it in `npm run build`                        | `tsconfig.json:25-27`; `tsconfig.node.json:11`                   |
| **M4** | CI / Testing      | **Medium**            | Integration (99 cases) and E2E (50 scenarios) are wired to npm scripts but to no workflow                                                                                                                       | Add a CI job for `test:integration`; run `test:e2e` at least nightly                                        | `.github/workflows/build.yml`; `package.json:17-18`              |
| **M5** | Dependencies      | ~~Medium~~ **CLOSED** | ~~`rusqlite` 0.32.1 → `libsqlite3-sys` 0.30.1 bundles an older SQLite~~ — fixed in `7e592ac`: 0.39.0, SQLite **3.46.0 → 3.51.3**. Held below 0.40 because `libsqlite3-sys` 0.38 needs Rust 1.95 (`cfg_select!`) | Plan the bump; breaking API change, so schedule it rather than batching with a feature                      | `Cargo.toml:35`; `Cargo.lock`                                    |
| **M6** | Input validation  | **Medium**            | No length or format bound on any free-text field: `ClientInput.*`, `SettingsPatch.{language,currency_code,date_format,shop_name,shop_info}`                                                                     | Add length caps; allow-list `language`, `currency_code`, `date_format`                                      | `commands.rs:380-385`, `402-409`, `1908-1924`; `models.rs:46-52` |
| **L1** | Input validation  | Low                   | `list_all_payments` binds `limit` straight into `LIMIT ?1`; SQLite reads a negative limit as unlimited                                                                                                          | Clamp to e.g. `1..=5000`                                                                                    | `commands.rs:1414`, `1430`                                       |
| **L2** | Licensing         | Low                   | A release binary embedding the development public key only warns; the matching secret is published in `docs/license-format.md`                                                                                  | Make `verifying_key()` return `None` for the dev key when `!cfg!(debug_assertions)`                         | `license.rs:635-650`                                             |
| **L3** | Tooling           | Low                   | All 14 `security/*` ESLint rules are `warn` and `npm run lint` has no `--max-warnings 0` — findings cannot fail CI or the pre-commit hook                                                                       | Promote the retained high-signal rules to `error`, or add `--max-warnings 0`                                | `eslint.config.js:33`; `package.json:20`                         |
| **L4** | Licensing         | Low                   | Licence state is evaluated once at startup and cached for the process lifetime; expiry mid-session takes effect only on restart                                                                                 | Re-evaluate on a timer or on window focus, if the business rule requires it                                 | `lib.rs:81`; `license.rs:320-352`                                |
| **L5** | Rust robustness   | Low                   | `rebalance_amounts`/`apply_pool` validate `index` against `amounts.len()` then index `paid_amounts[index]`; a mismatched-length caller panics                                                                   | Add `debug_assert_eq!(amounts.len(), paid_amounts.len())`                                                   | `db.rs:427`, `430`, `465-481`                                    |
| **L6** | Input validation  | Low                   | `ImpayeFilter.date_from`/`date_to` never reach `parse_date`; a malformed date yields a silently empty list instead of `INVALID_DATE`                                                                            | Run both through `parse_date`                                                                               | `commands.rs:1505-1514`                                          |
| **L7** | Logging           | Low                   | `db.rs:272` logs a rejected, frontend-supplied date string verbatim — log noise, bounded by `{:?}` escaping. Local disk only, never IPC                                                                         | Truncate, or log only that parsing failed                                                                   | `db.rs:272`                                                      |
| **L8** | Architecture      | Low                   | `SettingsView.vue` imports `@tauri-apps/plugin-dialog` directly — the only Tauri import outside the gateway                                                                                                     | Move the dialogs behind `src/api/`, or accept and document it; today it blocks a mechanical lint rule       | `SettingsView.vue:103`, `138`, `170`                             |
| **L9** | Testing           | Low                   | 28/34 SFCs, 3/4 stores and 4/6 composables have no unit test — including `AppIcon.vue`, the one file with `v-html`. Locale key parity untested                                                                  | Add a locale-parity test (cheap, high value) and component tests for the two modals                         | `src/components/**`, `src/stores/**`, `src/composables/**`       |
| **I1** | Tauri security    | Info                  | No updater is configured — no `plugin-updater`, no `updater` key. The HTTPS-endpoint and signature checks do not apply                                                                                          | None. Revisit when an updater is added                                                                      | `tauri.conf.json`; `Cargo.toml`                                  |
| **I2** | Dependencies      | Info                  | 18 RustSec warnings (0 vulnerabilities): 10 GTK3 bindings, 5 `unic-*`, `proc-macro-error`, `glib` unsoundness, `event-listener` unsoundness                                                                     | Nothing actionable — all transitive via `tauri → tao/gtk`, no fixed versions exist. Policy already recorded | `Cargo.lock`; `deny.toml:5-16`                                   |
| **I3** | Performance       | Info                  | N+1 query pattern survives in `list_purchases` (3 queries per row) and the dashboard; commands are `async` but do blocking DB work                                                                              | Matters only at scale; `spawn_blocking` for `VACUUM INTO` and the listings if the DB grows                  | `commands.rs:564-574`, `1646+`                                   |

---

## 3. Reconnaissance detail

**Project layout.** `src-tauri/` (Rust core, 8 modules), `src/` (Vue renderer),
`tests/` (integration + E2E, deliberately outside `src/`), `docs/`. Config:
`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/deny.toml`,
`package.json`, `vite.config.ts`, `vitest.integration.config.ts`,
`eslint.config.js`, `tsconfig.json`,
`.github/workflows/{build,codeql,security}.yml`, `.github/dependabot.yml`.

**Tauri version: 2** (`tauri 2.11.5`), confirmed three ways — the `$schema` URL
at `tauri.conf.json:2`, the `capabilities` array under `app.security` (a v2-only
construct; v1 used a flat `allowlist`), and the locked crate version. This
matters for everything in §5: the permission model reviewed below is the v2
capability system, not the v1 allowlist.

**Plugins in use** (5, all registered in `lib.rs:55-65`, all exactly current
against crates.io):

| Plugin                         | Version | Why it is there                                                                                            |
| ------------------------------ | ------- | ---------------------------------------------------------------------------------------------------------- |
| `tauri-plugin-single-instance` | 2.4.3   | Registered **first**, deliberately — two processes on one SQLite file is the classic corruption window     |
| `tauri-plugin-log`             | 2.9.0   | Backend diagnostics to stdout + `LogDir`                                                                   |
| `tauri-plugin-dialog`          | 2.7.2   | Native open/save pickers (logo, backup destination, licence import)                                        |
| `tauri-plugin-os`              | 2.3.2   | OS locale detection on first run                                                                           |
| `tauri-plugin-opener`          | 2.5.4   | Hands `tel:`/`sms:` to the OS — navigating the WebView there destroys the SPA. Scoped to those two schemes |

**Notably absent, and deliberately so:** no `fs` plugin, no `shell` plugin, no
`http` plugin, no `updater`, no `sql` plugin.

**SQLite access.** Raw `rusqlite 0.39.0` with `features = ["bundled"]` — the
SQLite C library is statically compiled in, so there is no system-SQLite
dependency and no separately-patchable `libsqlite3` (which is also why M5
matters). A single `Connection` behind `std::sync::Mutex` (`db.rs:14-16`), locked
per command via `Db::lock()`. **The frontend has no SQL access of any kind** —
no SQL plugin and no `fs` permission, so the "frontend never touches the DB"
invariant is enforced by the capability set, not merely by convention.

**DB file at runtime:** `app.path().app_data_dir()` → `create_dir_all` →
`payment_schedule.db` (`lib.rs:67-71`). Platform app-data, never bundled in
resources, never user-selectable. Startup aborts if it cannot be opened — correct;
the app cannot function without it.

---

## 4. Dependency & version audit

### 4.1 Rust

`cargo-outdated` is not installed on this machine and was not installed for this
audit. Direct dependencies were compared against the crates.io API by hand
instead; transitive crates are covered by `cargo audit` and `cargo deny`, both of
which run in CI.

| Crate                          | Locked            | Latest stable | Assessment                                                                                                                                                                                |
| ------------------------------ | ----------------- | ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tauri`                        | 2.11.5            | 2.11.5        | ✅ current                                                                                                                                                                                |
| `tauri-build`                  | 2.6.3             | 2.6.3         | ✅ current                                                                                                                                                                                |
| `tauri-plugin-dialog`          | 2.7.2             | 2.7.2         | ✅ current                                                                                                                                                                                |
| `tauri-plugin-os`              | 2.3.2             | 2.3.2         | ✅ current                                                                                                                                                                                |
| `tauri-plugin-opener`          | 2.5.4             | 2.5.4         | ✅ current                                                                                                                                                                                |
| `tauri-plugin-single-instance` | 2.4.3             | 2.4.3         | ✅ current                                                                                                                                                                                |
| `tauri-plugin-log`             | 2.9.0             | 2.9.0         | ✅ current                                                                                                                                                                                |
| `chrono`                       | 0.4.45            | 0.4.45        | ✅ current                                                                                                                                                                                |
| `serde` / `serde_json`         | 1.0.229 / 1.0.151 | same          | ✅ current                                                                                                                                                                                |
| `log`                          | 0.4.33            | 0.4.33        | ✅ current                                                                                                                                                                                |
| `machine-uid`                  | 0.6.0             | 0.6.0         | ✅ current                                                                                                                                                                                |
| **`rusqlite`**                 | **0.39.0**        | 0.40.1        | ✅ **upgraded** (`7e592ac`). Held at 0.39: 0.40 needs `libsqlite3-sys` 0.38, whose build script uses `cfg_select!` (Rust **1.95**), and 0.40 only adds unused vtab APIs                   |
| `ed25519-dalek`                | **3.0.0**         | 3.0.0         | ✅ **upgraded** (`2edaa7e`). Forced `sha2 0.11`; brings `curve25519-dalek` 5, `ed25519` 3, `signature` 3                                                                                  |
| `base64`                       | **0.23.1**        | 0.23.1        | ✅ **upgraded** (`2edaa7e`). Three copies now in the lock (0.21.7, 0.22.1 via `plist`, ours) — build-time chain only                                                                      |
| `sha2`                         | **0.11.0**        | 0.11.0        | ✅ **upgraded** (`2edaa7e`) — not optional: `ed25519-dalek` 3 requires `^0.11`. The duplicate is against `tauri-codegen` (not `wry`, as the old comment claimed), on the build-time chain |

**Pre-1.0 crates relied on heavily:** `rusqlite` (0.32) and `chrono` (0.4) are
both pre-1.0 and load-bearing, as are `base64` 0.22 and `sha2` 0.10. All four are
mature and widely used; the risk is churn at each minor, not abandonment.

**Unmaintained / yanked:** no yanked crates — `deny.toml` sets `yanked = "deny"`
and CI enforces it. 18 unmaintained/unsound warnings, all transitive through
`tauri → tauri-runtime-wry → tao/gtk`; they clear when Tauri's Linux backend
leaves GTK3. `deny.toml:5-16` records this as a decision rather than an
oversight, which is the right way to carry an accepted risk.

**Edition / MSRV:** edition 2021, `rust-version = "1.88"`. The MSRV claim is
**proven** by a dedicated `cargo +1.88 check --locked --all-targets` job
(`build.yml:81-118`) — unusual and good; the previous audit found the declared
floor was fiction.

### 4.2 Node

**Package manager:** npm (only `package-lock.json` present; no yarn/pnpm lock).

**`npm audit`: 1 high, 0 critical, 0 moderate, 0 low.**

```
brace-expansion  4.0.0 - 5.0.8
Severity: high
brace-expansion: DoS via unbounded intermediate arrays,
bypassing the CVE-2026-14257 mitigation — GHSA-rgw5-rvv9-x895
```

Reached via `eslint@10.8.0 → minimatch@10.2.5 → brace-expansion@5.0.8`. It is
dev-only (ESLint is not shipped), but `npm audit` does not distinguish, and the
gate at `security.yml:52-53` is `npm audit --audit-level=high` — **verified exit
1**. `npm audit fix` resolves it without a major bump.

Note the improvement since the last audit: the previous 8 advisories came through
`vue-tsc 2.x` and `@vue/test-utils`, and were closed by the `vue-tsc@3` upgrade
plus the `js-beautify` override still present at `package.json:27-29`.

**`npm outdated`:**

| Package             | Current | Wanted | Latest | Note            |
| ------------------- | ------- | ------ | ------ | --------------- |
| `pinia`             | 2.3.1   | 2.3.1  | 4.0.2  | 2 majors behind |
| `vue-i18n`          | 10.0.8  | 10.0.8 | 11.4.8 | 1 major behind  |
| `vue-router`        | 4.6.4   | 4.6.4  | 5.2.0  | 1 major behind  |
| `vite`              | 7.3.6   | 7.3.6  | 8.2.0  | 1 major behind  |
| `typescript`        | 5.9.3   | 5.9.3  | 7.0.2  | 2 majors behind |
| `jsdom`             | 30.0.0  | 30.0.1 | 30.0.1 | patch           |
| `lint-staged`       | 17.2.0  | 17.3.0 | 17.3.0 | minor           |
| `playwright`        | 1.62.0  | 1.62.1 | 1.62.1 | patch           |
| `typescript-eslint` | 8.65.0  | 8.66.0 | 8.66.0 | minor           |
| `vue-tsc`           | 3.3.8   | 3.3.9  | 3.3.9  | patch           |

`vue` itself (3.5.x) is current. None of the outdated majors carries a known
advisory — this is maintenance debt, not a security finding. Dependabot runs
weekly for npm, cargo and actions, with dev-dependency minors/patches grouped to
cut PR noise.

### 4.3 Licences

Consistent and deliberate. `LICENSE` at the root is proprietary ("All rights
reserved"); `package.json` declares `"UNLICENSED"` + `"private": true`;
`Cargo.toml` uses `license-file = "../LICENSE"` with `publish = false`.
`deny.toml` exempts the private crate (`[licenses.private] ignore = true`) while
holding third-party crates to an explicit allow-list (MIT, Apache-2.0, BSD-2/3,
ISC, Zlib, MPL-2.0, CC0, Unicode, OpenSSL, and `blessing` for bundled SQLite).
`cargo deny check` runs in CI, so an incompatible transitive licence fails the
build. **No conflicts found.**

### 4.4 Runtime versions

`engines.node` is `">=22"`; installed **v24.16.0** ✅. CI pins Node 22
(`build.yml:33`, `security.yml:42`) — the declared floor, which is the right
choice for a floor test, though it does mean Node 24 is never exercised in CI.

---

## 5. Tauri-specific security review

### 5.1 Capabilities (v2)

`src-tauri/capabilities/default.json` — one capability, scoped to
`"windows": ["main"]`:

```
core:default, core:window:default, core:window:allow-set-title,
core:app:default, core:path:default, core:event:default,
dialog:default, dialog:allow-open, dialog:allow-save,
os:default, os:allow-locale,
{ "identifier": "opener:allow-open-url",
  "allow": [{ "url": "tel:*" }, { "url": "sms:*" }] }
```

**Nothing here is broader than the app needs.** Specifically checked and absent:
no `fs:*` of any kind, no `shell:*`, no `http:*`, no wildcard scopes. The
`opener` grant is the tightest form available — a URL allow-list of exactly two
schemes — and neither `opener:allow-open-path` nor
`opener:allow-reveal-item-in-dir` is granted. `dialog:allow-open`/`allow-save`
return paths to the renderer, which then hands them to a Rust command that
re-validates them: the right shape.

A marked improvement on the last audit, which found `fs:default` +
`fs:allow-write-file` over `$APPDATA/**` — i.e. the WebView could read and write
the database file directly.

### 5.2 CSP

`tauri.conf.json:26`:

```
default-src 'self'; script-src 'self'; object-src 'none';
img-src 'self' asset: http://asset.localhost data:;
style-src 'self' 'unsafe-inline'; font-src 'self' data:
```

Defined and restrictive. `script-src 'self'` is stated explicitly rather than
inherited — **no `unsafe-eval`, no `unsafe-inline` for scripts**, which is what
matters. `object-src 'none'` blocks plugin embedding. `style-src 'unsafe-inline'`
is required by Vue's runtime style bindings and is the standard accepted
trade-off; combined with zero `v-html` of user data (§8.2) it is not exploitable
here. `img-src` includes `asset:` because the shop logo is served over the asset
protocol.

**Asset protocol:** `enable: true`, `scope: ["$APPDATA/logo.*"]` — a single glob
for a single file family, not the whole app-data directory.
`remove_existing_logos` (`commands.rs:2042-2051`) exists specifically so a
png→jpg switch does not leave an orphan readable inside that scope.

### 5.3 devTools in production

Not force-enabled. There is no `"devtools": true` in `tauri.conf.json` and no
`open_devtools()` call anywhere in `src-tauri/src/`. Tauri's default already
gates devtools to debug builds and to the `devtools` Cargo feature, which is not
enabled. ✅

### 5.4 `#[tauri::command]` as an untrusted boundary

27 commands, every one treated as an untrusted-input boundary in this review.
Summary of the posture:

- **Every command taking a path** (`set_logo`, `backup_database`,
  `import_license`) validates before use, and every write destination is
  `app_data_dir().join(<fixed or allow-listed name>)`. **`PathBuf::join` with a
  caller-controlled component never occurs** — there is no path-traversal
  surface.
- **`set_logo`** (`commands.rs:1977-2039`): extension allow-list, `is_file()`,
  5 MiB cap, **magic-byte sniff**. Reduced to "any real image ≤ 5 MiB", and the
  in-code comment shows the residual was understood rather than missed.
- **`import_license`** (`commands.rs:2195`): 64 KiB cap checked against
  `metadata` _before_ the read, `is_file()`, and a full Ed25519 signature check
  _before_ the file is copied anywhere.
- **`backup_database`**: `.db` extension required, and an existing destination
  must itself start with `SQLite format 3\0`. Good — except for the temp path,
  which is **M2**.
- The gaps are numeric and string bounds, not path handling: **H3**, **M1**,
  **M6**, **L1**, **L6**.

Read commands (`get_client_detail`, `get_purchase_detail`, `get_settings`,
`get_license_status`) are intentionally reachable unlicensed; `list_clients` and
`list_purchases` _degrade_ to the active scope rather than refusing. 21 of 27
commands sit behind `require_license` (`commands.rs:62-67`), enforced in Rust —
`commands.rs:42-44` explains why a UI-only gate would be decoration.

### 5.5 Command injection

**Not applicable — no surface exists.** No `std::process::Command`, no `shell`
plugin, no `Command::new` anywhere in `src-tauri/src/`. The only OS handoff is
`tauri-plugin-opener`, scoped to `tel:`/`sms:`, and the URI it receives is built
by `contactUri()` (`src/composables/useContactActions.ts:41`), which allow-lists
characters, rejects other schemes, and bounds the digit count — with a unit test
asserting `file:///etc/passwd` is rejected.

### 5.6 Updater

**Not configured — this check does not apply.** No `tauri-plugin-updater` in
`Cargo.toml`, no `updater` key in `tauri.conf.json`, no update endpoint, no
`pubkey`. Distribution is via GitHub Releases built by `build.yml`.

Worth noting for when an updater _is_ added: `build.yml:157-181` already shows
the right instinct by refusing to build a release that carries the published
development licence key.

---

## 6. Rust backend review

### 6.1 Panics on input-handling paths

**One `expect()` in the entire non-test backend**, at `lib.rs:127`
(`.expect("error while running paymentSchedule")`) — startup, before IPC exists,
which is the correct place to abort. **Zero** `unwrap()`, `panic!`,
`unreachable!`, `todo!`, `unimplemented!` elsewhere in non-test code.

Mutex locks use `unwrap_or_else(|e| e.into_inner())` (`db.rs:53`,
`license.rs:338-349`) rather than `.unwrap()`, so one panicking command cannot
brick every later one — old finding #14, properly closed.

**Slice indexing** — every site was checked for a bound proof. All live sites are
safe: `rows[pos]` follows `.position()`, `rows[kept..]` follows
`kept = rows.len().min(amounts.len())`, `&bytes[8..12]` is guarded by a
`len() >= 12` check on the same line. The only unproven indexing is in
`rebalance_amounts`/`apply_pool` (**L5**), which has no callers today.

**Integer overflow** is the real residue — see **M1**. Note the contrast:
`add_interval` (`db.rs:339-358`) is _fully_ overflow-hardened with
`checked_mul` / `try_days` / `checked_add_signed` / `checked_add_months`, all
saturating, precisely because a naive version was a remote kill switch under
`panic = "abort"`. The same discipline has not reached the money sums.

`overflow-checks` is not set in `[profile.release]`, so release **wraps** where
debug **panics** — the two profiles disagree about what a bug even is.

### 6.2 Error handling and leakage to the frontend

**Clean by construction.** `AppError` (`error.rs:74-84`) has exactly one
serialization path (`error.rs:138-142`), which goes through `code()`
(`error.rs:120-127`), and the `Internal` variant collapses to the literal
`"INTERNAL"`. Every `From` impl — `rusqlite::Error`, `std::io::Error`,
`tauri::Error` — funnels through `AppError::internal`, which logs the detail and
drops it from the wire.

**No SQL text, constraint name, column name, schema detail or filesystem path can
cross IPC.** The only interpolated values are `Conflict` details, all numeric or
from a closed vocabulary (`PREVIOUS_UNPAID:{idx}`, `SUM_MISMATCH:{sum}:{total}`,
`OVERPAYMENT:{remaining}`, …).

`LicenseStatus` deliberately does not derive `Serialize`, and
`Malformed { reason }` is dropped in `to_info` (`license.rs:276-286`), so a parse
error cannot describe the file back to the renderer.

Residual (**L7**): `AppError::internal` writes detail to the log, and `db.rs:272`
logs a rejected frontend-supplied date string verbatim. Local disk, not IPC, and
bounded by `{:?}` escaping.

### 6.3 `unsafe`

**None.** `grep -rn "unsafe" src-tauri/src/ src-tauri/build.rs` returns nothing.
The only `unsafe` string in the repo is `'unsafe-inline'` in the CSP's
`style-src`.

### 6.4 Async runtime

All 27 commands are `async fn`, deliberately — `commands.rs:3-6` records that a
synchronous command would block the IPC/main event-loop thread. Old finding #7 is
half-closed:

- ✅ **No `MutexGuard` spans an await.** `grep "\.await"` across `src-tauri/src/`
  returns **zero matches** — no command awaits anything, so the guard never
  crosses a suspension point and every future stays `Send`. The invariant claimed
  in the module doc actually holds.
- ⚠️ **All DB work is blocking `rusqlite` on the async executor.** No
  `spawn_blocking`, no `tokio::` anywhere. For a single-user desktop app behind
  one `Mutex<Connection>` this is defensible, but `backup_database`'s
  `VACUUM INTO` and the N+1 listings can hold a Tokio worker for a long time
  (**I3**).

### 6.5 Module structure and separation of concerns

Clean, and the layering is enforced rather than merely described:

- `lib.rs` — wiring only (plugins, managed state, command registry)
- `commands.rs` — the IPC surface. Each command validates, locks, and delegates
  to a `*_impl` free function taking `&Connection` / `&mut Connection`
- `db.rs` — connection, migrations, bounds constants, pure date/money helpers
- `models.rs` — serde DTOs, no logic
- `error.rs` — the single error choke point
- `license.rs` — self-contained
- `seed.rs` — dev-only, gated on `debug_assertions` or an env var

The `*_impl` split is the best structural decision in the backend: it is why the
test count went from 3 to 126 without restructuring anything.

### 6.6 TODO/FIXME/HACK

**Zero** across `src-tauri/`. Combined with the density of _why_-comments, this
codebase resolves issues rather than parking them.

---

## 7. SQLite / data-layer review

### 7.1 Parameterized statements

**No SQL injection exists.** Every value-carrying position is a bound `?n`.

Five sites build SQL with `format!`. All interpolate compile-time-fixed text, and
are flagged here for completeness as requested:

| Site                    | What is interpolated                                                      | Verdict                                   |
| ----------------------- | ------------------------------------------------------------------------- | ----------------------------------------- |
| `db.rs:138`             | `PRAGMA user_version = {version}` — `version` is a `const` slice index    | Safe. PRAGMA cannot take a placeholder    |
| `db.rs:230`             | `ALTER TABLE {table} ADD COLUMN {column} {ddl}` — `&'static str` literals | Safe. Identifiers cannot be bound         |
| `commands.rs:301-315`   | A scope predicate: one of three `&'static str` from a closed serde enum   | Safe. `today_str` is bound as `?1`        |
| `commands.rs:536-538`   | Same, for `PurchaseScope`                                                 | Safe                                      |
| `commands.rs:1507-1517` | Only the _placeholder number_ `?{next}`; values go into `params_vec`      | Safe — the correct dynamic-filter pattern |

Both enums (`ClientScope`, `PurchaseScope`, `models.rs:37-43`, `107-113`) are
closed — no `#[serde(other)]`, no untagged variant — so no renderer value can
select an unintended predicate.

Two details done right: the `list_purchases` `search` string is **never** put
into SQL at all (it filters in Rust at `commands.rs:566-572`, so there is no
`LIKE` surface), and `VACUUM INTO ?1` binds the destination filename rather than
interpolating it.

### 7.2 Migrations

**Properly versioned.** `MIGRATIONS: &[fn(&Connection) -> DbResult<()>]`
(`db.rs:96-100`), append-only, index == version, tracked via
`PRAGMA user_version`. `migrate()` (`db.rs:113-150`):

- clamps a negative version
- **refuses to run against a schema newer than the binary knows** (`:117-128`) —
  a downgraded binary cannot corrupt a newer database
- runs each step in its own `BEGIN`/`COMMIT` **together with the version bump**,
  with `ROLLBACK` on failure, so no half-applied schema can be recorded complete
- `add_column_if_missing` (`db.rs:223-233`) makes `ALTER` steps replay-safe

Three migrations exist (`m0001_initial_schema`, `m0002_client_archive`,
`m0003_purchase_archive`). Migration behaviour is itself test-covered.

### 7.3 DB file location

`app_data_dir()/payment_schedule.db` (`lib.rs:67-71`) — OS app-data via Tauri's
path API, exactly as it should be. Not in app resources, not writable from the
renderer (no `fs` permission; the asset-protocol scope is `logo.*` only).

### 7.4 Connection handling and locking

A single `Connection` behind `std::sync::Mutex`, locked per command. Coherent
with the rest of the design:

- **WAL mode** ✅ (`db.rs:29-34`)
- **`busy_timeout = 5000`** ✅
- **`synchronous = NORMAL`** ✅
- **`foreign_keys = ON`** ✅ — the one most commonly missed, and this schema
  relies on `ON DELETE CASCADE` in three places
- All four are asserted by a test (`db.rs:852-869`), so they cannot silently
  regress
- `tauri-plugin-single-instance`, registered first, prevents two processes on one
  file

`SQLITE_BUSY` is therefore addressed on all three fronts — WAL for reader/writer
concurrency, `busy_timeout` for contention, single-instance for the process case.
No connection pool, which is correct for a single-user desktop app.

### 7.5 Sensitive data at rest

**Everything is plaintext, unencrypted, in `$APPDATA/payment_schedule.db`:**
client names, phone numbers, addresses, emails, and the complete financial
ledger — purchase totals, installments, payments, free-text notes.

No SQLCipher, no OS-keychain use, no file-permission hardening beyond whatever
`create_dir_all` yields. `backup_database` writes the same unencrypted content
wherever the user points it.

**Is encryption at rest warranted?** A judgement call, and the current choice is
defensible for a single-operator shop app on a machine the owner controls — but
it is the largest data-at-rest exposure in the project: a stolen laptop is the
whole customer book, including debt positions. Worth an explicit recorded
decision rather than a default. The `.gitignore` comment (`.gitignore:37-40`)
shows the PII sensitivity is already understood; the same reasoning applied to the
file itself leads to SQLCipher, or at minimum restrictive file permissions.

**No tokens, API keys or passwords are stored anywhere** — there are none in the
system. `license.json` holds only vendor-attested public data and a signature.

### 7.6 Licence validation (fail-closed check)

Reviewed because it is the only cryptography in the tree and the app's access
control. It is fail-closed at every branch:

- The public key is embedded with `env!` and **no fallback** — a release build
  without `PAYMENT_SCHEDULE_LICENSE_PUBKEY` does not compile
  (`license.rs:133-138`). `build.rs` injects the dev key for debug only.
- `verifying_key()` returns `None` on a bad constant, and `validate_bytes` then
  returns `InvalidSignature` — _"without a trust anchor no licence can be proven
  good"_ — rather than panicking (deliberate, since `panic = "abort"` would turn
  a misconfigured key into a startup crash).
- **Check order is signature-before-parse** (`license.rs:474-526`): envelope
  shape → version → **signature** → payload decode → dates → machine binding →
  expiry. Nothing attacker-supplied is parsed before the signature verifies.
- Uses `verify_strict`, rejecting small-order keys and non-canonical scalars.
- Domain-separated by a signing prefix, over the base64 text as it appears in the
  file — eliminating JSON-canonicalization ambiguity.
- `is_valid()` is `matches!(Valid(_))`; every other variant, including
  `ClockTampered`, is unlicensed.
- Clock-rollback guard via a high-water mark kept deliberately **out** of
  `Settings`/`SettingsPatch`, so the renderer — the thing it defends against —
  cannot write it. Verified: no such field exists in `models.rs`.
- Machine binding is hashed and salted, never the raw OS UUID.

Two residuals: **L2** (dev key only warns in a release binary) and **L4**
(evaluated once at startup).

---

## 8. Vue / frontend review

### 8.1 Version and API consistency

Vue **3.5**, Composition API, `<script setup lang="ts">` in **34 of 34** SFCs.
Zero Options API, zero plain-JS components, zero multi-block files. There is no
mix to flag — this is as consistent as it gets.

### 8.2 XSS

**One `v-html` in the whole frontend**, at `src/components/ui/AppIcon.vue:82`.
Traced fully: the bound value is `ICONS[props.name] ?? ""`, where `ICONS` is a
module-level literal of 40 inline SVG strings. `props.name` is used **only as a
key lookup** and is never interpolated into the output; an attacker-controlled
`name` can at worst miss and render `""`. Every call site passes a string
literal. The ESLint suppression is scoped to that one line and re-enabled
immediately after — correct practice, not a blanket disable.

**Zero** occurrences of `innerHTML`, `outerHTML`, `document.write`, `eval`,
`new Function`, `insertAdjacentHTML`, or `createContextualFragment` anywhere in
`src/`, `tests/`, or `index.html`.

Adjacent surfaces checked and clean: no `:href` bindings, no `window.open`, no
`location.href` assignment; the only anchor construction is the CSV download,
whose `href` is a `URL.createObjectURL` blob that is revoked after use; CSV
output is hardened against formula injection; locale files contain no HTML, and
vue-i18n runs `legacy: false` with no `v-html` of translations, so the missing
`escapeParameter` option is unreachable.

### 8.3 State management

Consistent. Four Pinia stores, all setup-syntax, holding only genuinely global
state (licence verdict, settings, sidebar counters, toasts). Per-page data is
local `ref` inside each view, fetched through the gateway. Six shared composables
carry cross-cutting logic. No event bus, no provide/inject state, no
prop-drilling workarounds.

`src/stores/license.ts:3-6` explicitly states the store is presentation-only and
that enforcement lives in Rust — "a `v-if` is not a control". Verified true.

### 8.4 `invoke()` routing

**All 29 `invoke()` call sites are inside `src/api/index.ts`.** No component,
view, store or composable calls `invoke` directly; the import itself is wrapped
in a private helper. Every method is an `isTauri() ? invoke(...) : mockDb.*(...)`
ternary, which is what structurally guarantees the browser/test path matches the
desktop surface — the property the whole integration suite depends on.

Four `@tauri-apps` imports exist outside the gateway; three are the
`plugin-dialog` calls in `SettingsView.vue` (**L8**) and one is `convertFileSrc`
in `src/lib/assets.ts`, a pure path→URL transform with no IPC round-trip. None is
an `invoke`.

### 8.5 Secrets and debug artefacts

**None.** A case-insensitive grep for
`api[_-]?key|secret|password|token|credential|private[_-]?key|bearer|authorization`
across `src/` returns 4 hits, all false positives ("design **tokens**", and a
security test asserting `file:///etc/passwd` is rejected). No `debugger`. No
feature or debug flags, no `NODE_ENV` branching in `src/`.

`console.*`: 6 occurrences in `src/`, **all `console.error` inside a `catch`**,
each with an explanatory comment. Zero `console.log`/`warn`/`info`/`debug`. The
only `console.log` in the repo is the E2E test reporter, explicitly allow-listed
in the ESLint config.

### 8.6 TypeScript rigour

`strict: true`, plus `noUnusedLocals`, `noUnusedParameters`,
`noFallthroughCasesInSwitch`. **Zero** `@ts-ignore`, `@ts-expect-error`,
`@ts-nocheck`, or `any` in `src/` and `tests/` — a raw `\bany\b` grep returns
only the English word in prose comments. Exactly one `eslint-disable` in the
whole frontend (the `AppIcon` one above).

The gap is coverage, not strictness: `tests/**` is in no TS project (**M3**).

---

## 9. Vite / build config review

`vite.config.ts` (47 lines), one plugin (`@vitejs/plugin-vue` 6.0.8, current).

| Setting               | Value                                   | Assessment                                                       |
| --------------------- | --------------------------------------- | ---------------------------------------------------------------- |
| `build.target`        | `chrome105` on Windows, else `safari13` | Correct — matches the WebView per platform                       |
| `build.minify`        | `"esbuild"` unless `TAURI_ENV_DEBUG`    | ✅                                                               |
| **`build.sourcemap`** | **`!!process.env.TAURI_ENV_DEBUG`**     | ✅ **Off in production.** Tauri sets that var only for dev/debug |
| `server.host`         | `process.env.TAURI_DEV_HOST \|\| false` | ✅ Loopback-only unless the mobile-dev var is explicitly set     |
| `server.port`         | 5173, `strictPort: true`                | Required by Tauri's `devUrl`                                     |
| `envPrefix`           | `["VITE_", "TAURI_ENV_"]`               | See below                                                        |
| Proxy / middleware    | **none**                                | ✅ Nothing dev-only can leak into a production build             |

**Environment variables — nothing is exposed.** There is **no `import.meta.env`
usage anywhere in `src/`**, and **no `VITE_`-prefixed variable is defined or read
anywhere in the project**. The `envPrefix` entry is Vite scaffolding, not an
active channel. `TAURI_ENV_*` is also bundled, but those are platform/arch/debug
flags injected by the Tauri CLI — no secret has any path into the client bundle.

**No `.env` files exist** anywhere in the repo (verified by `find`, excluding
`node_modules`), so there is nothing to leak. `.gitignore` covers `.env`,
`.env.local`, `.env.*.local` regardless.

`vitest.integration.config.ts` is a separate runner for `tests/integration/**`,
kept out of the default `vitest run` deliberately so the fast unit pass stays
fast — the rationale is documented in the file header.

---

## 10. Project hygiene

**Linting/formatting.** ESLint 10 flat config with `js.recommended`,
`eslint-plugin-vue` **flat/recommended** (note: `vue/no-v-html` lives in this
tier, so downgrading to `flat/essential` would silently open a hole),
`vueTsConfigs.recommended`, `eslint-plugin-security`, and
`eslint-plugin-no-unsanitized`. Both security plugins were checked in
`node_modules`; neither declares a `files` key, so both genuinely apply to every
linted file including `.vue`. `security/detect-object-injection` is disabled
globally with a three-line justification — the correct call, since it is the
plugin's highest-false-positive rule and TypeScript already covers it. Prettier
interop comes last, correctly. Husky + lint-staged enforce on commit.

`cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are both
clean and both gated in CI. The one gap is **L3** — security rules are `warn`,
and nothing fails on them.

**`.gitignore`** is thorough: `node_modules/`, `dist/`, `dist-ssr/`,
`src-tauri/target/`, `src-tauri/gen/`, `.env*`, editor/OS junk, `*.log`, and
`*.db` / `*.db-wal` / `*.db-shm` / `*.db-journal` / `*.db.part` / `*.sqlite` /
`*.sqlite3` — the last group with an explicit PII rationale. `Cargo.lock` is
intentionally committed (correct for a binary) with a comment explaining why.

**Tests.**

| Suite       | Files | Cases | Runs in CI?        |
| ----------- | ----- | ----- | ------------------ |
| Rust unit   | —     | 126   | ✅ since `9f9ad6c` |
| Vitest unit | 10    | 147   | ✅ `npm test`      |
| Integration | 8     | 99    | ❌ no (**M4**)     |
| E2E         | 1     | 50    | ❌ no (**M4**)     |

Coverage is deep on business logic — `finance.ts` (25 tests plus a 5-test
cross-language parity suite checking a shared fixture against the Rust
implementation), CSV (19, including formula-injection cases), the installment
rule matrix (30), archive guards (31), the error-code contract (11) — and thin on
presentation (**L9**).

**TODO/FIXME/HACK/XXX: zero** across `src/`, `tests/`, and `src-tauri/`.

**CI/CD.** Three workflows. All third-party actions SHA-pinned with a
human-readable version comment, and Dependabot configured to bump the pins so
pinning does not mean rot. `permissions:` is scoped per workflow. `build.yml`
gates a release on lint + format + typecheck + JS unit tests + rustfmt + clippy,
proves the MSRV in a dedicated job, and refuses to build a release carrying the
published development licence key. CodeQL runs `security-and-quality` on the
TS/JS side weekly. This is well above average — the gaps (H2, M4) are of
omission, not misconfiguration.

---

## 11. Appendix — raw dependency audit output

### `cargo audit` (from `src-tauri/`)

```
    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 1189 security advisories (from ~/.cargo/advisory-db)
    Updating crates.io index
    Scanning Cargo.lock for vulnerabilities (513 crate dependencies)

warning: 18 allowed warnings found
```

**0 vulnerabilities.** The 18 warnings:

| Crate                | Version | Kind         | Advisory          |
| -------------------- | ------- | ------------ | ----------------- |
| `atk`                | 0.18.2  | unmaintained | RUSTSEC-2024-0413 |
| `atk-sys`            | 0.18.2  | unmaintained | RUSTSEC-2024-0416 |
| `gdk`                | 0.18.2  | unmaintained | RUSTSEC-2024-0412 |
| `gdk-sys`            | 0.18.2  | unmaintained | RUSTSEC-2024-0418 |
| `gdkwayland-sys`     | 0.18.2  | unmaintained | RUSTSEC-2024-0411 |
| `gdkx11`             | 0.18.2  | unmaintained | RUSTSEC-2024-0417 |
| `gdkx11-sys`         | 0.18.2  | unmaintained | RUSTSEC-2024-0414 |
| `gtk`                | 0.18.2  | unmaintained | RUSTSEC-2024-0415 |
| `gtk-sys`            | 0.18.2  | unmaintained | RUSTSEC-2024-0420 |
| `gtk3-macros`        | 0.18.2  | unmaintained | RUSTSEC-2024-0419 |
| `proc-macro-error`   | 1.0.4   | unmaintained | RUSTSEC-2024-0370 |
| `unic-char-property` | 0.9.0   | unmaintained | RUSTSEC-2025-0081 |
| `unic-char-range`    | 0.9.0   | unmaintained | RUSTSEC-2025-0075 |
| `unic-common`        | 0.9.0   | unmaintained | RUSTSEC-2025-0080 |
| `unic-ucd-ident`     | 0.9.0   | unmaintained | RUSTSEC-2025-0100 |
| `unic-ucd-version`   | 0.9.0   | unmaintained | RUSTSEC-2025-0098 |
| `event-listener`     | 5.4.1   | **unsound**  | RUSTSEC-2026-0221 |
| `glib`               | 0.18.5  | **unsound**  | RUSTSEC-2024-0429 |

All transitive through `tauri → tauri-runtime-wry → tao/gtk`. No fixed versions
exist; they clear when Tauri's Linux backend leaves GTK3.

### `cargo outdated`

**Not run — the tool is not installed, and installing it was declined for this
pass.** §4.1 contains a manual comparison of all 16 direct dependencies against
the crates.io API instead. Transitive coverage comes from `cargo audit` (above)
and `cargo deny check`, both of which run in CI.

### `npm audit`

```
# npm audit report

brace-expansion  4.0.0 - 5.0.8
Severity: high
brace-expansion: DoS via unbounded intermediate arrays, bypassing the
CVE-2026-14257 mitigation - https://github.com/advisories/GHSA-rgw5-rvv9-x895
fix available via `npm audit fix`
node_modules/brace-expansion

1 high severity vulnerability
```

Dependency path:

```
payment-schedule@0.1.0
└─┬ eslint@10.8.0
  └─┬ minimatch@10.2.5
    └── brace-expansion@5.0.8
```

`npm audit --audit-level=high` → **exit code 1** (this is the CI gate).

### `npm outdated`

```
Package            Current  Wanted  Latest
jsdom               30.0.0  30.0.1  30.0.1
lint-staged         17.2.0  17.3.0  17.3.0
pinia                2.3.1   2.3.1   4.0.2
playwright          1.62.0  1.62.1  1.62.1
typescript           5.9.3   5.9.3   7.0.2
typescript-eslint   8.65.0  8.66.0  8.66.0
vite                 7.3.6   7.3.6   8.2.0
vue-i18n            10.0.8  10.0.8  11.4.8
vue-router           4.6.4   4.6.4   5.2.0
vue-tsc              3.3.8   3.3.9   3.3.9
```

### Environment

```
node    v24.16.0   (engines: >=22 ✅)
npm     11.13.0
rustc   MSRV declared 1.88, proven by a dedicated CI job
```

---

## 12. Suggested next steps

**Quick wins — minutes each, no design decisions**

1. **`npm audit fix`** (H1). Turns the security workflow green. In-range patch.
2. ~~**Add a `cargo test` job to `build.yml`** and put it in `release.needs` (H2).~~ **Done — `9f9ad6c`.**
   The tests already exist and pass; this is pure wiring, and it is the biggest
   gap between "we have tests" and "the tests protect us".
3. **Clamp `limit` in `list_all_payments`** to `1..=5000` (L1).
4. **Run `ImpayeFilter` dates through `parse_date`** (L6).
5. **Add `debug_assert_eq!` on the slice lengths** in `rebalance_amounts` (L5).
6. **Add a locale key-parity test** (fr/en/ar) — cheap, and it pins a project
   invariant currently asserted nowhere (part of L9).

**Small, contained fixes — an hour or two each**

7. ~~**Assert `list.len() == installment_count` in `resolve_schedule`** (H3).~~ **Done.**
8. **Stage the backup temp file in app-data, or magic-check it before removing**
   (M2).
9. ~~**Bound `total_price` and per-line `amount`** (M1).~~ **Done** — via `MONEY_RANGE`.
   `overflow-checks` deliberately left off: with `panic = "abort"` it trades a
   correctness bug for an availability one, and the bounds make the wrap
   unreachable anyway.
10. **Add length caps and allow-lists to the free-text inputs** (M6).
11. **Put `tests/**` in a typechecked TS project** (M3) and add
    `test:integration` to CI (M4).

**Larger — schedule deliberately**

12. ~~**Bump `rusqlite` 0.32 → 0.40** (M5).~~ **Done — `7e592ac`, to 0.39.** (M5). Breaking API change plus a new bundled
    SQLite; give it its own PR with the full test suite behind it. The only
    dependency gap with a security dimension, and it has now survived two audits.
13. **Decide explicitly about encryption at rest** (§7.5). Not necessarily
    SQLCipher — restrictive file permissions plus a recorded decision may be the
    right answer for this threat model. What matters is that it stops being a
    default and becomes a choice.
14. **Frontend majors**: `pinia` 2→4, `vue-router` 4→5, `vue-i18n` 10→11,
    `vite` 7→8. No advisories; sequence them one at a time behind the E2E suite.
15. **Address the N+1 query pattern** (I3) if the data set ever outgrows a single
    shop's book — and wrap `VACUUM INTO` in `spawn_blocking` regardless, since it
    is the one operation that can hold a worker for seconds.
16. **Close the presentation-layer test gap** (L9), starting with
    `NewPurchaseModal.vue` — it carries the most logic of any component and has
    no unit test.
