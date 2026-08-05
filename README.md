# paymentSchedule

**paymentSchedule** is an offline-first desktop app for small Tunisian
electronics / appliance shops (_vente de produits électroménagers_) to track
**installment sales**: clients, purchases (_achats_), payment schedules
(_échéances_), payments, and overdue balances (_impayés_).

Built with **Tauri 2** (Rust core) + **Vue 3** (`<script setup>`, TypeScript,
Composition API) + **Vite**. All data lives in a local **SQLite** database — no
network or cloud dependency.

- 🌍 Trilingual UI — **Arabic (RTL)**, **French**, **English** — default from the
  OS locale, switchable live in Settings.
- 💰 Currency (default **TND**) and date format (default **dd/MM/yyyy**)
  configurable in Settings and applied everywhere.
- 🧾 Auto-computed installment schedules with per-line overrides, partial
  payments, and a full payment audit trail.
- 🔴 Overdue tracking (dashboard counters, due-date alerts, dedicated _Impayés_
  page with filters, contact shortcuts, and CSV export).

---

## Screens

Dashboard (Tableau de bord), Achats, Clients, Paiements, Échéances, Impayés,
Paramètres, plus Alertes/Rapports placeholders. The Dashboard is a
pixel-accurate implementation of `docs/intsallment.png`.

---

## Prerequisites

| Tool          | Version                                                 |
| ------------- | ------------------------------------------------------- |
| **Node.js**   | ≥ 20.19 (Vite 7 requirement; CI uses 22, tested on 24)  |
| **Rust**      | ≥ 1.77 (stable)                                         |
| **Tauri CLI** | v2 (installed as a dev dependency; use `npm run tauri`) |

### Linux system libraries (for `tauri dev` / `tauri build`)

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
     libjavascriptcoregtk-4.1-dev librsvg2-dev build-essential curl wget \
     file libssl-dev libayatana-appindicator3-dev
```

### Windows

- **WebView2 runtime** (preinstalled on Windows 10/11).
- **Microsoft C++ Build Tools** (MSVC) + the Rust `x86_64-pc-windows-msvc` target.

---

## Install & run (development)

```bash
npm install          # install frontend + Tauri CLI
npm run tauri dev    # launch the desktop app (starts Vite, then the Tauri shell)
```

`npm run tauri dev` runs the Vite dev server on `http://localhost:5173` and opens
the native window. On first launch the database is created, and in development
builds it is seeded with demo Tunisian data. Release builds start empty (see
below).

### Browser preview (no Rust)

```bash
npm run dev          # http://localhost:5173 — runs against an in-memory mock
```

Outside the Tauri runtime the app uses a built-in mock backend
(`src/api/mock.ts`) that mirrors the Rust logic, so every screen works in a plain
browser. This is what the automated screenshots and tests use.

### Tests & type-checking

```bash
npm test                 # Vitest unit tests (installment/payment math, overdue logic)
npm run test:integration # Vitest integration tests (api facade + backend flows)
npm run test:e2e         # Playwright end-to-end suite (tests/e2e/run.mjs)
npm run build            # vue-tsc type-check + production build

cd src-tauri && cargo test   # Rust backend tests (commands over a temp SQLite DB)
```

### Toolchain requirements

- **Node >= 22** (declared in `package.json`'s `engines`; CI pins 22).
- **Rust >= 1.88** — the real floor of the locked dependency set, declared as
  `rust-version` in `src-tauri/Cargo.toml` and verified by the `MSRV` CI job so
  the claim cannot drift. `src-tauri/rust-toolchain.toml` pins the channel and
  the `rustfmt`/`clippy` components the gates need.

`src/lib/finance.ts` and `src-tauri/src/db.rs` implement the same installment
math independently. Both test suites assert against the shared fixture
`tests/fixtures/finance-parity.json`, so changing one without the other fails a
test rather than drifting silently.

### Code quality & security

Linting, formatting, and dependency/security scanning are set up for both the
frontend and the Rust backend. A `husky` **pre-commit hook** runs `lint-staged`
(ESLint `--fix` + Prettier) on staged files automatically after `npm install`.

```bash
# Frontend (Vue / TypeScript)
npm run lint             # ESLint (eslint-plugin-vue, typescript-eslint, security, no-unsanitized)
npm run lint:fix         # ESLint with autofix
npm run format           # Prettier write
npm run format:check     # Prettier check (CI gate)

# Rust backend (run from src-tauri/)
cargo fmt --check                          # rustfmt (config: src-tauri/rustfmt.toml)
cargo clippy --all-targets -- -D warnings  # clippy, warnings as errors
cargo audit                                # RustSec advisory scan
cargo deny check                           # advisories/licenses/bans/sources (src-tauri/deny.toml)
```

`cargo audit` / `cargo deny` need the tools installed once
(`cargo install cargo-audit cargo-deny`, or via `taiki-e/install-action` in CI).
CI enforces all of the above on every push/PR (`build.yml` lint gates,
`security.yml` audits, `codeql.yml` static analysis), and Dependabot
(`.github/dependabot.yml`) keeps npm, cargo, and GitHub Actions dependencies
current.

---

## Building release binaries

> **Every release build needs the licence public key.**
>
> ```bash
> export PAYMENT_SCHEDULE_LICENSE_PUBKEY=<base64url-nopad public key>
> ```
>
> There is no default: `src-tauri/src/license.rs` reads it with `env!`, so a
> release build without it **fails to compile**, naming the variable. Debug
> builds (`npm run tauri dev`, `cargo test`) get a development key from
> `src-tauri/build.rs` and need no setup.
>
> Generate a production keypair with the `certificate-generation` tool, which
> prints the line to export. Never build a release with the development key —
> its secret half is published in `docs/license-format.md` §7, so anyone could
> mint licences your build accepts.

### Getting the key value

`keygen` prints the exact line to use:

```bash
cd ../certificate-generation
java -jar target/certificate-generation.jar keygen --out ~/secure/paymentschedule-production.key
# → PAYMENT_SCHEDULE_LICENSE_PUBKEY=<43-character base64url value>
```

For a keypair you already have, read back **only** the public line — never the
`seed=` line, which is the secret that mints licences:

```bash
grep '^publicKey=' ~/secure/paymentschedule-production.key | cut -d= -f2-
```

### Setting it in CI

The release workflow reads the key from a **repository variable** and refuses to
build without one. Set it once, at
[Settings → Secrets and variables → Actions → Variables](https://github.com/MalekGn/payment-schedule-desktop/settings/variables/actions):

| Field | Value                                 |
| ----- | ------------------------------------- |
| Name  | `PAYMENT_SCHEDULE_LICENSE_PUBKEY`     |
| Value | the 43-character base64url public key |

> **It must be a _Variable_, not a _Secret_.** The workflow reads
> `${{ vars.PAYMENT_SCHEDULE_LICENSE_PUBKEY }}`, so a value stored under Secrets
> leaves `vars.` empty and the guard reports **"is not set"** — indistinguishable
> from never having set it at all. This is the easiest hour to lose here.

A public key is not confidential, which is why it is a variable: keeping it
unmasked means a wrong-key release is legible in the logs instead of showing up
as `***`.

With the [GitHub CLI](https://cli.github.com/) instead of the browser:

```bash
gh variable set PAYMENT_SCHEDULE_LICENSE_PUBKEY \
  --repo MalekGn/payment-schedule-desktop \
  --body "<43-character base64url public key>"
```

On a tagged push the guard step prints `Licence public key present (43 chars)`
before the build starts. It fails the job if the variable is missing, **or if it
holds the development key** — a release that trusts a published keypair is worse
than no release.

### Linux (`.deb` + `.AppImage`)

```bash
npm run tauri build
# → src-tauri/target/release/bundle/deb/*.deb
# → src-tauri/target/release/bundle/appimage/*.AppImage
```

### Windows (`.msi` + `.exe` NSIS installer)

Native builds must run **on Windows** (the WiX/NSIS bundlers are Windows-only):

```bash
rustup target add x86_64-pc-windows-msvc
npm run tauri build
# → src-tauri/target/release/bundle/msi/*.msi
# → src-tauri/target/release/bundle/nsis/*-setup.exe
```

**Cross-compiling Windows binaries from Linux is not supported** by the Tauri
bundlers (they rely on Windows-only tooling). Use a Windows machine or a Windows
CI runner (e.g. GitHub Actions `windows-latest` with `tauri-apps/tauri-action`).
The build **configuration** for both targets already lives in
`src-tauri/tauri.conf.json` (`bundle.targets`).

### Releases via CI (`.github/workflows/build.yml`)

Pushing a **version tag** (`v*`) runs a GitHub Actions pipeline that typechecks
and runs the unit tests, then builds native installers on two runners and
publishes them to a **GitHub Release** for that tag:

| Runner           | Installers produced                             |
| ---------------- | ----------------------------------------------- |
| `ubuntu-22.04`   | `.deb` (Debian/Ubuntu) + `.rpm` (RedHat/Fedora) |
| `windows-latest` | `.msi` + NSIS `-setup.exe` (Windows 10/11)      |

The **app version** is set in one place: `version` in `src-tauri/Cargo.toml`.
`src-tauri/tauri.conf.json` deliberately omits its `version` key so Tauri
inherits that value — it drives the installer filenames, the Windows MSI product
version, and the deb/rpm package version. (`package.json` carries its own
`version` for npm metadata; it has no effect on the built app, but keep it in
step.)

To cut a release, bump the version and **commit it before tagging** — the
workflow takes the release name from the tag and the installer names from the
build, so tagging first ships a release whose title and assets disagree:

```bash
git tag v1.0.0
git push origin v1.0.0
```

Both runners attach their installers to one shared Release, published as a
**draft** so you can review the assets and notes before making it public. The
workflow can also be triggered manually via **workflow_dispatch**. Per-OS bundle
targets are set with `tauri build --bundles …`, so each runner emits only its
own package formats.

### App icons

Icons are generated from a single source image:

```bash
npm run tauri icon path/to/logo.png   # writes src-tauri/icons/*
```

A generated set is committed under `src-tauri/icons/`.

---

## Data & storage

Everything is stored locally in the OS **app-data directory**:

| Platform | Location                                 |
| -------- | ---------------------------------------- |
| Linux    | `~/.local/share/tn.paymentschedule.app/` |
| Windows  | `%APPDATA%\tn.paymentschedule.app\`      |

- **`payment_schedule.db`** — the SQLite database (clients, purchases, installments,
  payments, settings). Created on first launch. Demo data is seeded only in
  development builds (`tauri dev`); release bundles start empty. Set
  `PAYMENT_SCHEDULE_SEED=1` to force seeding a fresh DB in a release build.
- **`logo.<ext>`** — the shop logo uploaded in Settings, copied into the app-data
  dir and referenced from the `setting` table. Displayed in the sidebar/header
  via Tauri's asset protocol. Only PNG/JPG/WEBP/GIF are accepted, up to 5 MB,
  and the file contents are checked — not just the extension.
- **`logs/`** — backend log files (also echoed to stdout). Written by
  `tauri-plugin-log` at `Info` level in release builds, `Debug` in dev. They
  record command failures with ids and error codes only, never client names,
  phone numbers or payment notes.

### Backing up

**Settings → Backup database** writes a consistent snapshot to a location you
choose. Take one before emptying the purchase archive: that final delete
cascades to the purchase's installments and cannot be undone.

Purchases follow the same reversible-by-default rule as clients. Editing one is
free while nothing has been paid on it; once a payment is recorded, the amount,
the schedule and the date lock and only the product label can change. Removing a
purchase **archives** it — it disappears from every list and every total and can
be restored exactly — and archiving is refused once a payment has been
collected, so a purchase that took real money stays on the books. A purchase
must be archived before it can be deleted for good.

Clients are safer by design. A client who has any purchase cannot be deleted at
all — you **archive** them instead, which hides them from the client list and
the new-purchase picker while keeping every record, and can be undone at any
time from the "Archivés" tab. Archiving is refused while the client still owes
money, so a client with unpaid installments can be neither deleted nor archived.

To reset the app to a fresh state, delete `payment_schedule.db` and restart. In a
development build it is re-seeded; in a release build it comes back empty.

---

## License

Proprietary — all rights reserved. See [LICENSE](LICENSE). Third-party
dependencies keep their own licences; the Rust tree is restricted to an
allow-list enforced by `cargo deny check licenses`.

---

## Architecture

See [`architecture.md`](./architecture.md) for the component/data-flow overview
and [`features.md`](./features.md) for the feature list and status.

Persistence rule: **the frontend never touches the database or filesystem
directly** — all reads and writes go through typed Tauri commands
(`src-tauri/src/commands.rs`), exposed to Vue via a single gateway
(`src/api/index.ts`).

---

## Business rules (summary)

- **Installment split:** `total / n`, with the rounding remainder placed on the
  **last** installment so the parts sum exactly to the total. Amounts are
  editable per line before saving, with a live sum check.
- **Money:** stored as whole currency units (integers) to keep the split exact.
- **Due dates:** `purchase_date + k × interval` (weekly / monthly / custom days);
  the first installment falls on the purchase date.
- **Partial payments:** each payment accumulates into the installment's
  `paid_amount`; the installment is `paid` once fully covered, `partial`
  otherwise, and `late` when past due with a remaining balance.
- **Payments** are recorded per installment and kept as a separate audit log.
