//! Build script.
//!
//! Besides the usual Tauri codegen, this decides where the licence public key
//! comes from. `license.rs` reads it with `env!`, so a build with no
//! `PAYMENT_SCHEDULE_LICENSE_PUBKEY` does not compile — something has to supply
//! one for everyday development.
//!
//! Debug builds get the published development key. Release builds get nothing:
//! shipping a binary that trusts a keypair whose secret half is in the project's
//! own documentation would make the whole licence check ornamental, and the
//! build machine is the only place that mistake is cheap to catch.

/// The development public key, matching the seed published in
/// `docs/license-format.md` §7. Debug builds only — never a release default.
///
/// `src/license.rs` carries the same value in order to recognise and warn about
/// it; the test `the_documented_dev_seed_matches_the_embedded_public_key` proves
/// the two stay in step.
const DEV_PUBLIC_KEY: &str = "vA58s7GMDPCW-FnoVy7jDxJQWShUznnJM2aFPT5TVsc";

const KEY_VAR: &str = "PAYMENT_SCHEDULE_LICENSE_PUBKEY";

fn main() {
    // Without this, changing the variable would not trigger a rebuild and the
    // binary would silently keep the key compiled in last time.
    println!("cargo:rerun-if-env-changed={KEY_VAR}");

    // Debug builds *always* use the development key, even when the variable is
    // set. That is deliberate rather than merely convenient.
    //
    // The README tells you to `export PAYMENT_SCHEDULE_LICENSE_PUBKEY` for a
    // release build, so the natural next step — running `cargo test` in the same
    // terminal — used to fail seventeen licence tests. Every one of them signs
    // with the development seed and validates through the compiled-in key, so a
    // foreign key breaks them all for a reason that is not a defect.
    //
    // Nothing is lost by overriding: a debug build carrying a production public
    // key is useless anyway, because minting a licence it would accept needs the
    // production *seed*, which is not in this repository. Release builds are
    // untouched and still refuse to compile without a key.
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        if std::env::var_os(KEY_VAR).is_some() {
            println!(
                "cargo:warning={KEY_VAR} is set but debug builds always use the \
                 development licence key; build with --release to use yours"
            );
        }
        println!("cargo:rustc-env={KEY_VAR}={DEV_PUBLIC_KEY}");
    }

    tauri_build::build()
}
