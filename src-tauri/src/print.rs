//! Naming the file the GTK print dialog offers to save.
//!
//! # Why this exists
//!
//! A printed document suggests its own file name through `document.title`, which
//! is what WebView2 and Chromium read. **WebKitGTK does not.** On Linux the
//! output name comes from the GTK print job's `output-basename` setting, and
//! when nothing sets it GTK falls back to its own hardcoded `output` — so every
//! document the app printed was offered as `output.pdf`, whatever it was.
//!
//! Nothing reachable from JavaScript can move that: it is a print *setting*, not
//! a document or window property. So this reaches the native `WebKitWebView`
//! through Tauri's `with_webview` escape hatch, listens for the print signal,
//! and stamps the basename onto the operation before the dialog opens.
//!
//! The handler returns `false` — "not handled" — deliberately. WebKit then runs
//! its own dialog on the same `WebKitPrintOperation`, which by then carries our
//! settings. Running the dialog here instead would mean owning the parent
//! window, the response handling and the error paths, to arrive at the same
//! place.
//!
//! The basename comes from the webview's title, which tracks `document.title`.
//! That means this needs no state, no command and no coordination with the
//! renderer: the names the frontend already sets (`src/lib/filename.ts`) are the
//! names that come out, and the two cannot drift apart.
//!
//! # Stability
//!
//! `with_webview` hands out a platform handle that Tauri explicitly excludes
//! from its semver guarantees. A Tauri upgrade can therefore break this file
//! without breaking anything else, so it is deliberately the only place in the
//! tree that touches `webkit2gtk`. `webkit2gtk` and `gtk` are declared for Linux
//! only, at the versions `wry` already pulls in.

/// GTK print-settings key for the "Print to File" destination's base name.
/// Spelled out rather than imported: `gtk::PRINT_SETTINGS_OUTPUT_BASENAME` is
/// not re-exported by the `gtk` crate, and the string is part of GTK's stable
/// public API.
#[cfg(target_os = "linux")]
const OUTPUT_BASENAME: &str = "output-basename";

/// Reduce a webview title to something safe to hand a print dialog.
///
/// The frontend already sends an ASCII slug, so in practice this changes
/// nothing. It exists because the title is whatever the page put there: a
/// future page — or a page that failed to set one — must not be able to put a
/// path separator into a file name the dialog then resolves.
#[cfg(target_os = "linux")]
fn sanitize(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for c in title.chars() {
        if c.is_control() || matches!(c, '/' | '\\' | ':' | '\0') {
            // Runs collapse: `C:\\Windows` maps two characters onto one
            // separator, and `C--Windows` reads like a defect in a file name.
            if !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c);
        }
    }
    // Leading dots would hide the saved file; trailing ones give it a second
    // extension.
    out.trim_matches(['-', ' ', '.']).to_string()
}

/// Make the GTK print dialog name its output after the document.
///
/// A no-op everywhere except Linux, where the whole problem lives: WebView2 and
/// WKWebView both derive the name from `document.title` on their own.
#[cfg(target_os = "linux")]
pub fn name_printed_documents(window: &tauri::WebviewWindow) {
    use webkit2gtk::{PrintOperationExt, WebViewExt};

    let result = window.with_webview(|platform| {
        platform.inner().connect_print(|webview, operation| {
            let basename = webview.title().map(|t| sanitize(&t)).unwrap_or_default();
            if basename.is_empty() {
                // Nothing better to offer than GTK's default; leave it alone
                // rather than write an empty name into the dialog.
                return false;
            }

            // Carry any settings WebKit already prepared, so this adds a name
            // rather than resetting the user's paper size or duplex choice.
            let settings = operation
                .print_settings()
                .unwrap_or_else(gtk::PrintSettings::new);
            settings.set(OUTPUT_BASENAME, Some(basename.as_str()));
            operation.set_print_settings(&settings);

            // `false` = not handled. WebKit runs its own dialog on this same
            // operation, which now carries the name.
            false
        });
    });

    if let Err(e) = result {
        // Printing still works without this; only the suggested file name is
        // lost. That is worth a log line, never a failed launch.
        log::warn!("could not install the print-name handler: {e}");
    }
}

#[cfg(not(target_os = "linux"))]
pub fn name_printed_documents(_window: &tauri::WebviewWindow) {}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// The frontend already sends an ASCII slug, so what matters here is the
    /// case it does not cover: a title this module did not author. The webview
    /// title is whatever the page put there, and it lands in a file name the
    /// dialog resolves.
    #[test]
    fn a_title_cannot_smuggle_a_path_into_the_file_name() {
        assert_eq!(sanitize("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize("C:\\Windows\\System32"), "C-Windows-System32");
        assert_eq!(sanitize("name\nwith\tcontrols"), "name-with-controls");
        assert_eq!(sanitize("a:b"), "a-b");
    }

    /// Leading and trailing dots would make the saved file hidden, or give it a
    /// second extension.
    #[test]
    fn edges_are_trimmed() {
        assert_eq!(sanitize("  .hidden.  "), "hidden");
        assert_eq!(sanitize("-Recu-"), "Recu");
    }

    /// The names the frontend actually sends must survive untouched — this must
    /// not quietly rewrite what `src/lib/filename.ts` carefully built.
    #[test]
    fn the_names_the_app_sends_pass_through_unchanged() {
        for name in [
            "Echeancier-A-000001-Mohamed-Trabelsi",
            "Recu-A-000001-T2-2026-08-20",
            "Releve-client-12-2026-08-20",
            "paymentSchedule",
        ] {
            assert_eq!(sanitize(name), name);
        }
    }

    /// An empty result is the signal to leave GTK's own default alone rather
    /// than write a nameless file.
    #[test]
    fn an_unusable_title_reduces_to_nothing() {
        assert_eq!(sanitize(""), "");
        assert_eq!(sanitize("///"), "");
        assert_eq!(sanitize("..."), "");
    }
}
