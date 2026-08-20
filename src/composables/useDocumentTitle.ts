// Naming the document, so the print dialog can name the file.
//
// # Why this exists
//
// The print engine derives the file name it suggests from the document title.
// This app set one exactly once, in `index.html`, and never changed it — so
// every printed document was offered under the engine's own fallback name,
// `output.pdf`, whether it was a schedule, a receipt or a statement.
//
// Two titles are set rather than one. `document.title` is the standard
// mechanism and is what Chromium and WebView2 use. On Linux the output name
// comes from the GTK print job, which commonly takes its name from the parent
// window instead — hence the native window title as well.

import { onBeforeUnmount, watch, type Ref } from "vue";

import { api } from "@/api";

/** What `index.html` ships, and what the app is called outside a document. */
const APP_TITLE = "paymentSchedule";

/**
 * Keep the document and window titles in step with `title`.
 *
 * `null` means "nothing to show yet" and leaves the current title alone, so a
 * half-loaded or failed document never renames the window.
 *
 * Both titles are restored when the component unmounts. That is the part worth
 * not losing: without it, navigating out of a print route leaves the whole
 * application window named after a client's receipt, which nobody notices until
 * they look at their taskbar.
 */
export function useDocumentTitle(title: Ref<string | null>): void {
  const original = typeof document === "undefined" ? APP_TITLE : document.title;

  function apply(next: string): void {
    if (typeof document !== "undefined") document.title = next;
    // Fire-and-forget: a window that would not accept a rename is not a reason
    // to fail the page the user is looking at. `toUserMessage` is not involved
    // because there is nothing here a shopkeeper could act on.
    void api.setWindowTitle(next).catch((e) => {
      console.error("could not rename the window:", e);
    });
  }

  watch(
    title,
    (next) => {
      if (next) apply(next);
    },
    { immediate: true },
  );

  onBeforeUnmount(() => apply(original));
}
