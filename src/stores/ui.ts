import { defineStore } from "pinia";
import { ref } from "vue";

export interface Toast {
  id: number;
  message: string;
  kind: "success" | "error" | "info";
}

export const useUiStore = defineStore("ui", () => {
  const sidebarOpen = ref(true);
  const toasts = ref<Toast[]>([]);
  // Optional header-title override for dynamic pages (e.g. a client's name).
  const pageTitle = ref<string | null>(null);
  let seq = 0;

  function toggleSidebar() {
    sidebarOpen.value = !sidebarOpen.value;
  }

  /** Most toasts kept at once. Errors persist, so the stack needs a ceiling. */
  const MAX_TOASTS = 4;

  function notify(message: string, kind: Toast["kind"] = "success") {
    const id = ++seq;
    toasts.value.push({ id, message, kind });
    // Oldest out first, so a repeatedly failing action cannot grow the list
    // without bound now that errors no longer expire on their own.
    if (toasts.value.length > MAX_TOASTS) {
      toasts.value = toasts.value.slice(-MAX_TOASTS);
    }
    // Errors wait to be dismissed. 3.5s is not long enough to read a sentence —
    // less so in a second language — and an error the user missed is one they
    // are about to walk into again. Success and info stay transient.
    if (kind !== "error") {
      setTimeout(() => dismiss(id), 3500);
    }
  }

  function dismiss(id: number) {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }

  return { sidebarOpen, pageTitle, toasts, toggleSidebar, notify, dismiss };
});
