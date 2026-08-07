# Licence file format (v1)

Everything needed to mint a licence for paymentSchedule without reading Rust.
The implementation is `src-tauri/src/license.rs`; if the two ever disagree, the
code wins and this file is the bug.

Licences are **offline**. There is no licence server, no activation call, and no
network access anywhere in the validation path. A licence is a signed file the
customer drops next to the database.

---

## 1. Where the file goes

`license.json` in the app-data directory — the same one holding
`payment_schedule.db` and `logo.*`. The bundle identifier is
`tn.paymentschedule` (`src-tauri/tauri.conf.json`), so:

| Platform | Path                                                            |
| -------- | --------------------------------------------------------------- |
| Linux    | `~/.local/share/tn.paymentschedule/license.json`                |
| Windows  | `%APPDATA%\tn.paymentschedule\license.json`                     |
| macOS    | `~/Library/Application Support/tn.paymentschedule/license.json` |

The authoritative location is whatever Tauri's `app_data_dir()` resolves to for
that identifier — the table is the usual result, not a second source of truth. If
in doubt, look for the directory that already contains `payment_schedule.db`.

You do not normally place this file by hand: **Settings → Licence → Import**
validates a licence and installs it here for you.

A file larger than **64 KiB** is rejected without being read.

---

## 2. The envelope

```json
{
  "version": 1,
  "payload": "eyJsaWNlbnNlSWQiOiJQUy0yMDI2LTAwMDEi...",
  "signature": "LIq9yiCuqIH2v-3gj0tgUuotOFed6eWoC3mu0Iyw..."
}
```

| Field       | Type   | Meaning                                                           |
| ----------- | ------ | ----------------------------------------------------------------- |
| `version`   | number | Envelope format version. Must be `1`; anything else is rejected.  |
| `payload`   | string | base64url, **no padding**, of the payload JSON (§3).              |
| `signature` | string | base64url, **no padding**, of the 64 raw Ed25519 signature bytes. |

Both fields use the **URL-safe alphabet** (`-` and `_`, not `+` and `/`) with
padding `=` stripped. Using the standard alphabet for one of them is the single
most likely way to produce a licence that silently fails to verify.

It is a single self-contained envelope rather than a detached `.sig` file so the
customer copies one file and cannot separate a licence from its proof.

---

## 3. The payload

```json
{
  "licenseId": "PS-2026-0001",
  "licensee": "Électro Sfax SARL",
  "issuedAt": "2026-01-15",
  "expiresAt": "2030-01-15",
  "machineId": "9883b9f0ce6aebf5292f77567449a4ad6bdb5bf6b7bb49ecc1f96e9d5c543316",
  "features": ["*"]
}
```

| Field       | Required | Meaning                                                                                                    |
| ----------- | :------: | ---------------------------------------------------------------------------------------------------------- |
| `licenseId` |   yes    | Your reference for this licence. Appears in the app's logs.                                                |
| `licensee`  |   yes    | Shop or company name. **Never written to the log** — treated like customer PII.                            |
| `issuedAt`  |   yes    | ISO-8601 `YYYY-MM-DD`. Must not be later than `expiresAt`.                                                 |
| `expiresAt` |   yes    | ISO-8601 `YYYY-MM-DD`. **Inclusive** — a licence expiring `2026-12-31` still works all day on 31 December. |
| `machineId` |    no    | Machine fingerprint (§5). `null` or absent = floating licence, valid on any machine.                       |
| `features`  |    no    | Reserved (§6). Absent = `[]`.                                                                              |

Notes that matter when minting:

- **Unknown fields are ignored, not rejected.** You can add fields for a future
  build and older installs will still validate the licence.
- Expiry is checked against the machine's **local** date, matching how every
  other date in the app is handled.
- A payload where `issuedAt` is later than `expiresAt` is rejected as malformed.
  An `issuedAt` in the future is _not_ rejected — this version has no
  "not yet valid" state.

---

## 4. What gets signed

```text
signature = Ed25519-sign(secret_key, b"payment-schedule-license.v1." || payload_b64)
```

The signed message is the ASCII of the **`payload` field exactly as it appears in
the file**, prefixed with the literal string `payment-schedule-license.v1.`
(28 bytes, trailing dot included). You are signing the base64 text, _not_ the
decoded JSON and _not_ the whole envelope.

Two consequences worth understanding before you write a signer:

- **There is nothing to canonicalize.** Key order, whitespace and Unicode
  escaping in your payload JSON are irrelevant, because whatever bytes you
  encoded are the bytes that get verified. Once minted, the `payload` string is
  opaque and must be copied verbatim — re-formatting the JSON inside it and
  re-encoding invalidates the signature.
- **The prefix is mandatory.** It exists so a signature the same key produced
  over something else cannot be pasted into a licence envelope. Omitting it
  produces a licence that always fails.

The verifier uses Ed25519 **strict** verification, which rejects small-order
public keys and non-canonical signature scalars. Any standard Ed25519 signing
library produces signatures it accepts.

---

## 5. Machine binding

A licence carrying `machineId` validates only on the machine whose fingerprint
matches. This is the anti-copy mechanism: the same file on a second computer is
rejected.

```text
raw        = the per-OS machine identifier
normalized = raw, with surrounding whitespace and { } braces stripped, lower-cased
machineId  = lowercase_hex( SHA-256( b"payment-schedule.machine-id.v1\0" || normalized ) )
```

The salt is the **30-byte** string `payment-schedule.machine-id.v1` followed by a
**single NUL byte** — 31 bytes in total. Count it: the NUL is part of the salt,
not a separator, and getting the length wrong changes every fingerprint you
derive while looking exactly like a signing failure.

Raw identifier per platform:

| Platform | Source                                             |
| -------- | -------------------------------------------------- |
| Linux    | `/etc/machine-id`                                  |
| Windows  | `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` |
| macOS    | `IOPlatformUUID`                                   |

Normalization is not cosmetic: `/etc/machine-id` ends in a newline and the
Windows GUID is sometimes brace-wrapped and upper-case. Hashing the raw string
gives one machine different fingerprints depending on where the value came from.

The identifier is hashed rather than stored raw so the bare OS UUID never appears
in a file the customer can read or forward.

**Operationally:** you cannot issue a bound licence until the customer reports
their fingerprint, and it changes on an OS reinstall or a motherboard swap, so
either needs a reissue. `license::machine_fingerprint()` is public so the app can
display it; use `machineId: null` for demo and support licences.

Comparison is case-insensitive, so upper-case hex in a licence still matches.

---

## 6. Feature vocabulary

`features` is **parsed and carried but never interpreted** by this version.
Nothing is gated on it yet.

- `["*"]` is the documented "everything" sentinel and is what you should ship
  today. There is no second tier to sell yet, and inventing one in the file
  format before it exists only makes it harder to change.

Reserved for a future tier, should one appear:

| String       | Would cover                             |
| ------------ | --------------------------------------- |
| `reports`    | The Rapports page (still a placeholder) |
| `export`     | CSV export of the filtered Impayés list |
| `alerts`     | The Alertes centre and the header bell  |
| `multi-shop` | Nothing yet                             |

### Unlicensed baseline

Independent of `features`, this is what the app does with **no licence at all**:

> Reading clients and purchases — the list and detail views — **without filters
> and without sorting** requires no licence.

Everything else is licensed, and this **is enforced**: `require_license` in
`src-tauri/src/commands.rs` refuses 20 of the 29 commands. The four baseline
reads _degrade_ rather than refuse — an unlicensed caller is pinned to the active
scope with no server-side search — so the pages still render. `get_settings` and
a language-only `update_settings` also stay open, because a user who cannot read
the current language must still be able to reach the licence screen.
`backup_database` stays open too: it snapshots only what the baseline reads
already show, and an expired licence must never stop a shop copying its own
ledger.

One honest limit: sorting and most filtering run in the browser on rows already
fetched, so the backend never sees them. Disabling those controls communicates
the boundary; it does not enforce it. `scope` is the one real exception.

---

## 7. Signing keys

The public key is **compiled into the binary** — never fetched, never read from
disk, never taken from configuration. It comes from an environment variable at
compile time, and **there is no fallback**:

```sh
PAYMENT_SCHEDULE_LICENSE_PUBKEY=<base64url-nopad public key> npm run tauri build
```

A release build without that variable **does not compile**. The error names the
variable and points here. This used to be a silent default, which meant a
forgotten variable produced a working binary that trusted a keypair whose secret
half is printed further down this page — the build machine is the only place that
mistake is cheap to catch.

Debug builds are handed the development key automatically by `src-tauri/build.rs`,
so `cargo test`, `cargo clippy` and `npm run tauri dev` need no setup.

### Generating a production keypair

Use the issuing tool at `../certificate-generation` — it prints the exact line to
build with:

```sh
java -jar target/certificate-generation.jar keygen --out ~/secure/paymentschedule-signing.key
# → PAYMENT_SCHEDULE_LICENSE_PUBKEY=<base64url>
```

Keep that file offline and out of version control. Anyone holding its `seed` can
mint licences the app accepts, and **there is no revocation** — a leak means
generating a new keypair, rebuilding, and reissuing every outstanding licence.

For release builds the public half also has to reach CI, as the
`PAYMENT_SCHEDULE_LICENSE_PUBKEY` **repository variable** (not a secret — the
workflow reads `vars.`). The release job refuses to build without it, and refuses
the development key outright. Step-by-step in
[README → Building release binaries → Setting it in CI](../README.md#setting-it-in-ci).

> `openssl genpkey -algorithm ed25519` also produces a valid key, but
> `openssl pkey -text -noout` prints it as **hex**, not the base64url-nopad this
> variable expects. Converting by hand is an easy way to embed a wrong key, so
> prefer the tool above.

### The development keypair

Published deliberately, so QA and the worked example in §8 can mint test licences
with no setup. It is **worthless by design** — the secret half is right here:

```text
public: vA58s7GMDPCW-FnoVy7jDxJQWShUznnJM2aFPT5TVsc
seed:   kQcFQb9clF5ooOIKwzjjAPhxaNkXK6zQcOCEC2_llq4
```

**Never build a release with this key, and never sign a customer licence with
this seed.** Anyone reading this page could then mint licences your build
accepts. A release binary handed this key still logs a warning on first
validation — treat that warning as a release blocker. If a file named like a
production key contains the seed above, it is not a production key.

---

## 8. Worked example

A complete minting script. The `machineId` is the fingerprint of the raw
identifier `abc123`, so the output is reproducible.

```python
import base64, hashlib, json
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

PREFIX = b"payment-schedule-license.v1."
SALT   = b"payment-schedule.machine-id.v1\x00"
b64u   = lambda b: base64.urlsafe_b64encode(b).decode().rstrip("=")

def fingerprint(raw_machine_id):
    normalized = raw_machine_id.strip().strip("{}").strip().lower()
    return hashlib.sha256(SALT + normalized.encode()).hexdigest()

def mint(seed_b64, payload):
    key = Ed25519PrivateKey.from_private_bytes(base64.urlsafe_b64decode(seed_b64 + "=="))
    payload_b64 = b64u(json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode())
    signature = key.sign(PREFIX + payload_b64.encode())
    return json.dumps({"version": 1, "payload": payload_b64, "signature": b64u(signature)}, indent=2)

print(mint("kQcFQb9clF5ooOIKwzjjAPhxaNkXK6zQcOCEC2_llq4", {
    "licenseId":  "PS-2026-0001",
    "licensee":   "Électro Sfax SARL",
    "issuedAt":   "2026-01-15",
    "expiresAt":  "2030-01-15",
    "machineId":  fingerprint("abc123"),
    "features":   ["*"],
}))
```

Output — this exact file is pinned by tests in **two** projects, so it cannot
drift from either implementation:

- `the_worked_example_from_the_format_docs_validates` (Rust) checks the app
  accepts it. It skips on a build that embeds a non-development key.
- `CrossLanguageConformanceTest` (Java, in `../certificate-generation`) is the
  stronger one: Ed25519 signatures are deterministic (RFC 8032), so it asserts
  the Java signer reproduces the payload and signature below **byte for byte**.
  That pins the prefix, base64 alphabet, JSON compaction, field order and key
  handling across Python, Java and Rust at once, against a fixed key that no
  environment variable can change.

```json
{
  "version": 1,
  "payload": "eyJsaWNlbnNlSWQiOiJQUy0yMDI2LTAwMDEiLCJsaWNlbnNlZSI6IsOJbGVjdHJvIFNmYXggU0FSTCIsImlzc3VlZEF0IjoiMjAyNi0wMS0xNSIsImV4cGlyZXNBdCI6IjIwMzAtMDEtMTUiLCJtYWNoaW5lSWQiOiI5ODgzYjlmMGNlNmFlYmY1MjkyZjc3NTY3NDQ5YTRhZDZiZGI1YmY2YjdiYjQ5ZWNjMWY5NmU5ZDVjNTQzMzE2IiwiZmVhdHVyZXMiOlsiKiJdfQ",
  "signature": "LIq9yiCuqIH2v-3gj0tgUuotOFed6eWoC3mu0IywVijM4jvFb118ZWTKoup9b44TO921IqXEgJqkhFvz_14WDg"
}
```

---

## 9. Validation outcomes

`license::validate_installed()` returns one of these. It never returns a bare
boolean, so the caller can respond differently to "never bought one" and "renewal
is three days late".

| Status             | Meaning                                                                                                                                         |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `Valid`            | Signature verified, right machine, not past `expiresAt`.                                                                                        |
| `Expired`          | Signature verified; `expiresAt` has passed. Carries the licence.                                                                                |
| `MachineMismatch`  | Signature verified; bound to a different machine. Carries the licence and this machine's fingerprint (or `None` if it could not be determined). |
| `InvalidSignature` | The envelope parsed but the signature did not verify. Nothing inside is trustworthy, so nothing is returned.                                    |
| `Malformed`        | Not a licence file this build can read.                                                                                                         |
| `Missing`          | No file at the expected path — the normal state of a fresh install.                                                                             |
| `ClockTampered`    | The system clock reads earlier than the latest date this install has seen, so any date-dependent verdict is untrustworthy. See §10.             |

Only `Valid` unlocks the app. `ClockTampered` in particular is **not** licensed —
the whole point is that winding the clock back must not buy access.

Checks run in this order, and the order is deliberate: envelope shape →
**signature** → payload shape and dates → machine → expiry. Nothing inside the
payload is parsed until the signature verifies, and a wrong machine is reported
ahead of expiry because a licence issued for someone else's computer was never
valid here at any time.

How the app _reacts_ to each status is implemented in `commands.rs` (which
commands refuse) and in the frontend (`LicenseRequiredPanel`, sidebar padlocks,
the Settings licence section). See the "Licensing" section of `architecture.md`.

---

## 10. Known limitations

Stated plainly so nobody assumes protection that isn't there.

- **Clock rollback is mitigated, not solved.** Expiry is checked against the
  machine's local date. A monotonic watermark (`license_clock_watermark` in the
  `setting` table) records the latest date this install has ever seen, and a clock
  reading earlier yields `ClockTampered` instead of reviving an expired licence.
  That defeats changing the system date. It does **not** defeat restoring an older
  copy of the database, because the watermark lives in that same file, and nothing
  in the app signs or checksums it.
- **A determined attacker can patch the binary.** The public key is a constant in
  the executable; anyone able to replace it can sign their own licences. Signature
  verification raises the cost of casual copying, it does not make the app
  tamper-proof.
- **Machine binding is as stable as the OS identifier.** Reinstalling the OS,
  swapping a motherboard, or restoring onto different hardware changes the
  fingerprint and requires a reissue. Some virtualised and imaged environments
  clone `/etc/machine-id` across hosts, in which case one bound licence would
  validate on every clone.
- **The licence file is not encrypted.** It contains the licensee name and the
  machine fingerprint in a form anyone can decode. It carries no secret — the
  signature protects integrity, not confidentiality — but treat it as readable by
  whoever can reach the app-data directory. (`import_license` copies the file
  with `std::fs::copy`, so its permissions are inherited from wherever the
  customer got it.)
