// Shared "save this CSV" plumbing for the export buttons.
//
// # Why this exists
//
// Both export buttons used to build a `Blob`, point an `<a download>` at it and
// click it. That is a browser mechanism: it works in the dev preview and in the
// E2E suite, and does nothing whatsoever inside the Tauri WebView, which has no
// download manager. The buttons were silently inert in the shipped desktop app —
// no file, no error, no toast. Nothing caught it because the only automated
// coverage runs in real Chromium, where the old path was fine.
//
// The gateway now owns that split (`api.saveCsv`), and this owns what the user
// sees about it: a confirmation when a file was written, a localized message
// when the write failed, and deliberate silence when they simply cancelled the
// save dialog.

import { useI18n } from "vue-i18n";

import { api } from "@/api";
import { toUserMessage } from "@/lib/errors";
import { useUiStore } from "@/stores/ui";

export interface CsvExport {
  /** Save `contents` as `filename`. Never rejects; reports through toasts. */
  saveCsv: (filename: string, contents: string) => Promise<void>;
}

/** Must be called from `setup()` — it uses `useI18n` and the ui store. */
export function useCsvExport(): CsvExport {
  const { t } = useI18n();
  const ui = useUiStore();

  async function saveCsv(filename: string, contents: string): Promise<void> {
    try {
      const written = await api.saveCsv(filename, contents);
      // Dismissing the save dialog is a decision, not a failure. Toasting it
      // would tell the user something went wrong when nothing did.
      if (written) ui.notify(t("common.exported"));
    } catch (e) {
      // `toUserMessage` logs the original and returns only the localized
      // sentence, so a filesystem path never reaches the toast.
      ui.notify(toUserMessage(e, t), "error");
    }
  }

  return { saveCsv };
}
