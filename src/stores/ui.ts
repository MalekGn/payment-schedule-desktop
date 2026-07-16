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

  function notify(message: string, kind: Toast["kind"] = "success") {
    const id = ++seq;
    toasts.value.push({ id, message, kind });
    setTimeout(() => dismiss(id), 3500);
  }

  function dismiss(id: number) {
    toasts.value = toasts.value.filter((t) => t.id !== id);
  }

  return { sidebarOpen, pageTitle, toasts, toggleSidebar, notify, dismiss };
});
