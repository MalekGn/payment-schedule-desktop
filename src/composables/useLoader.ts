// Shared load-with-error-handling wrapper for the list and dashboard views.
//
// Every view used to do the same three unguarded lines:
//
//   loading.value = true;
//   rows.value = await api.listSomething();
//   loading.value = false;
//
// A rejection skipped the third line entirely, so the page sat on its skeleton
// spinner forever with nothing shown and nothing logged — the failure was
// indistinguishable from a slow query. This centralises the try/catch/finally,
// the localized message, and the retry affordance so all seven views behave the
// same way.

import { ref, type Ref } from "vue";
import { useI18n } from "vue-i18n";

import { toUserMessage } from "@/lib/errors";
import { useUiStore } from "@/stores/ui";

export interface Loader {
  /** True while `run` is in flight. Always cleared, including on failure. */
  loading: Ref<boolean>;
  /** Localized message for the last failure, or `""` when the load succeeded. */
  error: Ref<string>;
  /** Run (or re-run) the load. Never rejects. */
  run: () => Promise<void>;
}

/**
 * Wrap a view's data fetch.
 *
 * Must be called from `setup()` — it uses `useI18n` and the ui store.
 * `loading` starts `true` so the first paint shows the skeleton rather than an
 * empty state that is about to be replaced.
 */
export function useLoader(load: () => Promise<void>): Loader {
  const loading = ref(true);
  const error = ref("");
  const { t } = useI18n();
  const ui = useUiStore();

  async function run(): Promise<void> {
    loading.value = true;
    error.value = "";
    try {
      await load();
    } catch (e) {
      // `toUserMessage` logs the original; the user sees only the localized
      // sentence, both inline (with a retry button) and as a toast.
      error.value = toUserMessage(e, t);
      ui.notify(error.value, "error");
    } finally {
      loading.value = false;
    }
  }

  return { loading, error, run };
}
