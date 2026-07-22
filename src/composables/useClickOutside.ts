// Calls `handler` when a click lands outside `elRef`. Used to dismiss popovers
// and menus (e.g. the header language menu, date-range filter popup).
import { onBeforeUnmount, onMounted, type Ref } from "vue";

export function useClickOutside(elRef: Ref<HTMLElement | null>, handler: () => void) {
  function onDocClick(e: MouseEvent) {
    const target = e.target as Element | null;
    // Ignore clicks inside a teleported DatePicker popup (rendered at <body>):
    // a nested picker must not dismiss the popover/menu that contains it.
    if (target?.closest?.("[data-datepicker-pop]")) return;
    const el = elRef.value;
    if (el && !el.contains(e.target as Node)) handler();
  }
  onMounted(() => document.addEventListener("click", onDocClick));
  onBeforeUnmount(() => document.removeEventListener("click", onDocClick));
}
