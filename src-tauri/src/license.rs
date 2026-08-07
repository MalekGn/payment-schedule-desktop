//! Offline licence validation.
//!
//! # Scope
//!
//! This module **only reports what a licence file says**. It decides nothing
//! about how the app should behave. Which commands a [`LicenseStatus`] permits
//! lives in [`crate::commands`], and what the user sees lives in the frontend —
//! keeping the verdict separate from the reaction is what makes the verdict
//! testable without a database, a clock, or a WebView.
//!
//! There is no network access and no licence server. A licence is a file the
//! shop keeper drops next to the database, signed by a key only the vendor
//! holds; the binary carries the matching public key.
//!
//! # File format (v1)
//!
//! The licence lives at `$APPDATA/license.json` — the same directory as
//! `payment_schedule.db` and `logo.*`. It is a self-contained envelope rather
//! than a detached signature, so the file cannot be separated from its proof:
//!
//! ```json
//! {
//!   "version": 1,
//!   "payload": "eyJsaWNlbnNlSWQiOiJQUy0yMDI2LTAwMDEiLCJ...",
//!   "signature": "3q2-7_8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA..."
//! }
//! ```
//!
//! Both `payload` and `signature` are **base64url without padding** — one
//! alphabet for both fields, so a signer can never mix the URL-safe and standard
//! tables. `signature` decodes to the 64 raw Ed25519 signature bytes.
//!
//! ## What is signed
//!
//! ```text
//! signature = Ed25519-sign(secret_key, b"payment-schedule-license.v1." || payload_b64_ascii)
//! ```
//!
//! Two properties this buys, both load-bearing:
//!
//! * The signature covers the **base64 text exactly as it appears in the file**,
//!   not a re-serialization of the decoded object. That removes the JSON
//!   canonicalization problem entirely: there is no key ordering, whitespace or
//!   number formatting for a signer and a verifier to disagree about, and no way
//!   to reorder keys under a still-valid signature. It also means the signature
//!   is checked *before* any untrusted JSON is parsed — see [`validate_bytes`].
//! * The `payment-schedule-license.v1.` domain-separation prefix stops a
//!   signature the same key produced in another context from being replayed as a
//!   licence.
//!
//! ## Payload
//!
//! ```json
//! {
//!   "licenseId": "PS-2026-0001",
//!   "licensee": "Électro Sfax SARL",
//!   "issuedAt": "2026-07-28",
//!   "expiresAt": "2027-07-28",
//!   "machineId": "9f2b…c41e",
//!   "features": ["*"]
//! }
//! ```
//!
//! camelCase and ISO-8601 `YYYY-MM-DD` dates, matching every DTO in
//! [`crate::models`]. `machineId` and `features` are optional. Unknown fields are
//! **ignored, not rejected**, so a licence minted for a later build still
//! validates on an older one.
//!
//! `features` is parsed and carried but never interpreted here; `["*"]` is the
//! documented "everything" sentinel. See `docs/license-format.md`.
//!
//! # The public key
//!
//! [`LICENSE_PUBLIC_KEY_B64`] holds it, base64url without padding. It is
//! **compiled into the binary** and never fetched, read from disk, or taken from
//! configuration — a licence check whose trust anchor is editable is not a check.
//!
//! It comes from the `PAYMENT_SCHEDULE_LICENSE_PUBKEY` environment variable at
//! compile time, and there is **no fallback**:
//!
//! ```sh
//! PAYMENT_SCHEDULE_LICENSE_PUBKEY=<base64url-nopad public key> cargo build --release
//! ```
//!
//! A release build without it does not compile. That is deliberate. The variable
//! used to default to a development key whose secret half is published in
//! `docs/license-format.md`, which meant a forgotten variable produced a
//! perfectly working binary that anyone could mint licences for. Failing on the
//! build machine is the only place that mistake is cheap.
//!
//! Debug builds are handed the development key by `build.rs`, so `cargo test`,
//! `cargo clippy` and `tauri dev` need no setup. A *release* build given that
//! same key on purpose still logs a warning — see [`warn_if_development_key`].
//!
//! # Machine binding
//!
//! A payload carrying `machineId` only validates on the machine whose salted
//! fingerprint matches — see [`machine_fingerprint`]. `machineId: null` (or
//! absent) is a floating licence that validates anywhere, which is what demo and
//! support licences use.
//!
//! # Logging
//!
//! Every rejection is logged at `warn` where the status is constructed, the same
//! way [`crate::error::AppError::internal`] logs in its constructor: a licence
//! that silently refuses to validate is unsupportable. Logs carry the licence id
//! and a reason only — never `licensee`, which is a shop name and stays out of
//! the log under the same rule as client PII (see `architecture.md`).
//!
//! [`LicenseStatus`] itself never crosses the IPC boundary — [`LicenseInfo`]
//! does. `Malformed { reason }` is diagnostic text for the log, not a
//! user-facing message, and is dropped in the projection the way
//! [`crate::error::AppError::Internal`] collapses to an opaque code.

use std::path::Path;
use std::sync::OnceLock;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::NaiveDate;
use ed25519_dalek::{Signature, VerifyingKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::{parse_date, AppError, DbResult};

/// Base64url (no padding) of the 32-byte Ed25519 public key licences are signed
/// with, taken from `PAYMENT_SCHEDULE_LICENSE_PUBKEY` at compile time.
///
/// There is no fallback: a build without the variable fails here rather than
/// producing a binary that trusts some default. `build.rs` supplies the
/// development key for debug builds, so only release builds must set it.
const LICENSE_PUBLIC_KEY_B64: &str = env!(
    "PAYMENT_SCHEDULE_LICENSE_PUBKEY",
    "PAYMENT_SCHEDULE_LICENSE_PUBKEY is not set. A release build must be given the \
     production licence public key; debug builds get a development key from build.rs. \
     See docs/license-format.md section 7."
);

/// The published development public key.
///
/// Kept **only** to recognise it — it is no longer a fallback, and nothing
/// selects it. A release build can still be handed this key deliberately or by
/// copy-paste, and that is worth a warning; see [`warn_if_development_key`].
/// The value is a public key, so nothing secret lives in production code. Its
/// secret half exists only in the test module and in `docs/license-format.md`.
const DEV_PUBLIC_KEY_B64: &str = "vA58s7GMDPCW-FnoVy7jDxJQWShUznnJM2aFPT5TVsc";

/// Prepended to the signed message so a signature made by this key in another
/// context cannot be replayed as a licence.
const SIGNING_PREFIX: &[u8] = b"payment-schedule-license.v1.";

/// Mixed into the machine fingerprint so the stored value is specific to this
/// app and cannot be correlated with the raw OS identifier or other software.
const MACHINE_ID_SALT: &[u8] = b"payment-schedule.machine-id.v1\0";

/// The only envelope version this build understands.
const ENVELOPE_VERSION: u32 = 1;

/// Licence file name inside the app-data directory.
const LICENSE_FILE_NAME: &str = "license.json";

/// A licence is a few hundred bytes. The cap exists so a hostile or corrupt file
/// cannot be read into memory at all — it is checked against the file metadata
/// before the read, mirroring `LOGO_MAX_BYTES` in [`crate::commands`].
const LICENSE_MAX_BYTES: u64 = 64 * 1024;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The signed envelope, as it appears on disk.
#[derive(Debug, Deserialize)]
struct Envelope {
    version: u32,
    /// base64url (no padding) of the payload JSON.
    payload: String,
    /// base64url (no padding) of the 64 raw Ed25519 signature bytes.
    signature: String,
}

/// The licence itself: the decoded payload of a signature that has **already
/// verified**. Every field here is vendor-attested, so callers may trust it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct License {
    pub license_id: String,
    /// Shop or company name. Treated as PII-adjacent: never logged.
    pub licensee: String,
    /// ISO-8601 `YYYY-MM-DD`. Guaranteed parseable — validation rejects it otherwise.
    pub issued_at: String,
    /// ISO-8601 `YYYY-MM-DD`. Guaranteed parseable, and never before `issued_at`.
    pub expires_at: String,
    /// Salted machine fingerprint this licence is bound to, or `None` for a
    /// floating licence that validates on any machine.
    #[serde(default)]
    pub machine_id: Option<String>,
    /// Reserved for later feature gating. Parsed and carried, never interpreted.
    /// `["*"]` is the documented "everything" sentinel.
    #[serde(default)]
    pub features: Vec<String>,
}

/// The outcome of validating a licence.
///
/// Deliberately not a `bool` and not a `Result`: "no licence installed" and
/// "expired last week" are ordinary states a caller reacts to differently, not
/// errors. A genuine fault (an unreadable file, a broken app-data path) is the
/// `Err` half of [`DbResult`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseStatus {
    /// Signature verified, bound to this machine, not past its expiry date.
    Valid(License),
    /// Signature verified, but `expires_at` is in the past. The licence is
    /// carried because its contents are vendor-attested and worth showing.
    Expired {
        license: License,
        expired_on: NaiveDate,
    },
    /// Signature verified, but the licence is bound to a different machine.
    /// `local` is this machine's fingerprint, or `None` when it could not be
    /// determined at all — support needs to tell those two cases apart.
    MachineMismatch {
        license: License,
        local: Option<String>,
    },
    /// The envelope parsed but the signature did not verify against the embedded
    /// public key. Nothing inside the payload can be trusted, so nothing is
    /// returned with it.
    InvalidSignature,
    /// The file is not a licence this build can read. `reason` is diagnostic text
    /// for the log — see the note on IPC in the module docs.
    Malformed { reason: &'static str },
    /// No licence file at the expected path. The normal state of a fresh install.
    Missing,
    /// The system clock reads earlier than the latest date this install has ever
    /// seen, so any date-dependent verdict is untrustworthy. See
    /// [`apply_clock_guard`].
    ClockTampered { watermark: NaiveDate },
}

impl LicenseStatus {
    /// Whether the app is licensed. Only [`LicenseStatus::Valid`] qualifies —
    /// every other variant, including `ClockTampered`, is unlicensed.
    pub fn is_valid(&self) -> bool {
        matches!(self, LicenseStatus::Valid(_))
    }

    /// The licence, when the signature verified. `None` for the variants where
    /// nothing inside the file can be trusted.
    fn license(&self) -> Option<&License> {
        match self {
            LicenseStatus::Valid(license)
            | LicenseStatus::Expired { license, .. }
            | LicenseStatus::MachineMismatch { license, .. } => Some(license),
            _ => None,
        }
    }

    /// The stable discriminant sent to the frontend, camelCase to match the
    /// string-union convention every other enum uses across IPC.
    fn tag(&self) -> &'static str {
        match self {
            LicenseStatus::Valid(_) => "valid",
            LicenseStatus::Expired { .. } => "expired",
            LicenseStatus::MachineMismatch { .. } => "machineMismatch",
            LicenseStatus::InvalidSignature => "invalidSignature",
            LicenseStatus::Malformed { .. } => "malformed",
            LicenseStatus::Missing => "missing",
            LicenseStatus::ClockTampered { .. } => "clockTampered",
        }
    }

    /// Project onto the wire type. See [`LicenseInfo`] for why this is not a
    /// `Serialize` impl on the status itself.
    pub fn to_info(&self, local_machine: Option<String>) -> LicenseInfo {
        LicenseInfo {
            status: self.tag(),
            license: self.license().cloned(),
            expired_on: match self {
                LicenseStatus::Expired { expired_on, .. } => Some(expired_on.to_string()),
                _ => None,
            },
            machine_id: local_machine,
        }
    }
}

/// The IPC projection of a [`LicenseStatus`].
///
/// `LicenseStatus` deliberately does **not** derive `Serialize`. Two of its
/// variants carry things that must not cross the boundary as-is:
///
/// * `Malformed { reason }` is diagnostic text written for the log. Shipping it
///   would put internal parser detail in front of a shop keeper, the same
///   mistake [`crate::error::AppError::Internal`] exists to prevent — so it is
///   dropped here and only the `"malformed"` tag survives.
/// * `Expired { expired_on: NaiveDate }` is a Rust date type; the wire uses ISO
///   `YYYY-MM-DD` strings like every other date in the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    /// `"valid" | "expired" | "machineMismatch" | "invalidSignature"
    /// | "malformed" | "missing" | "clockTampered"`.
    pub status: &'static str,
    /// Present only when the signature verified, so its contents are attested.
    pub license: Option<License>,
    /// ISO date, set only for `"expired"`.
    pub expired_on: Option<String>,
    /// **This machine's** fingerprint, not the licence's. The customer has to
    /// read it off the screen before a bound licence can be issued to them.
    pub machine_id: Option<String>,
}

/// Managed Tauri state holding the current licence verdict.
///
/// Validation touches the filesystem and hashes a machine identifier, so it runs
/// on a schedule rather than per command: once at startup, then on every tick of
/// `commands::start_license_watcher`, plus whenever `import_license` installs a
/// file. Three writers is why this is a lock and not a plain field.
///
/// Read it, do not cache it. `require_license` calls [`Self::is_valid`] on every
/// gated command precisely so a verdict that changes mid-session takes effect
/// without a restart.
pub struct LicenseState {
    status: std::sync::RwLock<LicenseStatus>,
}

impl LicenseState {
    pub fn new(status: LicenseStatus) -> Self {
        LicenseState {
            status: std::sync::RwLock::new(status),
        }
    }

    /// Read the current verdict, tolerating a poisoned lock for the same reason
    /// [`crate::db::Db::lock`] does: under `tauri dev` one panicking command must
    /// not brick every later one. The guarded value is a plain enum, so
    /// recovering it is sound.
    pub fn get(&self) -> LicenseStatus {
        self.status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set(&self, status: LicenseStatus) {
        *self.status.write().unwrap_or_else(|e| e.into_inner()) = status;
    }

    pub fn is_valid(&self) -> bool {
        self.status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_valid()
    }
}

// ---------------------------------------------------------------------------
// Clock guard
// ---------------------------------------------------------------------------

/// `setting` key holding the latest date this install has ever observed.
///
/// It lives in the existing key/value `setting` table, so it needs no migration.
/// It must **never** be added to [`crate::models::Settings`] or `SettingsPatch`:
/// the former is serialized straight to the renderer, and the latter is written
/// by it — either would hand the value to the code it defends against.
pub const CLOCK_WATERMARK_KEY: &str = "license_clock_watermark";

/// Reject a date-dependent verdict when the clock has moved backwards.
///
/// Expiry is checked against the local system clock, so winding the date back a
/// year would otherwise revive any expired licence. Comparing against the latest
/// date the install has ever seen catches that.
///
/// Only `Valid` and `Expired` are rewritten: they are the two verdicts that
/// depend on the clock. A wrong machine or a broken signature is equally wrong
/// whatever the date says, and reporting those accurately is more useful.
///
/// This defends against a user changing the system date. It does **not** defend
/// against restoring an old copy of the database, since the watermark lives in
/// that same file — a documented limitation, not an oversight.
pub fn apply_clock_guard(
    status: LicenseStatus,
    today: NaiveDate,
    watermark: Option<NaiveDate>,
) -> LicenseStatus {
    let Some(watermark) = watermark else {
        return status;
    };
    if today >= watermark {
        return status;
    }

    match status {
        LicenseStatus::Valid(_) | LicenseStatus::Expired { .. } => {
            log::warn!(
                "system clock reads {today}, behind the recorded high-water mark {watermark}"
            );
            LicenseStatus::ClockTampered { watermark }
        }
        other => other,
    }
}

/// The watermark to persist, or `None` when the stored one is already current.
///
/// Returning `None` for the common case keeps a normal launch read-only against
/// the `setting` table.
pub fn next_watermark(watermark: Option<NaiveDate>, today: NaiveDate) -> Option<NaiveDate> {
    match watermark {
        Some(mark) if mark >= today => None,
        _ => Some(today),
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a licence for this installation: `$APPDATA/license.json`, today's
/// local date, this machine's fingerprint.
///
/// Returns `Err` only when the app-data path cannot be resolved or the file
/// exists but cannot be read — a missing file is [`LicenseStatus::Missing`].
pub fn validate_installed(app: &tauri::AppHandle) -> DbResult<LicenseStatus> {
    use tauri::Manager;

    let path = app.path().app_data_dir()?.join(LICENSE_FILE_NAME);
    validate_file(&path, crate::db::today(), machine_fingerprint().as_deref())
}

/// Read and validate the licence at `path`.
///
/// Split from [`validate_installed`] so it is testable without a Tauri
/// `AppHandle`, and from [`validate_bytes`] so the interesting logic needs no
/// filesystem at all.
pub fn validate_file(
    path: &Path,
    today: NaiveDate,
    local_machine: Option<&str>,
) -> DbResult<LicenseStatus> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LicenseStatus::Missing),
        Err(e) => return Err(AppError::internal(format!("licence file metadata: {e}"))),
    };

    if !meta.is_file() {
        return Ok(malformed("licence path is not a regular file"));
    }
    // Checked against the metadata, so an oversized file is never read at all.
    if meta.len() > LICENSE_MAX_BYTES {
        log::warn!(
            "licence file is {} bytes, over the {LICENSE_MAX_BYTES}-byte cap",
            meta.len()
        );
        return Ok(malformed("licence file exceeds the size cap"));
    }

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        // The file can vanish between the metadata call and the read; that is
        // still "no licence installed", not a fault worth surfacing.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LicenseStatus::Missing),
        Err(e) => return Err(AppError::internal(format!("licence file read: {e}"))),
    };

    Ok(validate_bytes(&bytes, today, local_machine))
}

/// Validate licence bytes against the embedded public key.
///
/// Pure: no filesystem, no clock, no hardware — `today` and `local_machine` are
/// injected so every branch is testable. `local_machine` is `None` when this
/// machine's fingerprint could not be determined.
///
/// # Order of checks
///
/// The order is the security property, not an implementation detail:
///
/// 1. Envelope shape and version.
/// 2. **Signature.** Everything after this point reads attacker-supplied JSON,
///    so nothing after this point runs until the vendor's signature is proven.
/// 3. Payload shape and dates.
/// 4. Machine binding — **before** expiry, deliberately. A licence issued for
///    another machine was never valid here at any time, which is a stronger and
///    more useful statement than "yours ran out".
/// 5. Expiry, inclusive: a licence expiring today is still valid today.
///
/// Every path that cannot prove a licence good returns a non-`Valid` status;
/// there is no branch that fails open.
pub fn validate_bytes(
    bytes: &[u8],
    today: NaiveDate,
    local_machine: Option<&str>,
) -> LicenseStatus {
    let envelope: Envelope = match serde_json::from_slice(bytes) {
        Ok(envelope) => envelope,
        Err(_) => return malformed("file is not a licence envelope object"),
    };

    if envelope.version != ENVELOPE_VERSION {
        return malformed("unsupported licence envelope version");
    }

    let signature = match decode_signature(&envelope.signature) {
        Some(signature) => signature,
        None => return malformed("signature is not 64 base64url-encoded bytes"),
    };

    let Some(key) = verifying_key() else {
        // The embedded key constant is unusable. Fail closed: without a trust
        // anchor no licence can be proven good.
        return invalid_signature("the embedded public key could not be loaded");
    };

    // Verify the base64 text exactly as it appears in the file, prefixed for
    // domain separation. Nothing is decoded before this succeeds.
    let mut signed = Vec::with_capacity(SIGNING_PREFIX.len() + envelope.payload.len());
    signed.extend_from_slice(SIGNING_PREFIX);
    signed.extend_from_slice(envelope.payload.as_bytes());

    // `verify_strict` over `verify`: it rejects small-order public keys and
    // non-canonical signature scalars, which plain `verify` accepts.
    if key.verify_strict(&signed, &signature).is_err() {
        return invalid_signature("Ed25519 signature did not verify");
    }

    // --- past this line the bytes are vendor-attested -----------------------

    let payload = match URL_SAFE_NO_PAD.decode(&envelope.payload) {
        Ok(payload) => payload,
        Err(_) => return malformed("payload is not valid base64url"),
    };
    let license: License = match serde_json::from_slice(&payload) {
        Ok(license) => license,
        Err(_) => return malformed("payload is missing required licence fields"),
    };

    let (Ok(issued_at), Ok(expires_at)) = (
        parse_date(&license.issued_at),
        parse_date(&license.expires_at),
    ) else {
        return malformed("issuedAt or expiresAt is not an ISO YYYY-MM-DD date");
    };
    if issued_at > expires_at {
        return malformed("issuedAt is later than expiresAt");
    }

    if let Some(bound) = license.machine_id.as_deref() {
        // Case-insensitive: the fingerprint is hex, and a signer emitting it in
        // upper case should not lock a customer out of their own licence.
        if !local_machine.is_some_and(|local| local.eq_ignore_ascii_case(bound)) {
            log::warn!(
                "licence {} is bound to another machine (local fingerprint {})",
                license.license_id,
                if local_machine.is_some() {
                    "differs"
                } else {
                    "unavailable"
                }
            );
            return LicenseStatus::MachineMismatch {
                license,
                local: local_machine.map(str::to_owned),
            };
        }
    }

    if today > expires_at {
        log::warn!("licence {} expired on {expires_at}", license.license_id);
        return LicenseStatus::Expired {
            license,
            expired_on: expires_at,
        };
    }

    LicenseStatus::Valid(license)
}

/// Build a [`LicenseStatus::Malformed`], logging the reason as it is created.
///
/// The logging lives here rather than at each call site for the same reason
/// [`crate::error::AppError::internal`] does it in its constructor: this is the
/// one place guaranteed to see the reason, and a licence that quietly refuses to
/// validate is impossible to support.
fn malformed(reason: &'static str) -> LicenseStatus {
    log::warn!("licence rejected as malformed: {reason}");
    LicenseStatus::Malformed { reason }
}

/// Build a [`LicenseStatus::InvalidSignature`], logging why. `reason` is for the
/// log only — the status itself carries nothing, because when a signature fails
/// there is nothing trustworthy to carry.
fn invalid_signature(reason: &'static str) -> LicenseStatus {
    log::warn!("licence rejected: {reason}");
    LicenseStatus::InvalidSignature
}

/// Decode a base64url signature into exactly 64 bytes.
fn decode_signature(encoded: &str) -> Option<Signature> {
    let bytes = URL_SAFE_NO_PAD.decode(encoded).ok()?;
    let bytes: [u8; SIGNATURE_LENGTH] = bytes.try_into().ok()?;
    Some(Signature::from_bytes(&bytes))
}

/// The public key licences are verified against, decoded once.
///
/// `None` means the embedded constant is not a valid Ed25519 public key — a
/// build-configuration mistake. It is logged once and every licence then fails
/// closed rather than panicking, because `panic = "abort"` in release would turn
/// a bad `PAYMENT_SCHEDULE_LICENSE_PUBKEY` into a crash on startup.
fn verifying_key() -> Option<&'static VerifyingKey> {
    static KEY: OnceLock<Option<VerifyingKey>> = OnceLock::new();

    KEY.get_or_init(|| {
        warn_if_development_key();

        let bytes = URL_SAFE_NO_PAD.decode(LICENSE_PUBLIC_KEY_B64).ok();
        let bytes: Option<[u8; PUBLIC_KEY_LENGTH]> = bytes.and_then(|b| b.try_into().ok());
        let Some(bytes) = bytes else {
            log::error!("embedded licence public key is not 32 base64url-encoded bytes");
            return None;
        };

        match VerifyingKey::from_bytes(&bytes) {
            Ok(key) => Some(key),
            Err(e) => {
                log::error!("embedded licence public key is not a valid Ed25519 point: {e}");
                None
            }
        }
    })
    .as_ref()
}

/// Warn when a release binary is still trusting the published development key.
fn warn_if_development_key() {
    // `cfg!` rather than `#[cfg]` so the constant is referenced in every profile.
    // Under `#[cfg(not(debug_assertions))]` the whole branch vanishes from a debug
    // build, which makes `DEV_PUBLIC_KEY_B64` read as dead code and fails
    // `clippy -D warnings`. Both forms compile to nothing in debug; only this one
    // keeps the reference. Debug builds are *expected* to carry this key —
    // `build.rs` puts it there — so warning about it then would be noise.
    if !cfg!(debug_assertions) && LICENSE_PUBLIC_KEY_B64 == DEV_PUBLIC_KEY_B64 {
        log::warn!(
            "licence verification is using the published development public key, \
             whose secret half is in docs/license-format.md — anyone could mint \
             licences this build accepts. Rebuild with PAYMENT_SCHEDULE_LICENSE_PUBKEY \
             set to the production key."
        );
    }
}

// ---------------------------------------------------------------------------
// Machine fingerprint
// ---------------------------------------------------------------------------

/// This machine's salted fingerprint, or `None` if the OS identifier could not
/// be read.
///
/// `pub` because the vendor cannot issue a bound licence until the customer
/// reports this value, so it has to be displayable somewhere in the UI.
///
/// The source is the per-OS stable machine identifier: `/etc/machine-id` on
/// Linux, `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` on Windows,
/// `IOPlatformUUID` on macOS. It changes on a reinstall or a motherboard swap,
/// which means a bound licence has to be reissued after either.
pub fn machine_fingerprint() -> Option<String> {
    static FINGERPRINT: OnceLock<Option<String>> = OnceLock::new();

    FINGERPRINT
        .get_or_init(|| match machine_uid::get() {
            Ok(raw) => Some(fingerprint_of(&raw)),
            Err(e) => {
                log::warn!(
                    "machine identifier unavailable, licence binding cannot be checked: {e}"
                );
                None
            }
        })
        .clone()
}

/// Salted SHA-256 of a normalized raw machine identifier, lower-case hex.
///
/// Normalization is load-bearing, not cosmetic: `/etc/machine-id` ends in a
/// newline and the Windows `MachineGuid` is sometimes brace-wrapped. Hashing the
/// raw string would give one machine different fingerprints depending on where
/// the value came from, silently invalidating a licence that was correct.
///
/// The identifier is hashed rather than stored raw so the bare OS UUID never
/// appears in a file the customer can read or forward.
fn fingerprint_of(raw: &str) -> String {
    let normalized = raw
        .trim_matches(|c: char| c.is_whitespace() || c == '{' || c == '}')
        .to_ascii_lowercase();

    let mut hasher = Sha256::new();
    hasher.update(MACHINE_ID_SALT);
    hasher.update(normalized.as_bytes());
    hex_lower(&hasher.finalize())
}

/// Lower-case hex. Avoids a `hex` dependency and, unlike a `write!`-based
/// version, has no `Result` to discard: indexing a 16-byte table with a nibble
/// cannot fail.
fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
// Note on running this suite: almost every test here signs with the development
// seed and validates through the key compiled into the binary, so it assumes a
// build embedding the development key. `build.rs` guarantees that for every
// debug build — including when `PAYMENT_SCHEDULE_LICENSE_PUBKEY` is exported in
// your shell, which it overrides — so `cargo test` is hermetic and this suite
// does not depend on the environment.
//
// The `require_dev_key!` guards below are belt-and-braces for that invariant: if
// `build.rs` ever stops forcing the development key, the two tests that assert
// against the *documented* example will say so and skip rather than fail
// mysteriously.
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::path::PathBuf;

    /// The secret half of the development key, as published in
    /// `docs/license-format.md` §7.
    ///
    /// It lives here, in test scope, rather than in the module header: nothing
    /// in the shipped binary needs a private seed, and secret-shaped material
    /// should not sit in production source even when the secret is worthless.
    const DEV_SECRET_SEED_B64: &str = "kQcFQb9clF5ooOIKwzjjAPhxaNkXK6zQcOCEC2_llq4";

    /// A fingerprint-shaped value (64 lower-case hex chars) for binding tests.
    const MACHINE_A: &str = "aaaa000000000000000000000000000000000000000000000000000000000001";
    const MACHINE_B: &str = "bbbb000000000000000000000000000000000000000000000000000000000002";

    /// Whether this build embeds the development key.
    ///
    /// Normally true: `build.rs` supplies it for every debug build. It is false
    /// when someone exports `PAYMENT_SCHEDULE_LICENSE_PUBKEY` — a production key,
    /// say — and then runs `cargo test`. The handful of tests that assert against
    /// the *documented* example only make sense in the first case, so they skip
    /// loudly in the second rather than failing for a reason that is not a defect.
    ///
    /// The authoritative byte-exact conformance vector lives in the
    /// `certificate-generation` project, where it is checked against a fixed key
    /// and cannot be skipped by an environment variable.
    fn embedded_is_development_key() -> bool {
        LICENSE_PUBLIC_KEY_B64 == DEV_PUBLIC_KEY_B64
    }

    /// Skip a doc-vector test when this build does not embed the development key.
    macro_rules! require_dev_key {
        ($test:literal) => {
            if !embedded_is_development_key() {
                eprintln!(
                    "skipping {}: this build embeds a non-development licence key \
                     (PAYMENT_SCHEDULE_LICENSE_PUBKEY is set)",
                    $test
                );
                return;
            }
        };
    }

    fn dev_key() -> SigningKey {
        let seed: [u8; 32] = URL_SAFE_NO_PAD
            .decode(DEV_SECRET_SEED_B64)
            .expect("dev seed is base64url")
            .try_into()
            .expect("dev seed is 32 bytes");
        SigningKey::from_bytes(&seed)
    }

    /// A second key that no build trusts.
    fn other_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn d(iso: &str) -> NaiveDate {
        NaiveDate::parse_from_str(iso, "%Y-%m-%d").expect("test date is ISO")
    }

    /// Build an envelope the way an external signer would, with every knob the
    /// tests need to vary exposed.
    fn envelope_with(key: &SigningKey, payload_json: &str, version: u32, prefix: &[u8]) -> Vec<u8> {
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        let mut signed = prefix.to_vec();
        signed.extend_from_slice(payload_b64.as_bytes());
        let signature_b64 = URL_SAFE_NO_PAD.encode(key.sign(&signed).to_bytes());
        format!(
            r#"{{"version":{version},"payload":"{payload_b64}","signature":"{signature_b64}"}}"#
        )
        .into_bytes()
    }

    /// The happy-path envelope: dev key, current version, correct prefix.
    fn signed(payload_json: &str) -> Vec<u8> {
        envelope_with(&dev_key(), payload_json, ENVELOPE_VERSION, SIGNING_PREFIX)
    }

    fn payload_json(expires_at: &str, machine_id: Option<&str>) -> String {
        serde_json::json!({
            "licenseId": "PS-2026-0001",
            "licensee": "Électro Sfax SARL",
            "issuedAt": "2026-01-01",
            "expiresAt": expires_at,
            "machineId": machine_id,
            "features": ["*"],
        })
        .to_string()
    }

    /// An unbound licence expiring well after any `today` the tests use.
    fn valid_bytes() -> Vec<u8> {
        signed(&payload_json("2030-01-01", None))
    }

    fn temp_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the epoch")
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "payment_schedule_license_test_{tag}_{}_{nanos}.json",
            std::process::id()
        ));
        path
    }

    // -- signature and envelope --------------------------------------------

    #[test]
    fn a_correctly_signed_licence_validates_and_round_trips_its_fields() {
        let status = validate_bytes(&valid_bytes(), d("2026-07-28"), Some(MACHINE_A));
        let LicenseStatus::Valid(license) = status else {
            panic!("expected Valid, got {status:?}");
        };
        assert_eq!(license.license_id, "PS-2026-0001");
        assert_eq!(license.licensee, "Électro Sfax SARL");
        assert_eq!(license.issued_at, "2026-01-01");
        assert_eq!(license.expires_at, "2030-01-01");
        assert_eq!(license.machine_id, None);
        // `features` is carried verbatim and not interpreted anywhere here.
        assert_eq!(license.features, vec!["*".to_string()]);
    }

    #[test]
    fn a_single_flipped_payload_byte_is_rejected() {
        // The whole point of the signature: editing the expiry date in a text
        // editor must not produce a licence this build accepts.
        let original = valid_bytes();
        // Corrupt a character inside the payload field, leaving the surrounding
        // JSON structure intact — otherwise this would only prove the envelope
        // no longer parses.
        let payload_start = original
            .windows(11)
            .position(|w| w == br#""payload":""#)
            .expect("envelope has a payload field")
            + 11;

        // Every position in the payload must be covered, not just a lucky one.
        let payload_len = original[payload_start..]
            .iter()
            .position(|&b| b == b'"')
            .expect("payload field is quoted");

        for offset in 0..payload_len {
            let mut bytes = original.clone();
            let at = payload_start + offset;
            bytes[at] = if bytes[at] == b'A' { b'B' } else { b'A' };

            assert_eq!(
                validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_A)),
                LicenseStatus::InvalidSignature,
                "corrupting payload byte {offset} must not produce an accepted licence"
            );
        }
    }

    #[test]
    fn a_signature_from_an_untrusted_key_is_rejected() {
        // Anyone can generate an Ed25519 keypair and sign a well-formed payload;
        // only the vendor's key may produce a licence this build honours.
        let bytes = envelope_with(
            &other_key(),
            &payload_json("2030-01-01", None),
            ENVELOPE_VERSION,
            SIGNING_PREFIX,
        );
        assert_eq!(
            validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_A)),
            LicenseStatus::InvalidSignature
        );
    }

    #[test]
    fn a_signature_without_the_domain_prefix_is_rejected() {
        // Pins domain separation: a signature the vendor key produced over some
        // other payload must not be transplantable into a licence envelope.
        let bytes = envelope_with(
            &dev_key(),
            &payload_json("2030-01-01", None),
            ENVELOPE_VERSION,
            b"",
        );
        assert_eq!(
            validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_A)),
            LicenseStatus::InvalidSignature
        );
    }

    #[test]
    fn an_unsigned_garbage_payload_reports_invalid_signature_not_malformed() {
        // Pins the verify-before-parse ordering. If the payload were decoded and
        // parsed first, this would report Malformed — which would mean untrusted
        // JSON was being parsed before any signature check.
        let payload_b64 = URL_SAFE_NO_PAD.encode(b"this is not JSON at all");
        let signature_b64 = URL_SAFE_NO_PAD.encode([0u8; SIGNATURE_LENGTH]);
        let bytes =
            format!(r#"{{"version":1,"payload":"{payload_b64}","signature":"{signature_b64}"}}"#)
                .into_bytes();

        assert_eq!(
            validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_A)),
            LicenseStatus::InvalidSignature
        );
    }

    #[test]
    fn the_embedded_public_key_is_a_usable_ed25519_key() {
        // A typo in the constant, or a bad PAYMENT_SCHEDULE_LICENSE_PUBKEY, would
        // otherwise only show up as every licence failing in the field.
        assert!(verifying_key().is_some());
    }

    #[test]
    fn the_documented_dev_seed_matches_the_embedded_public_key() {
        // `docs/license-format.md` §7 publishes a keypair, and `build.rs` compiles
        // its public half into every debug build. Three copies of that value now
        // exist — the docs, build.rs and DEV_PUBLIC_KEY_B64 — so this derives the
        // public key from the seed and checks all of them agree at once. If they
        // drift, every documented example silently stops working.
        let derived = URL_SAFE_NO_PAD.encode(dev_key().verifying_key().to_bytes());
        assert_eq!(
            derived, DEV_PUBLIC_KEY_B64,
            "seed and DEV_PUBLIC_KEY_B64 disagree"
        );

        require_dev_key!("the_documented_dev_seed_matches_the_embedded_public_key");
        assert_eq!(
            derived, LICENSE_PUBLIC_KEY_B64,
            "build.rs did not compile in the development key"
        );
    }

    // -- malformed input ----------------------------------------------------

    #[test]
    fn structurally_broken_envelopes_are_malformed() {
        let today = d("2026-07-28");
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("not JSON", b"neither JSON nor a licence".to_vec()),
            ("empty file", Vec::new()),
            (
                "missing signature field",
                br#"{"version":1,"payload":"eyJ9"}"#.to_vec(),
            ),
            (
                "future envelope version",
                envelope_with(
                    &dev_key(),
                    &payload_json("2030-01-01", None),
                    ENVELOPE_VERSION + 1,
                    SIGNING_PREFIX,
                ),
            ),
            (
                "signature is not base64url",
                br#"{"version":1,"payload":"eyJ9","signature":"not base64!!"}"#.to_vec(),
            ),
            (
                "signature is the wrong length",
                format!(
                    r#"{{"version":1,"payload":"eyJ9","signature":"{}"}}"#,
                    URL_SAFE_NO_PAD.encode([0u8; SIGNATURE_LENGTH - 1])
                )
                .into_bytes(),
            ),
        ];

        for (label, bytes) in cases {
            assert!(
                matches!(
                    validate_bytes(&bytes, today, Some(MACHINE_A)),
                    LicenseStatus::Malformed { .. }
                ),
                "{label} should be Malformed"
            );
        }
    }

    #[test]
    fn a_signed_payload_with_broken_dates_is_malformed() {
        let today = d("2026-07-28");

        // Correctly signed, so this is reached only after the signature passes —
        // a vendor mistake, not an attack.
        let bad_date = signed(&payload_json("28/07/2027", None));
        assert!(matches!(
            validate_bytes(&bad_date, today, Some(MACHINE_A)),
            LicenseStatus::Malformed { .. }
        ));

        let inverted = signed(
            &serde_json::json!({
                "licenseId": "PS-2026-0001",
                "licensee": "Électro Sfax SARL",
                "issuedAt": "2027-01-01",
                "expiresAt": "2026-01-01",
            })
            .to_string(),
        );
        assert!(matches!(
            validate_bytes(&inverted, today, Some(MACHINE_A)),
            LicenseStatus::Malformed { .. }
        ));
    }

    #[test]
    fn a_signed_payload_missing_required_fields_is_malformed() {
        let bytes = signed(r#"{"licenseId":"PS-2026-0001"}"#);
        assert!(matches!(
            validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_A)),
            LicenseStatus::Malformed { .. }
        ));
    }

    #[test]
    fn unknown_payload_fields_are_ignored_rather_than_rejected() {
        // Forward compatibility: a licence minted for a later build, carrying
        // fields this one has never heard of, must still validate.
        let bytes = signed(
            &serde_json::json!({
                "licenseId": "PS-2026-0001",
                "licensee": "Électro Sfax SARL",
                "issuedAt": "2026-01-01",
                "expiresAt": "2030-01-01",
                "seats": 3,
                "supportTier": "gold",
            })
            .to_string(),
        );
        assert!(matches!(
            validate_bytes(&bytes, d("2026-07-28"), None),
            LicenseStatus::Valid(_)
        ));
    }

    #[test]
    fn an_absent_features_list_defaults_to_empty() {
        let bytes = signed(
            &serde_json::json!({
                "licenseId": "PS-2026-0001",
                "licensee": "Électro Sfax SARL",
                "issuedAt": "2026-01-01",
                "expiresAt": "2030-01-01",
            })
            .to_string(),
        );
        let LicenseStatus::Valid(license) = validate_bytes(&bytes, d("2026-07-28"), None) else {
            panic!("expected Valid");
        };
        assert!(license.features.is_empty());
        assert_eq!(license.machine_id, None);
    }

    // -- expiry -------------------------------------------------------------

    #[test]
    fn expiry_is_inclusive_of_the_expiry_date() {
        // A licence sold "until 31 December" must still work on 31 December.
        let bytes = signed(&payload_json("2026-12-31", None));
        assert!(matches!(
            validate_bytes(&bytes, d("2026-12-31"), None),
            LicenseStatus::Valid(_)
        ));
    }

    #[test]
    fn the_day_after_expiry_reports_expired_with_the_date() {
        let bytes = signed(&payload_json("2026-12-31", None));
        let status = validate_bytes(&bytes, d("2027-01-01"), None);
        let LicenseStatus::Expired {
            license,
            expired_on,
        } = status
        else {
            panic!("expected Expired, got {status:?}");
        };
        assert_eq!(expired_on, d("2026-12-31"));
        // The licence is carried: its signature verified, so the contents are
        // trustworthy and worth showing in the eventual message.
        assert_eq!(license.license_id, "PS-2026-0001");
    }

    // -- machine binding ----------------------------------------------------

    #[test]
    fn a_licence_bound_to_this_machine_validates() {
        let bytes = signed(&payload_json("2030-01-01", Some(MACHINE_A)));
        assert!(matches!(
            validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_A)),
            LicenseStatus::Valid(_)
        ));
    }

    #[test]
    fn a_licence_bound_to_another_machine_is_rejected() {
        // The anti-copy property: the same file on a second machine must not work.
        let bytes = signed(&payload_json("2030-01-01", Some(MACHINE_A)));
        let status = validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_B));
        let LicenseStatus::MachineMismatch { license, local } = status else {
            panic!("expected MachineMismatch, got {status:?}");
        };
        assert_eq!(license.machine_id.as_deref(), Some(MACHINE_A));
        assert_eq!(local.as_deref(), Some(MACHINE_B));
    }

    #[test]
    fn an_unbound_licence_validates_on_any_machine() {
        // Floating licences (demo, support) carry machineId: null.
        let bytes = signed(&payload_json("2030-01-01", None));
        for local in [Some(MACHINE_A), Some(MACHINE_B), None] {
            assert!(matches!(
                validate_bytes(&bytes, d("2026-07-28"), local),
                LicenseStatus::Valid(_)
            ));
        }
    }

    #[test]
    fn binding_is_rejected_when_the_local_fingerprint_is_unavailable() {
        // Fail closed, but keep `local: None` so support can tell "wrong machine"
        // apart from "this machine could not be identified".
        let bytes = signed(&payload_json("2030-01-01", Some(MACHINE_A)));
        let status = validate_bytes(&bytes, d("2026-07-28"), None);
        assert!(matches!(
            status,
            LicenseStatus::MachineMismatch { local: None, .. }
        ));
    }

    #[test]
    fn machine_binding_comparison_ignores_hex_case() {
        // A signer emitting upper-case hex must not lock a customer out.
        let bytes = signed(&payload_json("2030-01-01", Some(&MACHINE_A.to_uppercase())));
        assert!(matches!(
            validate_bytes(&bytes, d("2026-07-28"), Some(MACHINE_A)),
            LicenseStatus::Valid(_)
        ));
    }

    #[test]
    fn a_wrong_machine_is_reported_ahead_of_expiry() {
        // Pins the documented check order. A licence issued for someone else's
        // machine was never valid here, which is the more accurate thing to say.
        let bytes = signed(&payload_json("2026-01-01", Some(MACHINE_A)));
        assert!(matches!(
            validate_bytes(&bytes, d("2027-01-01"), Some(MACHINE_B)),
            LicenseStatus::MachineMismatch { .. }
        ));
    }

    // -- fingerprint derivation ---------------------------------------------

    #[test]
    fn fingerprint_normalization_absorbs_the_per_os_formatting_differences() {
        // /etc/machine-id ends in a newline; the Windows MachineGuid is sometimes
        // brace-wrapped and upper-case. All of these are one machine.
        let canonical = fingerprint_of("abc123");
        for variant in ["abc123\n", " abc123 ", "{abc123}", "{ABC123}", "ABC123\r\n"] {
            assert_eq!(fingerprint_of(variant), canonical, "variant {variant:?}");
        }
        assert_ne!(fingerprint_of("abc124"), canonical);
    }

    #[test]
    fn the_fingerprint_is_a_salted_sha256_in_lower_case_hex() {
        let fingerprint = fingerprint_of("abc123");
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(fingerprint.chars().all(|c| !c.is_ascii_uppercase()));

        // Pins the exact derivation so `docs/license-format.md` and any external
        // signing tool cannot drift from this implementation.
        let mut hasher = Sha256::new();
        hasher.update(MACHINE_ID_SALT);
        hasher.update(b"abc123");
        assert_eq!(fingerprint, hex_lower(&hasher.finalize()));

        // ...and specifically that it is *not* a bare hash of the identifier.
        assert_ne!(fingerprint, hex_lower(&Sha256::digest(b"abc123")));
    }

    #[test]
    fn hex_lower_encodes_every_nibble() {
        assert_eq!(hex_lower(&[0x00, 0x0f, 0xf0, 0xff, 0xa5]), "000ff0ffa5");
        assert_eq!(hex_lower(&[]), "");
    }

    // -- clock guard --------------------------------------------------------

    #[test]
    fn winding_the_clock_back_does_not_revive_an_expired_licence() {
        // The attack the watermark exists to stop: a licence that expired in
        // 2027 validates again if the user sets the system date to 2026.
        let bytes = signed(&payload_json("2027-01-01", None));
        let expired = validate_bytes(&bytes, d("2027-06-01"), None);
        assert!(matches!(expired, LicenseStatus::Expired { .. }));

        // Same licence, clock wound back to before it expired: on its own this
        // now reads Valid...
        let revived = validate_bytes(&bytes, d("2026-06-01"), None);
        assert!(matches!(revived, LicenseStatus::Valid(_)));

        // ...but the install has already seen 2027-06-01, so it is refused.
        assert_eq!(
            apply_clock_guard(revived, d("2026-06-01"), Some(d("2027-06-01"))),
            LicenseStatus::ClockTampered {
                watermark: d("2027-06-01")
            }
        );
    }

    #[test]
    fn the_clock_guard_leaves_verdicts_that_do_not_depend_on_the_date() {
        // A broken signature or a wrong machine is equally wrong whatever the
        // clock says, and reporting that accurately is more useful than
        // masking it behind a clock complaint.
        let past = d("2020-01-01");
        let mark = Some(d("2027-01-01"));

        assert_eq!(
            apply_clock_guard(LicenseStatus::InvalidSignature, past, mark),
            LicenseStatus::InvalidSignature
        );
        assert_eq!(
            apply_clock_guard(LicenseStatus::Missing, past, mark),
            LicenseStatus::Missing
        );
        let malformed = LicenseStatus::Malformed { reason: "x" };
        assert_eq!(apply_clock_guard(malformed.clone(), past, mark), malformed);
    }

    #[test]
    fn the_clock_guard_is_inert_without_a_watermark_or_when_time_moves_forward() {
        let bytes = signed(&payload_json("2030-01-01", None));
        let valid = validate_bytes(&bytes, d("2026-07-28"), None);

        // Fresh install: nothing recorded yet, so nothing to contradict.
        assert_eq!(
            apply_clock_guard(valid.clone(), d("2026-07-28"), None),
            valid
        );
        // Normal operation: today is at or after the mark.
        assert_eq!(
            apply_clock_guard(valid.clone(), d("2026-07-28"), Some(d("2026-07-28"))),
            valid
        );
        assert_eq!(
            apply_clock_guard(valid.clone(), d("2026-07-29"), Some(d("2026-07-28"))),
            valid
        );
    }

    #[test]
    fn the_watermark_only_advances() {
        // A normal launch on an already-recorded day writes nothing, so the
        // common path stays read-only against the settings table.
        assert_eq!(next_watermark(Some(d("2026-07-28")), d("2026-07-28")), None);
        assert_eq!(next_watermark(Some(d("2026-07-29")), d("2026-07-28")), None);
        // First run, and any genuinely later day, record the new high mark.
        assert_eq!(next_watermark(None, d("2026-07-28")), Some(d("2026-07-28")));
        assert_eq!(
            next_watermark(Some(d("2026-07-28")), d("2026-07-29")),
            Some(d("2026-07-29"))
        );
    }

    // -- the IPC projection -------------------------------------------------

    #[test]
    fn the_wire_type_never_carries_the_malformed_reason() {
        // `reason` is parser detail written for the log. The exact leak this
        // projection exists to prevent, mirroring `AppError::Internal`.
        let status = LicenseStatus::Malformed {
            reason: "payload is missing required licence fields",
        };
        let info = status.to_info(None);
        assert_eq!(info.status, "malformed");
        assert_eq!(info.license, None);

        let json = serde_json::to_string(&info).expect("LicenseInfo serializes");
        assert!(
            !json.contains("payload is missing"),
            "leaked reason: {json}"
        );
        assert!(!json.contains("reason"), "leaked reason field: {json}");
    }

    #[test]
    fn the_wire_type_withholds_the_licence_when_the_signature_failed() {
        // Nothing inside an unverified file may be shown as if it were attested.
        for status in [
            LicenseStatus::InvalidSignature,
            LicenseStatus::Missing,
            LicenseStatus::Malformed { reason: "x" },
        ] {
            assert_eq!(status.to_info(None).license, None, "{status:?}");
        }
    }

    #[test]
    fn the_wire_type_reports_dates_as_iso_strings() {
        let bytes = signed(&payload_json("2026-12-31", None));
        let info = validate_bytes(&bytes, d("2027-03-01"), None).to_info(None);

        assert_eq!(info.status, "expired");
        assert_eq!(info.expired_on.as_deref(), Some("2026-12-31"));
        // The licence is carried here: its signature verified.
        assert_eq!(
            info.license.map(|l| l.license_id).as_deref(),
            Some("PS-2026-0001")
        );
    }

    #[test]
    fn every_status_maps_to_a_distinct_stable_tag() {
        // These strings are a wire contract mirrored by the TypeScript union in
        // `src/types/models.ts`; a silent rename would break the frontend.
        let license = License {
            license_id: "id".into(),
            licensee: "who".into(),
            issued_at: "2026-01-01".into(),
            expires_at: "2030-01-01".into(),
            machine_id: None,
            features: vec![],
        };
        let tags: Vec<&str> = [
            LicenseStatus::Valid(license.clone()),
            LicenseStatus::Expired {
                license: license.clone(),
                expired_on: d("2026-01-01"),
            },
            LicenseStatus::MachineMismatch {
                license,
                local: None,
            },
            LicenseStatus::InvalidSignature,
            LicenseStatus::Malformed { reason: "x" },
            LicenseStatus::Missing,
            LicenseStatus::ClockTampered {
                watermark: d("2026-01-01"),
            },
        ]
        .iter()
        .map(|s| s.tag())
        .collect();

        assert_eq!(
            tags,
            [
                "valid",
                "expired",
                "machineMismatch",
                "invalidSignature",
                "malformed",
                "missing",
                "clockTampered"
            ]
        );
    }

    #[test]
    fn only_valid_counts_as_licensed() {
        let license = License {
            license_id: "id".into(),
            licensee: "who".into(),
            issued_at: "2026-01-01".into(),
            expires_at: "2030-01-01".into(),
            machine_id: None,
            features: vec![],
        };
        assert!(LicenseStatus::Valid(license.clone()).is_valid());
        // Notably `ClockTampered` is *not* licensed — the whole point is that a
        // tampered clock must not buy access.
        for status in [
            LicenseStatus::Expired {
                license: license.clone(),
                expired_on: d("2026-01-01"),
            },
            LicenseStatus::MachineMismatch {
                license,
                local: None,
            },
            LicenseStatus::InvalidSignature,
            LicenseStatus::Malformed { reason: "x" },
            LicenseStatus::Missing,
            LicenseStatus::ClockTampered {
                watermark: d("2026-01-01"),
            },
        ] {
            assert!(!status.is_valid(), "{status:?} must not be licensed");
        }
    }

    #[test]
    fn license_state_round_trips_and_reports_validity() {
        let state = LicenseState::new(LicenseStatus::Missing);
        assert!(!state.is_valid());
        assert_eq!(state.get(), LicenseStatus::Missing);

        let bytes = signed(&payload_json("2030-01-01", None));
        state.set(validate_bytes(&bytes, d("2026-07-28"), None));
        assert!(state.is_valid());
    }

    // -- documentation parity -----------------------------------------------

    #[test]
    fn the_worked_example_from_the_format_docs_validates() {
        // `docs/license-format.md` §8 publishes this exact file, produced by a
        // Python signer against the development seed. Pinning it here means the
        // documented recipe — prefix, base64url alphabet, salt, fingerprint
        // derivation — is verified against this implementation on every run,
        // rather than being prose that quietly stops being true.
        //
        // It goes through the embedded key, so it only means anything on a build
        // that embeds the development one. The `certificate-generation` project
        // holds the same vector checked against a fixed key, where no environment
        // variable can skip it.
        require_dev_key!("the_worked_example_from_the_format_docs_validates");

        let example = br#"{
  "version": 1,
  "payload": "eyJsaWNlbnNlSWQiOiJQUy0yMDI2LTAwMDEiLCJsaWNlbnNlZSI6IsOJbGVjdHJvIFNmYXggU0FSTCIsImlzc3VlZEF0IjoiMjAyNi0wMS0xNSIsImV4cGlyZXNBdCI6IjIwMzAtMDEtMTUiLCJtYWNoaW5lSWQiOiI5ODgzYjlmMGNlNmFlYmY1MjkyZjc3NTY3NDQ5YTRhZDZiZGI1YmY2YjdiYjQ5ZWNjMWY5NmU5ZDVjNTQzMzE2IiwiZmVhdHVyZXMiOlsiKiJdfQ",
  "signature": "LIq9yiCuqIH2v-3gj0tgUuotOFed6eWoC3mu0IywVijM4jvFb118ZWTKoup9b44TO921IqXEgJqkhFvz_14WDg"
}"#;

        // The example is bound to the fingerprint of the raw identifier "abc123",
        // so this also pins the Python and Rust fingerprint derivations together.
        let machine = fingerprint_of("abc123");
        let status = validate_bytes(example, d("2026-07-28"), Some(&machine));

        let LicenseStatus::Valid(license) = status else {
            panic!("the documented example must validate, got {status:?}");
        };
        assert_eq!(license.license_id, "PS-2026-0001");
        assert_eq!(license.expires_at, "2030-01-15");
        assert_eq!(license.machine_id.as_deref(), Some(machine.as_str()));

        // ...and on any other machine it is a mismatch, which is the whole point
        // of the `machineId` in the documented payload.
        assert!(matches!(
            validate_bytes(example, d("2026-07-28"), Some(MACHINE_B)),
            LicenseStatus::MachineMismatch { .. }
        ));
    }

    // -- filesystem ---------------------------------------------------------

    #[test]
    fn a_missing_licence_file_is_missing_not_an_error() {
        // The normal state of a fresh install, so it must not surface as a fault.
        let path = temp_path("absent");
        assert_eq!(
            validate_file(&path, d("2026-07-28"), None).expect("absence is not an error"),
            LicenseStatus::Missing
        );
    }

    #[test]
    fn a_licence_file_on_disk_validates() {
        let path = temp_path("valid");
        std::fs::write(&path, valid_bytes()).expect("temp write");

        let status = validate_file(&path, d("2026-07-28"), None).expect("readable file");
        assert!(matches!(status, LicenseStatus::Valid(_)));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_oversized_file_is_rejected_without_being_read() {
        // The cap is checked against the metadata, so a hostile file never
        // reaches memory. Written just over the limit.
        let path = temp_path("oversized");
        std::fs::write(&path, vec![b'{'; (LICENSE_MAX_BYTES + 1) as usize]).expect("temp write");

        let status = validate_file(&path, d("2026-07-28"), None).expect("readable file");
        assert!(matches!(status, LicenseStatus::Malformed { .. }));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_directory_at_the_licence_path_is_malformed_not_a_crash() {
        let path = temp_path("directory");
        std::fs::create_dir(&path).expect("temp dir");

        let status = validate_file(&path, d("2026-07-28"), None).expect("stat-able path");
        assert!(matches!(status, LicenseStatus::Malformed { .. }));

        let _ = std::fs::remove_dir(&path);
    }
}
