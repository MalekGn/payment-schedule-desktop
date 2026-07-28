# paymentSchedule — Development Audit Report

**Date:** 2026-07-26
**Commit audited:** `eb86a6a` (branch `dev`, working tree clean)
**Scope:** architecture, security, dependencies, data layer, frontend, build config, hygiene
**Nature:** read-only review. **No code was changed.** The only file written is this report.

**Stack as actually found**

| Layer    | What is there                                                                                                          |
| -------- | ---------------------------------------------------------------------------------------------------------------------- |
| Shell    | **Tauri 2** (`tauri 2.11.5`, `tauri-build 2.6.3`, config `$schema: schema.tauri.app/config/2`)                         |
| Backend  | Rust, edition 2021, 1 895 LOC across `src-tauri/src/{lib,commands,db,models,seed}.rs`                                  |
| Database | **`rusqlite 0.32.1`, `features = ["bundled"]`** (SQLite compiled into the binary). No SQL plugin, no ORM               |
| Frontend | **Vue 3.5** + TypeScript, `<script setup>` everywhere (16/16 SFCs), Pinia 2, vue-router 4, vue-i18n 10                 |
| Build    | Vite 7.3.6, `vue-tsc --noEmit` typecheck, ESLint 10 flat config, Prettier 3, husky + lint-staged                       |
| CI       | 3 GitHub Actions workflows (release bundles, CodeQL, security audit), Dependabot on npm/cargo/actions                  |
| Tests    | Vitest unit (5 files / 56 tests), Vitest integration (3 files), Playwright E2E (`tests/e2e/run.mjs`), 3 Rust `#[test]` |

**Local gate status at audit time** (all run, all results real):

| Gate                                        | Result                                                                                                                    |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `npx eslint .`                              | ✅ clean (exit 0)                                                                                                         |
| `npx vue-tsc --noEmit`                      | ✅ clean (exit 0)                                                                                                         |
| `npx vitest run`                            | ✅ 5 files, 56 tests passed                                                                                               |
| `cargo fmt --check`                         | ✅ clean                                                                                                                  |
| `cargo clippy --all-targets -- -D warnings` | ✅ clean                                                                                                                  |
| `cargo audit`                               | ✅ **0 vulnerabilities**, ⚠️ 17 unmaintained/unsound warnings                                                             |
| `npm audit --audit-level=high`              | ❌ **exit 1** — 8 high advisories (dev chain). _This is the CI gate in `.github/workflows/security.yml`; it fails today._ |

---

## 1. Executive summary

Ordered most-important first.

1. **The WebView is granted direct read _and write_ access to the app-data directory, which is where the SQLite database lives** — `fs:default` + `fs:allow-write-file` scoped to `$APPDATA/**` (`src-tauri/capabilities/default.json:17-26`) plus `assetProtocol.scope: ["$APPDATA/**", …]` (`src-tauri/tauri.conf.json:28-31`). `payment_schedule.db` is created at `app_data_dir()/payment_schedule.db` (`src-tauri/src/lib.rs:23-25`), i.e. inside that scope. This directly contradicts the project's own stated invariant ("the frontend never touches the DB or filesystem directly") at the capability level. **Nothing in `src/` imports `@tauri-apps/plugin-fs`** — the permission and the plugin are pure attack surface with zero callers.
2. **Unvalidated numbers from the frontend reach `chrono` date math that panics, and the release profile is `panic = "abort"`** — so a bad IPC argument terminates the process. Confirmed panic paths: `get_dashboard`'s `upcoming_days` (`commands.rs:706`) and `add_interval`'s `interval_days` for `interval_kind: "custom"` (`db.rs:146`). `installment_count` is only checked for `< 1` (`commands.rs:305`), so a large value drives an unbounded `Vec` allocation + insert loop.
3. **Raw backend error strings are shown verbatim to end users** — `ui.notify(String(e), "error")` at `ClientsView.vue:84`, `ClientForm.vue:52`, `NewPurchaseModal.vue:136`, and `error.value = String(e)` at `PaymentModal.vue:51`. Every Rust error is `rusqlite::Error::to_string()` (~90 `map_err(|e| e.to_string())` call sites), so SQL text, column names and constraint details surface in a toast, unlocalized. `CLAUDE.md` explicitly classes this as a Code Review blocker.
4. **`set_logo` is an arbitrary-file-read primitive** — it `std::fs::copy`s any `source_path` the frontend sends into app data with no path, type, or size validation (`commands.rs:921-937`). Combined with finding 1 (app data is readable by the WebView via `asset:` and `fs:read`), a compromised renderer can copy e.g. `~/.ssh/id_rsa` to `logo.rsa` and then read it back.
5. **All 20 commands are synchronous, so they execute inline on the IPC/main event-loop thread**, and several are N+1 query loops (`list_purchases` → `build_purchase_summary` per row → 3 queries each, `commands.rs:268-295`; `get_client_detail`, `commands.rs:187-193`; `get_dashboard` runs 7 aggregates + 5 summaries + a detail + `build_impayes`). The UI will freeze under load; there is no `spawn_blocking` and no async anywhere in the backend.
6. **The data layer has no durability hardening**: no WAL, no `busy_timeout`, no single-instance guard (`db.rs:21-31`). Two launched copies of the app both open the same file and will hit `SQLITE_BUSY`, surfaced as a raw SQL toast (finding 3).
7. **The schema has no version tracking** — `migrate()` is `CREATE TABLE IF NOT EXISTS` only (`db.rs:70-127`), no `PRAGMA user_version`, no migration table. Any future column change has no upgrade path for databases already in the field, and this app's data (a shop's receivables) is not reproducible.
8. **Dev-dependency chain is failing its own CI security gate**: 8 high npm advisories, all from `brace-expansion` (GHSA-mh99-v99m-4gvg) reached via `vue-tsc 2.2.12` → `@vue/language-core` → `minimatch`, and via `@vue/test-utils` → `js-beautify`. Fix requires `vue-tsc@3.x` (major). `rusqlite` is 8 minor versions behind (0.32.1 vs 0.40.1), which also means an older SQLite C library is compiled into every shipped binary.
9. **Rust behaviour is effectively untested** — 3 `#[test]` functions total, and both the integration and E2E suites drive `src/api/mock.ts`, not the Rust commands. Transactions, cascade deletes, overpayment, and the `finance.ts` ↔ `db.rs` parity invariant have no backend coverage.
10. **Positives worth stating**: no SQL string interpolation of user data anywhere; money is integer end-to-end; CSP is defined and restrictive for scripts; no `shell` plugin and no `Command::new`; devtools are not force-enabled for release; capabilities are per-window and mostly narrow (`opener` is scoped to `tel:*`/`sms:*` only); actions are SHA-pinned; all three locale files are exactly in sync (264 keys each); lint/format/typecheck/clippy are all clean.

---

## 2. Findings table

| #   | Area             | Severity | Finding                                                                                                                                                                                                                | Recommendation                                                                                                                                                                   | File / Location                                                                                                                    |
| --- | ---------------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Tauri security   | High     | WebView granted `fs` read **and write** over `$APPDATA/**`, which contains the SQLite DB; plugin has no callers in `src/`                                                                                              | Remove `tauri_plugin_fs`, the npm `@tauri-apps/plugin-fs` dep, and all `fs:*` permissions. Narrow `assetProtocol.scope` to `$APPDATA/logo.*`                                     | `capabilities/default.json:17-26`; `tauri.conf.json:28-31`; `lib.rs:17`                                                            |
| 2   | Rust robustness  | High     | Frontend-supplied `upcoming_days` flows into `chrono::Duration::days` → panics on overflow; `panic = "abort"` kills the app                                                                                            | Clamp to a sane range (e.g. 1..=365) before the date math; or use `checked_add_signed`                                                                                           | `commands.rs:706`; `Cargo.toml:38`                                                                                                 |
| 3   | Rust robustness  | High     | `interval_days` unbounded for `interval_kind: "custom"`: `interval_days * k` overflows (wraps in release), then `Duration::days`/`NaiveDate + Duration` panic                                                          | Validate `interval_days` (e.g. 1..=365) and `interval_kind` against the three known values; use checked arithmetic                                                               | `db.rs:143-152` (line 146); `commands.rs:358-364`                                                                                  |
| 4   | Data integrity   | High     | `installment_count` only checked `< 1`; `split_amounts` then allocates a `Vec` of that size and the loop inserts that many rows                                                                                        | Add an upper bound (e.g. ≤ 120) alongside the existing `< 1` check                                                                                                               | `commands.rs:305-307`; `db.rs:189-198`                                                                                             |
| 5   | Error handling   | High     | Raw `rusqlite` error text rendered to users in toasts / modal errors (project-declared blocker)                                                                                                                        | Return typed error codes from Rust (the `CLIENT_HAS_PURCHASES:`/`SUM_MISMATCH:` style, applied consistently), map to i18n keys on the frontend, log detail to console only       | `ClientsView.vue:84`; `ClientForm.vue:52`; `NewPurchaseModal.vue:136`; `PaymentModal.vue:51`; ~90 `map_err` sites in `commands.rs` |
| 6   | Tauri security   | Medium   | `set_logo` copies any `source_path` into app data; no extension allow-list, no size cap, no image sniff, no path containment                                                                                           | Validate the extension against the dialog's filter list, cap the file size, and reject paths outside the user's home/pictures dirs                                               | `commands.rs:920-937`                                                                                                              |
| 7   | Performance / UX | Medium   | All commands are sync → run on the IPC/main thread; N+1 query patterns in list/dashboard paths                                                                                                                         | Make read commands `async` (Tauri then runs them off the main thread) and/or `spawn_blocking`; replace per-row loops with set-based SQL + `GROUP BY`                             | `commands.rs:268-295`, `187-193`, `701-841`                                                                                        |
| 8   | SQLite           | Medium   | No WAL, no `busy_timeout`, no single-instance guard — two app instances share one DB file                                                                                                                              | `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL` at open; add `tauri-plugin-single-instance`                                                       | `db.rs:21-31`; `lib.rs:22-30`                                                                                                      |
| 9   | SQLite           | Medium   | No schema versioning or migration mechanism; irreversible cascade deletes with no backup/export of the DB                                                                                                              | Add `PRAGMA user_version` + ordered migration steps; add a "backup database" action (file copy) before destructive operations                                                    | `db.rs:70-127`; `commands.rs:246-261`                                                                                              |
| 10  | Data integrity   | Medium   | Manual `installments[].due_date` is written to the DB **unparsed**; a malformed date then silently degrades status to `"pending"` forever                                                                              | Run every incoming date through `parse_date` before insert                                                                                                                       | `commands.rs:349-372`; silent fallback at `commands.rs:65-67`, `672-674`                                                           |
| 11  | Data integrity   | Medium   | `update_settings` performs up to 7 separate writes with no transaction — a failure mid-way leaves settings half-applied                                                                                                | Wrap the whole patch in one `conn.transaction()`                                                                                                                                 | `commands.rs:890-916`                                                                                                              |
| 12  | Data integrity   | Medium   | `record_payment` accepts unlimited overpayment: `new_paid = paid + amount` with no cap, and marks the installment paid                                                                                                 | Reject `amount > (amount_due - paid)`, or explicitly allow with a distinct code path and UI confirmation                                                                         | `commands.rs:394-437` (line 411)                                                                                                   |
| 13  | Data integrity   | Medium   | `total_price` is never validated — a negative or zero total produces negative installments and negative KPIs                                                                                                           | Require `total_price > 0` in `create_purchase`                                                                                                                                   | `commands.rs:304-347`                                                                                                              |
| 14  | Concurrency      | Medium   | `db.conn.lock().unwrap()` at 20 call sites: in a debug build one panic poisons the mutex and every later command panics                                                                                                | Use `lock().unwrap_or_else(                                                                                                                                                      | e                                                                                                                                  | e.into_inner())` or a non-poisoning mutex (`parking_lot`) | `commands.rs:139,171,208,226,247,269,299,310,380,400,460,482,504,532,654,703,885,891,934,941`; `db.rs:41` |
| 15  | Dependencies     | Medium   | 8 high npm advisories (`brace-expansion` GHSA-mh99-v99m-4gvg) via `vue-tsc 2.2.12` and `@vue/test-utils`; the `npm audit --audit-level=high` CI job fails today (verified exit 1)                                      | Upgrade `vue-tsc` to 3.x (breaking; also unblocks `@vue/language-core`) and refresh `@vue/test-utils`; re-run the gate                                                           | `package.json:44,55`; `.github/workflows/security.yml:47-56`                                                                       |
| 16  | Dependencies     | Medium   | `rusqlite` 0.32.1 vs latest 0.40.1 → `libsqlite3-sys 0.30.1` bundles an ~2-year-old SQLite into every shipped binary                                                                                                   | Plan a `rusqlite` 0.40 upgrade so the bundled SQLite tracks upstream fixes                                                                                                       | `Cargo.toml:27`; `Cargo.lock`                                                                                                      |
| 17  | Observability    | Medium   | Zero logging in the backend — no `log`/`tracing`, no output on any failure path; frontend has one `console.error`                                                                                                      | Add `tauri-plugin-log` or `tracing`; log command failures at `warn`/`error` with no PII (names/phones are PII here)                                                              | whole of `src-tauri/src/`; only logger call: `useContactActions.ts:73`                                                             |
| 18  | Error handling   | Medium   | Several load paths have no error handling: `ClientsView.load()` leaves `loading = true` forever on failure; `stats.refresh()` swallows silently; `main.ts` uses `.finally()` so a settings-load rejection is unhandled | Add try/catch + an error state to view loaders; log the swallowed cases                                                                                                          | `ClientsView.vue:47-51`; `DashboardView.vue:21`; `stores/stats.ts:13-19`; `main.ts:19`                                             |
| 19  | Testing          | Medium   | 3 Rust tests; no backend coverage of transactions, cascade deletes, overpayment, or `finance.ts` ↔ `db.rs` parity. Integration + E2E both exercise the TS mock only                                                    | Add `#[test]`s over a temp DB for `create_purchase`/`record_payment`/`delete_*`; add a parity test asserting `split_amounts`/`add_interval` agree across the two implementations | `commands.rs:950-1028`; `db.rs:200-257`; `tests/integration/*`                                                                     |
| 20  | Hygiene          | Low      | Declared MSRV `rust-version = "1.77"` is wrong — locked deps require ≥ **1.88** (`darling`, `time`, `plist`). No `rust-toolchain.toml`, no `engines` in `package.json` (CI uses Node 22, this machine runs Node 24)    | Set `rust-version = "1.88"`, add `rust-toolchain.toml`, add `"engines": { "node": ">=22" }`                                                                                      | `Cargo.toml:7`; `package.json`                                                                                                     |
| 21  | Frontend         | Low      | CSV export: fields are wrapped in `"` but embedded `"` is not doubled (breaks the file), and no formula-injection guard on user-entered names/phones                                                                   | Escape `"` → `""` on every field; prefix values starting with `= + - @` with `'`                                                                                                 | `ImpayesView.vue:85-118`                                                                                                           |
| 22  | Rust             | Low      | `clear_logo` discards the `remove_file` error (`let _ = …`) — a failure to delete the old logo is invisible                                                                                                            | Log the failure; keep the setting update                                                                                                                                         | `commands.rs:940-948` (line 944)                                                                                                   |
| 23  | Rust             | Low      | `delete_client`'s `force = false` guard (and its `CLIENT_HAS_PURCHASES` code) is unreachable — the UI always passes `true`                                                                                             | Either pass `false` first and let the backend gate the confirm, or delete the dead parameter                                                                                     | `commands.rs:246-261` vs `ClientsView.vue:78`                                                                                      |
| 24  | Rust             | Low      | `SELECT *` + `row.get("column")` mapping in `fetch_client`/`map_purchase`/`map_payment` — silently breaks on schema drift                                                                                              | List columns explicitly in the projection                                                                                                                                        | `commands.rs:32`, `85-90`, `463-467`, `485-489`, `507-511`                                                                         |
| 25  | Dependencies     | Low      | 17 RustSec warnings: 10 unmaintained GTK3 crates + `glib 0.18.5` unsoundness (RUSTSEC-2024-0429) + `proc-macro-error` + 6 `unic-*`. All transitive through Tauri; no fixed versions exist                              | Nothing actionable locally. Track Tauri's GTK4 work; keep `deny.toml`'s empty `ignore` list so they stay visible                                                                 | `Cargo.lock`; appendix §9.1                                                                                                        |
| 26  | Frontend deps    | Low      | Majors behind: `pinia 2.3.1` → 4.0.2, `vue-router 4.6.4` → 5.2.0, `vue-i18n 10.0.8` → 11.4.8, `vite 7.3.6` → 8.1.5, `jsdom 25` → 29, `typescript 5.9.3` → 7.0.2                                                        | Schedule the Pinia/vue-router/vue-i18n majors as one deliberate upgrade PR with the E2E suite as the gate                                                                        | `package.json`; appendix §9.4                                                                                                      |
| 27  | Licensing        | Low      | Neither `package.json` nor `Cargo.toml` declares a license; the repo has no LICENSE file                                                                                                                               | Add a `license` field (and file) — `deny.toml` already enforces an allow-list for dependency licenses                                                                            | `package.json:2-6`; `Cargo.toml:1-8`                                                                                               |
| 28  | Hygiene          | Low      | `.gitignore` covers `node_modules/`, `dist/`, `src-tauri/target/`, `src-tauri/gen/`, `.env*` — but not `*.db`/`*.sqlite`                                                                                               | Add `*.db`, `*.sqlite*` as defence in depth (tests write temp DBs to the OS temp dir today, so nothing is leaking now)                                                           | `.gitignore`                                                                                                                       |
| 29  | Tauri security   | Info     | CSP allows `style-src 'unsafe-inline'` (required by Vue's runtime style bindings). `script-src` is not stated, so it inherits `default-src 'self'` — no `unsafe-eval`/`unsafe-inline` for scripts                      | Acceptable. Consider stating `script-src 'self'` explicitly so a future `default-src` relaxation can't widen it silently                                                         | `tauri.conf.json:26`                                                                                                               |
| 30  | Frontend         | Info     | One `v-html` (`AppIcon.vue:77`) renders a value from a module-local static `ICONS` map, with an ESLint-disable and a justification comment. `props.name` only selects a key                                            | No change needed — this is the correct pattern                                                                                                                                   | `AppIcon.vue:64-79`                                                                                                                |

---

## 3. Reconnaissance detail

### 3.1 Layout

```
payment-schedule-desktop/
├─ src/                       Vue 3 renderer (16 SFCs, 8 TS modules, 3 locale files)
│  ├─ api/{index,mock}.ts     the only IPC boundary + its in-browser twin
│  ├─ lib/{finance,alerts,assets}.ts
│  ├─ stores/{settings,stats,ui}.ts        (Pinia, setup-style)
│  ├─ composables/{useBack,useClickOutside,useContactActions,useFormat,useSort}.ts
│  └─ locales/{ar,fr,en}.json              264 keys each, exactly in sync
├─ src-tauri/
│  ├─ src/{lib,commands,db,models,seed}.rs 1 895 LOC
│  ├─ capabilities/default.json            one capability, window "main"
│  ├─ tauri.conf.json, Cargo.toml, Cargo.lock (committed, deliberately)
│  ├─ deny.toml, rustfmt.toml
│  └─ gen/schemas/                         generated ACL manifests (gitignored)
├─ tests/{integration,e2e}/
├─ .github/workflows/{build,codeql,security}.yml + dependabot.yml
├─ vite.config.ts, vitest.integration.config.ts, eslint.config.js, tsconfig*.json
└─ architecture.md, features.md, README.md, CLAUDE.md, docs/e2e/qa-report.md
```

### 3.2 Tauri version — v2, confirmed three ways

`tauri = { version = "2", features = ["protocol-asset"] }` (`Cargo.toml:21`), resolving to **2.11.5** in `Cargo.lock`; config declares `"$schema": "https://schema.tauri.app/config/2"`; the permission model in use is v2's **capabilities** (`src-tauri/capabilities/default.json`), not a v1 allowlist. `@tauri-apps/api ^2.1.1` on the JS side. Nothing v1-shaped remains.

### 3.3 Plugins in use

| Plugin                      | Rust        | JS                      | Used for                                                        | Verdict                                     |
| --------------------------- | ----------- | ----------------------- | --------------------------------------------------------------- | ------------------------------------------- |
| `tauri-plugin-os` 2.3.2     | `lib.rs:15` | `stores/settings.ts:28` | `locale()` for first-run language detection                     | Justified, minimal (`os:allow-locale`)      |
| `tauri-plugin-dialog` 2.7.2 | `lib.rs:16` | `SettingsView.vue:68`   | Native file picker for the shop logo                            | Justified                                   |
| `tauri-plugin-fs` 2.5.1     | `lib.rs:17` | **no importers**        | — nothing                                                       | **Remove (finding 1)**                      |
| `tauri-plugin-opener` 2.5.4 | `lib.rs:21` | `api/index.ts:33-35`    | Hands `tel:`/`sms:` to the OS instead of navigating the WebView | Justified; scoped to those two schemes only |

No community plugins. No `tauri-plugin-sql`, no `tauri-plugin-updater`, no `tauri-plugin-shell`, no `tauri-plugin-single-instance`, no `tauri-plugin-log`.

### 3.4 How SQLite is accessed, and where the file lives

Raw **`rusqlite`** with the `bundled` feature — SQLite is statically compiled in, so there is no dependency on a system libsqlite3. One `Connection` behind a `std::sync::Mutex`, managed as Tauri state:

```rust
// src-tauri/src/db.rs:11-14
pub struct Db { pub conn: Mutex<Connection> }
```

```rust
// src-tauri/src/lib.rs:22-30
let data_dir = app.path().app_data_dir()?;   // correct: OS app-data dir, not resources
std::fs::create_dir_all(&data_dir)?;
let db_path = data_dir.join("payment_schedule.db");
let database = db::Db::open(&db_path)…;
app.manage(database);
```

The location is right — `app_data_dir()` (e.g. `~/.local/share/tn.paymentschedule/` on Linux, `%APPDATA%\tn.paymentschedule\` on Windows), never a bundled read-only resource. The logo is copied next to it as `logo.<ext>`. The problem is not _where_ the file is, it is _who else can reach it_ (finding 1). Demo seeding is gated to debug builds unless `PAYMENT_SCHEDULE_SEED=1|true` (`db.rs:52-67`), and that gate is unit-tested (`db.rs:204-218`) — good.

---

## 4. Dependency & version audit

### 4.1 Rust

`cargo audit` (RustSec DB, 1 169 advisories, 490 crates scanned): **0 vulnerabilities**, 17 warnings — full output in §9.1.

- 10 × unmaintained **gtk-rs GTK3** bindings (`atk`, `atk-sys`, `gdk`, `gdk-sys`, `gdkwayland-sys`, `gdkx11`, `gdkx11-sys`, `gtk`, `gtk-sys`, `gtk3-macros`), RUSTSEC-2024-0411…0418.
- 1 × **unsound**: `glib 0.18.5`, RUSTSEC-2024-0429 (`VariantStrIter` iterator impls).
- 6 × unmaintained: `proc-macro-error 1.0.4` (RUSTSEC-2024-0370) and `unic-*` 0.9.0 (RUSTSEC-2025-0075/0080/0081/0098/0100).

Every one is transitive through `tauri` → `tauri-runtime-wry` → `tao`/`gtk`, or through build-time proc macros. There is no version to bump to; they clear when Tauri's Linux backend leaves GTK3. Since `Cargo.lock` is committed and `deny.toml`'s `ignore = []` is empty, the weekly CI run keeps them visible — that is the right posture.

Direct dependencies vs crates.io latest (`cargo-outdated` was not installed; versions fetched from the crates.io API — see §9.3 for the method):

| Crate                  | Locked            | Latest stable | Gap                                                                                                           |
| ---------------------- | ----------------- | ------------- | ------------------------------------------------------------------------------------------------------------- |
| `tauri`                | 2.11.5            | 2.11.5        | current                                                                                                       |
| `tauri-build`          | 2.6.3             | 2.6.3         | current                                                                                                       |
| `tauri-plugin-dialog`  | 2.7.2             | 2.7.2         | current                                                                                                       |
| `tauri-plugin-fs`      | 2.5.1             | 2.5.1         | current (but should be removed — finding 1)                                                                   |
| `tauri-plugin-os`      | 2.3.2             | 2.3.2         | current                                                                                                       |
| `tauri-plugin-opener`  | 2.5.4             | 2.5.4         | current                                                                                                       |
| `serde` / `serde_json` | 1.0.229 / 1.0.151 | same          | current                                                                                                       |
| `chrono`               | 0.4.45            | 0.4.45        | current — **pre-1.0 and load-bearing** for all date math (finding 3 is about how it is called, not the crate) |
| **`rusqlite`**         | 0.32.1            | **0.40.1**    | **8 minor versions behind** (finding 16)                                                                      |
| **`libsqlite3-sys`**   | 0.30.1            | **0.38.1**    | follows `rusqlite`; determines the bundled SQLite version                                                     |

Nothing is yanked. No git or non-crates.io sources (`deny.toml` flags them as warnings).

**Edition / MSRV.** `edition = "2021"`, `rust-version = "1.77"`. That declared MSRV is not achievable: locked dependencies declare MSRVs up to **1.88.0** (`darling`, `darling_core`, `darling_macro`, `plist`, `time`, `time-core`, `time-macros`, `serde_with`) and 1.87 (`zbus`, `wasip2`). Installed toolchain here is **1.97.0**, and CI uses `dtolnay/rust-toolchain@stable`, so nothing is broken today — but the manifest advertises a floor that would fail. There is no `rust-toolchain.toml` pinning a version for contributors.

### 4.2 Node

Package manager: **npm** (`package-lock.json`, lockfileVersion 3; no yarn/pnpm lock). Local Node **24.16.0** / npm **11.13.0**; CI pins Node **22**; `package.json` declares no `engines` and no `packageManager`.

`npm audit`: **8 high**, 0 critical/moderate/low. Single root cause:

```
brace-expansion <=5.0.7  (GHSA-mh99-v99m-4gvg, DoS via unbounded expansion → OOM)
└─ minimatch 2.0.0–10.0.2
   ├─ @vue/language-core → vue-tsc 1.7.0-alpha.0–3.0.0-beta.5
   ├─ editorconfig → js-beautify → @vue/test-utils
   └─ glob
```

All four paths are **devDependencies** (typecheck + component-test tooling), so no vulnerable code is shipped in the bundle. But `npm audit --audit-level=high` is exactly what `.github/workflows/security.yml:55-56` runs, and it exits 1 today — the weekly Security-audit job is red, and any PR touching `package.json`/`package-lock.json` will be too. `npm audit fix --force` resolves it by installing `vue-tsc@3.3.8` (breaking).

Runtime dependencies are clean of advisories. Majors behind: `pinia` 2.3.1 → 4.0.2, `vue-router` 4.6.4 → 5.2.0, `vue-i18n` 10.0.8 → 11.4.8. Dev: `vite` 7 → 8, `jsdom` 25 → 29, `typescript` 5.9 → 7.0.2, `vue-tsc` 2.2 → 3.3. Patches available: `@tauri-apps/plugin-dialog` 2.7.1 → 2.7.2, `eslint` 10.7 → 10.8, `playwright` 1.61.1 → 1.62.0.

Nothing unmaintained on the JS side: every direct dependency is a first-party Vue/Tauri/Vite/ESLint package with releases inside the last few months.

---

## 5. Tauri-specific security review

### 5.1 Capabilities (`src-tauri/capabilities/default.json`)

One capability, `main-capability`, bound to `windows: ["main"]` — correct v2 shape.

| Permission                                                                                                                          | Needed?                                                                                                                                                                                                  |
| ----------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `core:default`, `core:window:default`, `core:window:allow-set-title`, `core:app:default`, `core:path:default`, `core:event:default` | Reasonable for a single-window SPA                                                                                                                                                                       |
| `dialog:default`, `dialog:allow-open`                                                                                               | Yes — logo picker (`SettingsView.vue:68`). No save/message dialogs are used, so `dialog:default` is slightly wide but harmless                                                                           |
| `os:default`, `os:allow-locale`                                                                                                     | Yes — locale detection                                                                                                                                                                                   |
| **`fs:default`**                                                                                                                    | **No.** Expands to `create-app-specific-dirs` + `read-app-specific-dirs-recursive` + `deny-default` (per the generated manifest) — i.e. recursive read of AppConfig/AppData/AppLocalData/AppCache/AppLog |
| **`fs:allow-read-file` @ `$APPDATA/**`, `$APPLOCALDATA/**`**                                                                        | **No** — nothing in `src/` reads files                                                                                                                                                                   |
| **`fs:allow-write-file` @ `$APPDATA/**`, `$APPLOCALDATA/**`**                                                                       | **No, and worst of the three** — this grants the WebView write access to `payment_schedule.db` itself                                                                                                    |
| `opener:allow-open-url` @ `tel:*`, `sms:*`                                                                                          | Yes, and exemplary: URL-scoped allow-list, with `useContactActions.ts` validating the number before it is ever handed over                                                                               |

No wildcard `fs:allow-*` without a scope, no `shell` permissions at all, no `http` permissions. The single real problem is the unused `fs` grant, and it happens to cover the database.

`assetProtocol` (`tauri.conf.json:28-31`) is enabled with scope `$APPDATA/**`, `$APPLOCALDATA/**` so that `convertFileSrc` can render the logo (`src/lib/assets.ts`). The same over-broad scope applies: the renderer can `fetch()` the raw bytes of `payment_schedule.db` through `asset:`. Narrow it to `$APPDATA/logo.*`.

### 5.2 Content-Security-Policy

```
default-src 'self'; img-src 'self' asset: http://asset.localhost data:;
style-src 'self' 'unsafe-inline'; font-src 'self' data:
```

Defined (not `null`), which is the important part. `script-src` is absent and therefore inherits `default-src 'self'` — **no `unsafe-eval`, no `unsafe-inline` for scripts**. `style-src 'unsafe-inline'` is genuinely required by Vue's runtime style bindings and scoped-style injection; the exposure is limited given scripts cannot be injected. `img-src` includes `asset:`/`http://asset.localhost` (needed for the logo) and `data:`. `connect-src` is not stated → inherits `'self'`, so no outbound calls. No `frame-src`, and the app has no iframes. Recommend stating `script-src 'self'` and `object-src 'none'` explicitly so future edits to `default-src` cannot silently widen script execution.

### 5.3 devTools in production

Not force-enabled: `Cargo.toml:21` enables only `features = ["protocol-asset"]` — the `devtools` feature is absent, so Tauri wires devtools in debug builds only. There is no `withGlobalTauri` in the config either, so `window.__TAURI__` is not injected globally. Nothing to fix.

### 5.4 `#[tauri::command]` boundary — treating each as untrusted input

20 commands (`lib.rs:31-57`). Validation actually present:

| Command             | Validates                                                                                            | Missing                                                                                                                                                                                               |
| ------------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create_purchase`   | `installment_count >= 1`; `purchase_date` via `parse_date`; manual amounts must sum to `total_price` | upper bound on `installment_count` (#4); `total_price > 0` (#13); `interval_kind` ∈ {weekly, monthly, custom} and `interval_days` range (#3); `installments[].due_date` never parsed (#10)            |
| `record_payment`    | `amount > 0`; `payment_date` via `parse_date`; installment must exist                                | overpayment cap (#12); no check that `payment_date` is ≥ purchase date / ≤ today                                                                                                                      |
| `update_settings`   | `alert_soon_days` clamped to 1..=90 — good, and mirrored in the UI                                   | `language`/`currency_code`/`date_format` accepted as free-form strings and echoed back to the renderer (low risk: they only select formatting, and unknown locales fall back via `isSupportedLocale`) |
| `get_dashboard`     | —                                                                                                    | `upcoming_days` unbounded → panic (#2)                                                                                                                                                                |
| `set_logo`          | —                                                                                                    | everything (#6)                                                                                                                                                                                       |
| `list_all_payments` | —                                                                                                    | `limit` unbounded; negative becomes SQLite's "no limit". Harmless but sloppy                                                                                                                          |
| id-taking commands  | Type-safe `i64`; SQL is parameterized; FKs enforced (`PRAGMA foreign_keys = ON`, `db.rs:23`)         | fine                                                                                                                                                                                                  |

Text fields (`first_name`, `product_label`, `note`, …) are `.trim()`ed and bound as parameters — no length limits, but with parameterized SQL and no `v-html` sink, the impact is cosmetic.

### 5.5 Shell / command injection

**Not applicable, and confirmed absent.** No `tauri-plugin-shell`, no `shell:*` permission, no `Command::new`, no `std::process::Command` anywhere in `src-tauri/src/` (the only `std::process` uses are `std::process::id()` for unique temp-DB filenames in tests, `db.rs:236` and `commands.rs:964`). External URIs go through the opener plugin with a scheme allow-list and a validated payload. There is no string-built command line anywhere in this codebase.

### 5.6 Updater

**Not configured — stated explicitly since the audit asked.** No `tauri-plugin-updater` in `Cargo.toml` or `package.json`, no `plugins.updater` block, no `pubkey`, no `createUpdaterArtifacts` in `tauri.conf.json`. Distribution is manual: `.github/workflows/build.yml` bundles deb/rpm/AppImage/msi/nsis and attaches them to a **draft** GitHub Release. Consequences: nothing to misconfigure (no unpinned endpoint, no disabled signature check), but also **no way to ship a security fix to installed copies** — users must download a new installer. Bundles are also unsigned (no `signingIdentity`/`certificateThumbprint`), so Windows SmartScreen will warn. If auto-update is ever added, the endpoint must be HTTPS and `pubkey` signature verification must stay on (it is mandatory in Tauri v2's updater, which is a good default).

---

## 6. Rust backend review

### 6.1 `unwrap` / `expect` / `panic!`

No `panic!`, no `unreachable!`, no `todo!`. Two categories of `unwrap`:

1. **`db.conn.lock().unwrap()`** — 21 sites (20 in `commands.rs`, 1 at `db.rs:41`). This only fails on mutex poisoning, i.e. after another thread panicked while holding the lock. In release that is moot (`panic = "abort"`, `Cargo.toml:38` — the process is already gone). In a debug build, however, one panicking command poisons the mutex and **every subsequent command panics**, so `tauri dev` degrades into a dead app that still renders. Recommend `unwrap_or_else(|e| e.into_inner())`.
2. **`lib.rs:59` `.expect("error while running paymentSchedule")`** — startup-only, on `Builder::run`. Acceptable; nothing useful to recover to. Note the `setup` closure (`lib.rs:22-30`) correctly propagates with `?` and wraps the DB-open failure in a message.

The real crash risk is not `unwrap` — it is the **implicit panics inside `chrono`** reached from unvalidated IPC arguments (findings 2 and 3), verified against `chrono-0.4.45` source:

- `TimeDelta::days` → `expect(TimeDelta::try_days(days), "TimeDelta::days out of bounds")` (`time_delta.rs:137-139`)
- `impl Add<TimeDelta> for NaiveDate` → `.checked_add_signed(rhs).expect("`NaiveDate + TimeDelta` overflowed")` (`naive/date/mod.rs:1981-1989`)

`db.rs:143-152` uses the panicking `+` for `weekly`/`custom` but the _checked_ API for monthly (`checked_add_months(...).unwrap_or(date)`), so the safe pattern already exists in the file — it is simply not applied to the other two branches. Also note `Months::new(k as u32)` at `db.rs:149`: an `i64 → u32` cast that would wrap for a negative `k`. `k` is a loop index today, so it is latent, not live.

### 6.2 Error handling and information leakage

Every fallible call is `.map_err(|e| e.to_string())` and every command returns `Result<T, String>` (`DbResult<T>`, `db.rs:16`). Consequences:

- The **entire `rusqlite` error text** crosses the IPC boundary — constraint names, column names, SQL fragments, SQLite result codes. The frontend then shows it verbatim (finding 5). Example: a `UNIQUE`/FK violation on `create_purchase` reaches the shopkeeper as `FOREIGN KEY constraint failed`.
- **Filesystem paths leak** from `set_logo`/`Db::open`: `lib.rs:27` formats `Failed to open database: {e}`, and `commands.rs:923` returns the raw `app_data_dir()` error.
- The codebase already has the right pattern in two places — `CLIENT_HAS_PURCHASES:{count}` (`commands.rs:256`), `SUM_MISMATCH:{sum}:{total}` (`commands.rs:342`), `INVALID_AMOUNT`, `INVALID_INSTALLMENT_COUNT`, `INSTALLMENT_NOT_FOUND`. Extending that discipline to the generic `map_err` sites (a small `enum AppError` + `impl Serialize`) fixes finding 5 at the source.

There is a second, quieter failure mode: `parse_date(...).unwrap_or("pending")` at `commands.rs:65-67` and `672-674`, and `.unwrap_or(0)` for `days_late` at `commands.rs:591-593`, `815-817`. A corrupt `due_date` never raises anything — the row simply reports "pending" and 0 days late forever. Combined with finding 10 (dates written unvalidated), a bad write is invisible in every screen.

### 6.3 `unsafe`

**None.** Zero `unsafe` blocks in `src-tauri/src/`. Nothing to justify.

### 6.4 Async runtime / blocking

There is **no async in the backend at all** — no `tokio` usage of its own, no `async fn`, no `spawn_blocking`. All 20 commands are synchronous, so Tauri executes them inline in the IPC handler (`tauri::ipc::protocol::message_handler`, installed as wry's `ipc_handler` in `manager/webview.rs:530`), which runs on the main event-loop thread; only `async` commands get moved onto the async runtime. Every SQLite query therefore blocks the UI thread.

That is tolerable at demo scale (6 clients / 8 purchases) but the query patterns amplify it badly:

- `list_purchases` (`commands.rs:268-295`): 1 query for ids, then per purchase `build_purchase_summary` → `build_purchase_detail` → 3 queries (purchase, client, installments). **3N + 1 queries**, and the `search` filter is applied _in Rust after building every summary_ (`commands.rs:285-291`) — so a search over 500 purchases still builds 500 full details.
- `get_client_detail` (`commands.rs:187-193`): 3N + 2.
- `get_dashboard` (`commands.rs:701-841`): 7 scalar aggregates + 5 recent summaries (15 queries) + 1 featured detail (3) + `build_impayes` — ~30 queries plus a `HashMap` group-by, on every dashboard load _and_ on every `stats.refresh()` after any mutation.

Fix direction: make the read commands `async` (which alone moves them off the main thread), and collapse the loops into single `GROUP BY` queries — the schema already has the indices for it (`db.rs:120-123`).

### 6.5 Module structure

Clean and appropriately small for the size: `lib.rs` (wiring) → `commands.rs` (IPC surface) → `db.rs` (connection, schema, pure helpers) → `models.rs` (serde DTOs) → `seed.rs` (demo data). Dependency direction is one-way; `models.rs` has no logic; the pure math (`split_amounts`, `installment_status`, `purchase_status`, `add_interval`, `seeding_decision`) lives in `db.rs` and is genuinely unit-testable.

The one structural remark: at 1 028 lines, `commands.rs` mixes three concerns — the IPC layer, row mapping, and business logic (`build_purchase_detail`, `build_purchase_summary`, `build_impayes`). Splitting into `commands/` (thin, validating) + a `queries`/`domain` module would make the validation gaps in §5.4 obvious by inspection and give the business logic a place to be tested without a `State<Db>`. Not urgent at this size.

---

## 7. SQLite / data-layer review

### 7.1 SQL injection — clean, with one construct that deserves a note

Every value reaching SQLite is a bound parameter (`?1`, `?2`, …) via `params![…]` or a slice. There is **no `format!()`-built value anywhere in a query**. The one place SQL text is assembled dynamically is `build_impayes` (`commands.rs:546-580`):

```rust
let mut sql = String::from("SELECT … WHERE i.due_date < ?1 AND i.amount > i.paid_amount");
if let Some(from) = filter.date_from.clone() {
    next += 1;
    sql.push_str(&format!(" AND i.due_date >= ?{next}"));   // placeholder index only
    params_vec.push(Box::new(from));                        // value stays bound
}
```

The interpolated content is a monotonic placeholder _number_, never user data — this is safe, and the parameter/placeholder lockstep is documented in-code and covered by a regression test (`commands.rs:969-1009`, written after a real bug where a fixed set of four parameters broke the no-filter path). Worth keeping an eye on only because the pattern invites a future edit that interpolates a column or value; a comment already warns about the binding count specifically.

### 7.2 Migrations

`migrate()` (`db.rs:70-127`) runs one `execute_batch` of `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS` on every startup. Idempotent, and fine for greenfield installs — but there is **no version tracking of any kind**: no `PRAGMA user_version`, no `schema_migrations` table, no `ALTER TABLE` path. Adding a column later will silently do nothing on an existing database, and the app will then fail at query time against the old shape. Because this data is a shop's receivables ledger (not reproducible, not backed up anywhere), that is the highest-consequence structural gap after finding 1. Minimum viable fix: read `PRAGMA user_version`, apply an ordered `&[fn(&Connection) -> DbResult<()>]` from that index, write the new version back — all inside one transaction.

### 7.3 File location

Correct: `app.path().app_data_dir()` + `create_dir_all` + `join("payment_schedule.db")` (`lib.rs:23-25`). Not inside bundled resources, not next to the executable, no hardcoded path. The logo lands in the same directory as `logo.<ext>`. The problem is scope exposure (finding 1), not placement.

### 7.4 Connection handling & concurrency

One `Connection` in a `Mutex`, locked per command (`db.rs:11-14`). Within a single process that fully serializes access, so intra-process `SQLITE_BUSY` is impossible — at the cost of no read concurrency at all (finding 7). What is missing:

- **No WAL** — default rollback-journal mode. WAL would allow concurrent readers, reduce fsync cost, and is the standard choice for a desktop app.
- **No `busy_timeout`** — so any external contention fails _immediately_ with `SQLITE_BUSY` rather than retrying.
- **No single-instance guard** — nothing stops a user launching paymentSchedule twice (double-click on the AppImage, desktop entry + tray). Two processes, two `Mutex`es, one file: interleaved writes to the same tables, `SQLITE_BUSY` surfaced as a raw SQL toast (finding 5), and with rollback-journal mode a crash mid-write is the classic corruption window. `tauri-plugin-single-instance` is the direct fix.
- `PRAGMA foreign_keys = ON` **is** set (`db.rs:23`), which matters because the delete paths rely on `ON DELETE CASCADE`. It is per-connection and there is only one connection, so this is correct.

Transactions: `create_purchase` (`commands.rs:311-374`) and `record_payment` (`commands.rs:401-436`) both use `conn.transaction()`, and the early `SUM_MISMATCH` return correctly drops the transaction (rusqlite rolls back on drop) — so no partial purchase can be written. `update_settings` is the one multi-write command with **no** transaction (finding 11).

### 7.5 Sensitive data at rest

The DB holds **client PII** — first/last name, phone, postal address, email (`db.rs:73-81`) — plus each person's debt position and payment history. No credentials, API keys, or tokens are stored anywhere (verified by grep across `src/`, `src-tauri/src/`, and the config files); there is no auth in the app at all. Data is plaintext SQLite with **no encryption at rest** and **no application-level lock**.

Is encryption warranted? For a single shopkeeper's own machine, full-disk encryption is the proportionate control and app-level encryption (SQLCipher / `rusqlite` `bundled-sqlcipher`) mainly adds key-management burden. Two things do change the calculus and are worth deciding deliberately: (a) if the machine is shared with staff, any local user or process can read the ledger — no OS permission stops that today; (b) finding 1 means the _renderer_ can read and write it too. Fix finding 1 first; then treat encryption as a business decision, and note that under Tunisian law (loi 2004-63) this database is personal data with a retention/consent posture worth documenting. What _is_ unambiguously missing is a **backup/export path** — cascade deletes are irreversible and there is no snapshot mechanism (finding 9).

---

## 8. Frontend, build config, and hygiene

### 8.1 Vue

**Vue 3.5.13**, and the pattern discipline is genuinely consistent: all 16 SFCs use `<script setup lang="ts">`, there is **no `export default {}`** anywhere, no Options API, no mixins, no `defineComponent`. Composition API throughout, with logic factored into `composables/` (`useBack`, `useClickOutside`, `useContactActions`, `useFormat`, `useSort`) and pure modules (`lib/finance.ts`, `lib/alerts.ts`) that are unit-tested independently of components. `tsconfig.json` runs `strict` + `noUnusedLocals` + `noUnusedParameters` + `noFallthroughCasesInSwitch`, and `vue-tsc --noEmit` is clean.

**`v-html`**: exactly one occurrence (`AppIcon.vue:77`), rendering `body` = a lookup into a module-local static `ICONS` record; `props.name` can only select a key, and a miss yields `""`. It carries a scoped `eslint-disable vue/no-v-html` with a written justification. No XSS surface. No `innerHTML`, no `outerHTML`, no `eval`, no `new Function` anywhere in `src/`.

**State management**: Pinia only, three setup-style stores with clear ownership — `settings` (persisted via IPC, OS-locale detection), `stats` (sidebar badges), `ui` (toasts, sidebar, header-title override). No Vuex, no ad-hoc global reactive singletons, no cross-store reaching. Consistent.

**IPC access**: a real, single, typed gateway — `src/api/index.ts` exports one `api` object whose every method is `isTauri() ? invoke(...) : mockDb....`. **No component or store calls `invoke()` directly.** The three direct `@tauri-apps/*` imports outside the gateway are all non-command plugin APIs and each is defensible: `convertFileSrc` (`lib/assets.ts:1`), the dialog picker (`SettingsView.vue:68`), and `os.locale()` (`stores/settings.ts:28`). `mock.ts` (691 lines) mirrors the command surface method-for-method, including `openExternal`, so the api/mock parity invariant currently holds.

**Secrets / debug flags**: none. No `VITE_`-prefixed variable is read anywhere in `src/` (the only mentions of `VITE_` in the repo are `vite.config.ts:31`'s `envPrefix` declaration itself). No `.env` file exists. No API keys, tokens, or passwords. One `console.error` (`useContactActions.ts:73`), deliberate and documented, with the user-facing toast kept clean of plugin internals — the correct split.

**i18n / RTL**: `locales/{fr,en,ar}.json` all have **exactly 264 keys with zero divergence** (verified by flattening and diffing all three), and `applyLocale` sets both `lang` and `dir="rtl"` on `<html>` (`i18n/index.ts:33-38`). The locale invariant from `CLAUDE.md` is being respected. The gap is that user-facing _error_ text bypasses i18n entirely via `String(e)` (finding 5).

**Money math**: `lib/finance.ts` mirrors `db.rs` — `splitAmounts` uses `Math.trunc` integer division with the remainder on the last installment, matching Rust's `/`; `installmentStatus`/`purchaseStatus` match the Rust predicates branch for branch; monthly `addInterval` clamps to end-of-month, matching `checked_add_months`. **No floats in money math** on either side. One divergence worth knowing: Rust's monthly branch is overflow-guarded (`unwrap_or(date)`) while the TS side has no guard, and Rust's `weekly`/`custom` branches panic where TS silently produces an out-of-range date.

### 8.2 Vite / build

`vite.config.ts` is small and Tauri-idiomatic. Reviewed against the audit's checklist:

- **Plugin set**: only `@vitejs/plugin-vue` 6.0.8 (current). No dev-only plugin left in the production path; no `mode`-conditional plugins at all.
- **Build target**: `chrome105` on Windows / `safari13` elsewhere (`vite.config.ts:38`) — correct WebView2/WKWebView targeting.
- **Source maps**: `sourcemap: !!process.env.TAURI_ENV_DEBUG` (`vite.config.ts:40`) — off for release builds, and `minify` is `esbuild` unless debug. **Verified against the artifact**: the local `dist/` (456 KB) contains **zero `.map` files**. Correct.
- **Env handling**: `envPrefix: ["VITE_", "TAURI_ENV_"]`. `.gitignore` covers `.env`, `.env.local`, `.env.*.local`; no `.env` file exists in the repo or working tree; nothing tracked by git matches `.env`, `dist/`, `node_modules/`, or `*.db`. No secret is at risk of being bundled — the exposure surface exists in configuration only, with no values behind it.
- **Dev server**: `port: 5173`, `strictPort`, `host: TAURI_DEV_HOST || false` (so no LAN binding unless explicitly opted in), HMR only when that variable is set, and `src-tauri/**` excluded from the watcher. **No `server.proxy`, no dev middleware, nothing that could leak into a production build** — and `server.*` is ignored by `vite build` regardless.

### 8.3 Linting, formatting, hooks, CI

- **ESLint 10** flat config (`eslint.config.js`), thoughtfully assembled: `js.configs.recommended` + `eslint-plugin-vue` flat/recommended + `vueTsConfigs.recommended` + **`eslint-plugin-security`** + **`eslint-plugin-no-unsanitized`**, with `skipFormatting` last to stay out of Prettier's lane. `security/detect-object-injection` is disabled globally with a written rationale (false-positive rate on `obj[variable]`, already type-checked) while the high-signal rules (`detect-eval-with-expression`, `detect-child-process`, `detect-non-literal-fs-filename`, `detect-unsafe-regex`) stay on — a defensible call, not a blanket opt-out. Test files and the E2E Node script get narrow, justified overrides. **`npx eslint .` is clean.**
- **Prettier 3.9.6** with `.prettierrc.json` + `.prettierignore`; `format:check` is a CI gate.
- **husky + lint-staged**: `.husky/pre-commit` runs `eslint --fix` + `prettier --write` on staged files.
- **Clippy**: `cargo clippy --all-targets -- -D warnings` **passes clean**, and is gated in CI (`build.yml:81-83`) alongside `cargo fmt --check`. There is no `[lints]` table or `#![deny]` attributes — the strictness lives in the CI invocation, which is sufficient but means a local `cargo clippy` without `-D warnings` is more permissive than CI.
- **CI quality is above average for a project this size**: three workflows; every third-party action **pinned to a full commit SHA** with a version comment; least-privilege `permissions:` per workflow; `concurrency` cancellation on the release job; CodeQL `security-and-quality` on push/PR/weekly; `cargo audit` + `cargo deny check` + `npm audit` on manifest changes and weekly; Dependabot across npm, cargo, **and** github-actions so the SHA pins do not rot. `deny.toml` sets `yanked = "deny"`, an explicit license allow-list (including `blessing` for bundled SQLite), and warns on multiple-versions/wildcards/unknown sources.
  Two gaps: **(a)** the `npm-audit` job fails today (finding 15); **(b)** `codeql.yml` analyzes `javascript-typescript` only — the Rust backend, where every finding in §6 lives, gets no CodeQL coverage. Adding `language: rust` (or `actions`) to the matrix would close that.

### 8.4 `.gitignore`

Covers `node_modules/`, `dist/`, `dist-ssr/`, `src-tauri/target/`, `src-tauri/gen/`, `.env*`, editor/OS noise, `*.log`, and `launch.json`, with an explicit comment explaining that `Cargo.lock` is committed on purpose (reproducible builds + exact dependency set for `cargo audit`/`cargo deny`) — the right call for a binary. `git ls-files` confirms nothing matching `dist/`, `node_modules`, `.env`, or `*.db` is tracked. Missing: `*.db` / `*.sqlite*` patterns (finding 28).

### 8.5 Tests and coverage gaps

| Suite       | Location                         | Count                     | Runs via                                                                           |
| ----------- | -------------------------------- | ------------------------- | ---------------------------------------------------------------------------------- |
| Unit (TS)   | `src/**/*.test.ts`               | 5 files / **56 tests**    | `npm test` (Vitest, jsdom)                                                         |
| Integration | `tests/integration/*.ts`         | 3 files / ~19 tests       | `npm run test:integration` (separate config, opt-in by design)                     |
| E2E         | `tests/e2e/run.mjs`              | 6 scenarios + screenshots | `npm run test:e2e` (Playwright library, self-hosted harness, spawns Vite on :5199) |
| Unit (Rust) | `src-tauri/src/{db,commands}.rs` | **3 `#[test]`**           | `cargo test`                                                                       |

What is well covered: the pure TS math (`finance.test.ts`, 16 tests), alert classification, `useBack`'s history heuristic, `useContactActions`' phone validation, the overdue dataset invariants, and full UI flows against the mock.

The gap is the backend, and it is structural rather than incidental: **the integration and E2E suites both drive `src/api/mock.ts`, not Rust.** They validate the mock's fidelity to itself. So the code that actually owns the money — transactions, cascade deletes, overpayment accumulation, `paid_date` transitions, the parameter/placeholder lockstep beyond the one regression test, and the `finance.ts` ↔ `db.rs` parity invariant that `CLAUDE.md` treats as a blocker — has essentially no automated coverage. The two Rust tests that do exist are well-chosen (the seeding gate, and the `build_impayes` binding regression with a genuinely informative comment); there just need to be ~15 more in that style, over a temp DB, and one cross-language parity test.

### 8.6 TODO / FIXME / HACK

**None.** A grep for `TODO|FIXME|HACK|XXX` across `src/`, `src-tauri/src/`, `tests/`, and the root configs returns only a false positive inside a `package-lock.json` integrity hash. Unusually clean — and notable because the comments that _are_ present are high-value: several explain a past bug and why the current shape prevents it (`useContactActions.ts:1-15` on WebView `tel:` navigation, `commands.rs:969-974` on the parameter-count bug, `router/index.ts:26-28` warning that the `"not-found"` route name is matched by string elsewhere). That is the good kind of comment density: _why_, not _what_.

### 8.7 Licensing

Neither manifest declares a license: `package.json` has `"private": true` and no `license` field; `Cargo.toml` has no `license`/`license-file`; there is no root `LICENSE` file. So the work is "all rights reserved" by default — fine if intentional, but it should be explicit, and `Cargo.toml` is also missing `repository`/`homepage`. Dependency licenses are handled better than most: `deny.toml` enforces an explicit allow-list (MIT, Apache-2.0 ± LLVM-exception, BSD-2/3, ISC, Zlib, MPL-2.0, CC0-1.0, Unicode-DFS-2016/3.0, OpenSSL, and `blessing` for bundled SQLite) with `confidence-threshold = 0.8`, and `cargo deny check` runs in CI. Nothing copyleft-viral (no GPL/AGPL) can enter the Rust tree without failing that gate. The npm side has no equivalent license gate; the direct set is MIT/Apache-2.0 throughout.

---

## 9. Appendix — raw dependency audit output

### 9.1 `cargo audit` (run from `src-tauri/`)

`cargo-audit` was not installed on this machine; it was installed for this audit (`cargo install cargo-audit --locked`) so the check could actually run rather than be described.

```
    Fetching advisory database from `https://github.com/RustSec/advisory-db.git`
      Loaded 1169 security advisories (from /home/malek/.cargo/advisory-db)
    Updating crates.io index
    Scanning Cargo.lock for vulnerabilities (490 crate dependencies)

Crate:   atk             0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0413
Crate:   atk-sys         0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0416
Crate:   gdk             0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0412
Crate:   gdk-sys         0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0418
Crate:   gdkwayland-sys  0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0411
Crate:   gdkx11          0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0417
Crate:   gdkx11-sys      0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0414
Crate:   gtk             0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  RUSTSEC-2024-0415
Crate:   gtk-sys         0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  (gtk-rs GTK3 set)
Crate:   gtk3-macros     0.18.2  unmaintained  gtk-rs GTK3 bindings - no longer maintained  (gtk-rs GTK3 set)
Crate:   proc-macro-error 1.0.4  unmaintained  proc-macro-error is unmaintained             RUSTSEC-2024-0370
Crate:   unic-char-property 0.9.0 unmaintained `unic-char-property` is unmaintained         RUSTSEC-2025-0081
Crate:   unic-char-range 0.9.0   unmaintained  `unic-char-range` is unmaintained            RUSTSEC-2025-0075
Crate:   unic-common     0.9.0   unmaintained  `unic-common` is unmaintained                RUSTSEC-2025-0080
Crate:   unic-ucd-ident  0.9.0   unmaintained  `unic-ucd-ident` is unmaintained             RUSTSEC-2025-0100
Crate:   unic-ucd-version 0.9.0  unmaintained  `unic-ucd-version` is unmaintained           RUSTSEC-2025-0098
Crate:   glib            0.18.5  unsound       Unsoundness in Iterator/DoubleEndedIterator
                                               impls for glib::VariantStrIter               RUSTSEC-2024-0429

warning: 17 allowed warnings found
```

Exit status 0 — **no vulnerabilities**; all 17 entries are `unmaintained`/`unsound` warnings, which `cargo audit` does not fail on by default.

### 9.2 `cargo outdated`

**Not run — `cargo-outdated` is not installed on this machine.** Rather than install a second tool, direct dependencies were diffed against the crates.io API (`GET /api/v1/crates/<name>` → `crate.max_stable_version`), which is the alternative the audit brief allows. Results:

```
crate                  locked    latest stable   published
tauri                  2.11.5    2.11.5          2026-07-01   current
tauri-build            2.6.3     2.6.3           2026-06-30   current
tauri-plugin-dialog    2.7.2     2.7.2           2026-07-18   current
tauri-plugin-fs        2.5.1     2.5.1           2026-05-02   current  (remove — unused)
tauri-plugin-os        2.3.2     2.3.2           2025-10-27   current
tauri-plugin-opener    2.5.4     2.5.4           2026-05-02   current
serde                  1.0.229   1.0.229         2026-07-18   current
serde_json             1.0.151   1.0.151         2026-07-20   current
chrono                 0.4.45    0.4.45          2026-06-04   current (pre-1.0)
rusqlite               0.32.1    0.40.1          2026-06-06   8 minor versions behind
libsqlite3-sys         0.30.1    0.38.1          2026-06-06   8 minor versions behind
wry                    0.55.1    0.55.1          2026-05-04   current (transitive)
tao                    0.35.3    0.35.3          2026-05-23   current (transitive)
```

Toolchain: `rustc 1.97.0` / `cargo 1.97.0`. Highest MSRV among locked dependencies: **1.88.0** (`darling`, `plist`, `time`) vs `rust-version = "1.77"` declared in `Cargo.toml:7`.

### 9.3 `npm audit`

```
# npm audit report

brace-expansion  <=5.0.7
Severity: high
brace-expansion: DoS via unbounded expansion length causing an out-of-memory
process crash - https://github.com/advisories/GHSA-mh99-v99m-4gvg
fix available via `npm audit fix --force`
Will install vue-tsc@3.3.8, which is a breaking change
node_modules/brace-expansion
  minimatch  2.0.0 - 10.0.2
  Depends on vulnerable versions of brace-expansion
  node_modules/minimatch
    @vue/language-core  <=3.0.0-beta.5
    Depends on vulnerable versions of minimatch
    node_modules/@vue/language-core
      vue-tsc  1.7.0-alpha.0 - 3.0.0-beta.5
      Depends on vulnerable versions of @vue/language-core
      node_modules/vue-tsc
    editorconfig  1.0.0 - 3.0.1
    Depends on vulnerable versions of minimatch
    node_modules/editorconfig
      js-beautify  1.8.9 - 1.15.4
      Depends on vulnerable versions of editorconfig
      Depends on vulnerable versions of glob
      node_modules/js-beautify
        @vue/test-utils  >=2.4.1
        Depends on vulnerable versions of js-beautify
        node_modules/@vue/test-utils
    glob  4.3.0 - 10.5.0
    Depends on vulnerable versions of minimatch
    node_modules/glob

8 high severity vulnerabilities

To address all issues (including breaking changes), run:
  npm audit fix --force
```

`npm audit --audit-level=high` → **exit code 1** (the CI gate in `security.yml:55-56`). All affected packages are devDependencies.

### 9.4 `npm outdated`

```
Package                    Current  Wanted  Latest  Depended by
@tauri-apps/plugin-dialog    2.7.1   2.7.2   2.7.2   payment-schedule-desktop
eslint                      10.7.0  10.8.0  10.8.0   payment-schedule-desktop
jsdom                       25.0.1  25.0.1  29.1.1   payment-schedule-desktop
pinia                        2.3.1   2.3.1   4.0.2   payment-schedule-desktop
playwright                  1.61.1  1.62.0  1.62.0   payment-schedule-desktop
typescript                   5.9.3   5.9.3   7.0.2   payment-schedule-desktop
vite                         7.3.6   7.3.6   8.1.5   payment-schedule-desktop
vue-i18n                    10.0.8  10.0.8  11.4.8   payment-schedule-desktop
vue-router                   4.6.4   4.6.4   5.2.0   payment-schedule-desktop
vue-tsc                     2.2.12  2.2.12   3.3.8   payment-schedule-desktop
```

Environment: Node **24.16.0**, npm **11.13.0** locally; CI pins Node 22; `package.json` declares no `engines`.

---

## 10. Suggested next steps

### Tier 1 — quick wins (each under an hour, no design decisions)

1. **Drop the `fs` plugin and its permissions** (finding 1). Remove `tauri_plugin_fs::init()` (`lib.rs:17`), `tauri-plugin-fs` from `Cargo.toml`, `@tauri-apps/plugin-fs` from `package.json`, and the `fs:default` / `fs:allow-read-file` / `fs:allow-write-file` entries from `capabilities/default.json`. Nothing imports it, so this should be inert — verify the logo still renders (it goes through `asset:`, not `fs`).
2. **Narrow `assetProtocol.scope`** to `["$APPDATA/logo.*"]` (`tauri.conf.json:30`) so the database is no longer fetchable from the renderer.
3. **Clamp the numeric IPC inputs** (findings 2, 3, 4, 13): `upcoming_days` → 1..=365; `interval_days` → 1..=365; `installment_count` → 1..=120; `total_price` → `> 0`; validate `interval_kind` against the three known values. Five small guards in `commands.rs`, plus swapping `db.rs:146`'s `+` for the checked variant already used by the monthly branch.
4. **Parse every incoming date** — run `installments[].due_date` through `parse_date` before insert (finding 10).
5. **Harden the connection at open** (finding 8): add `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL;` to the `execute_batch` at `db.rs:23`.
6. **Wrap `update_settings` in a transaction** (finding 11).
7. **Fix the mutex poisoning footgun** — `lock().unwrap_or_else(|e| e.into_inner())` at the 21 sites (finding 14).
8. **CSV escaping + formula-injection guard** in `exportCsv` (finding 21).
9. **Declare `rust-version = "1.88"`, add `engines.node`, add a `license` field** (findings 20, 27).

### Tier 2 — this week

10. **Introduce a typed error enum and stop leaking SQL to users** (finding 5). Define `enum AppError { NotFound, Conflict(String), Validation(&'static str), Internal }` in the backend, `impl Serialize` with a stable `code`, map codes to i18n keys on the frontend, and keep the detail in `console.error` only. This is the change `CLAUDE.md` treats as a blocker, and it also gives findings 12/13's new validations somewhere to report to.
11. **Add `tauri-plugin-single-instance`** (finding 8) so two processes can never share the file.
12. **Add schema versioning** (finding 9): `PRAGMA user_version` + an ordered migration list applied in one transaction. Do this _before_ the next schema change, not with it.
13. **Backend test suite** (finding 19): ~15 `#[test]`s over a temp DB covering `create_purchase` (equal split, manual split, sum mismatch rolls back, invalid dates rejected), `record_payment` (partial → `paid_date` stays null, full → set, overpayment rejected), `delete_client`/`delete_purchase` cascades, and a parity test asserting `split_amounts`/`add_interval`/`installment_status` agree with `finance.ts` on a shared fixture table.
14. **Fix the failing CI security gate** (finding 15): upgrade `vue-tsc` to 3.x and `@vue/test-utils`, then confirm `npm audit --audit-level=high` exits 0 and `vue-tsc --noEmit` is still clean.
15. **Decide the overpayment rule** (finding 12) — reject, or allow with an explicit confirmation and a credit concept. Right now it silently over-credits and marks the installment paid.
16. **Add error states to the view loaders and a logger to the backend** (findings 17, 18): `tauri-plugin-log` or `tracing` with `warn`/`error` on command failures (no names/phones in the messages), and try/catch + a retry affordance in `ClientsView.load()` / `DashboardView.load()`.

### Tier 3 — larger, deliberate work

17. **Move reads off the main thread and kill the N+1s** (finding 7). Convert the read commands to `async` and rewrite `list_purchases` / `get_client_detail` / `get_dashboard` as set-based `GROUP BY` queries against the existing indices. Benchmark with a few thousand seeded purchases first so the win is measured, not assumed.
18. **`rusqlite` 0.32 → 0.40** (finding 16), which also refreshes the bundled SQLite. Mostly mechanical, but it touches every query site — do it on its own branch with Tier-2's backend tests already in place.
19. **Add a backup/export path** (finding 9): a "backup database now" command (file copy to a user-chosen location via the dialog plugin, which is already a dependency), ideally invoked automatically before cascade deletes. Pair it with a decision on encryption at rest and on PII retention (§7.5).
20. **Frontend majors as one PR** (finding 26): Pinia 4, vue-router 5, vue-i18n 11, Vite 8, and the TS/jsdom bumps, gated on the E2E suite.
21. **Extend CodeQL to Rust** (§8.3) so the backend gets static analysis, and consider splitting `commands.rs` into a thin validating IPC layer over a testable domain module (§6.5) — which would make the input-validation gaps visible by inspection instead of by audit.

### Explicitly checked and found not applicable

- **Updater**: not configured at all (§5.6) — no endpoint, no pubkey, nothing to misconfigure; the trade-off is that shipped copies cannot be patched.
- **Shell / command injection**: no shell plugin, no `Command::new`, no string-built command lines (§5.5).
- **SQL injection**: no user data is ever interpolated into SQL; the one dynamic query builds placeholder _indices_ only (§7.1).
- **`unsafe` Rust**: none (§6.3).
- **Secrets in the frontend bundle**: none; no `.env` exists and no `VITE_` variable is read anywhere (§8.1).
- **Production source maps**: disabled unless `TAURI_ENV_DEBUG`, and confirmed absent from the built `dist/` (§8.2).
- **devTools in release**: not enabled — the `devtools` Cargo feature is not set (§5.3).
- **Vue 2 / Options API / mixed patterns**: none — Vue 3.5, `<script setup>` in all 16 SFCs (§8.1).
- **`v-html` with untrusted data**: the single usage is a static icon map, documented and safe (§8.1).
- **Tauri v1 allowlist**: not present; this is a v2 capability-based app (§3.2).
