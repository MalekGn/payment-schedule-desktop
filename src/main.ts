import { createApp } from "vue";
import { createPinia } from "pinia";
import "./style.css";
import App from "./App.vue";
import { i18n } from "@/i18n";
import { router } from "@/router";
import { useLicenseStore } from "@/stores/license";
import { useSettingsStore } from "@/stores/settings";

const app = createApp(App);
const pinia = createPinia();

app.use(pinia);
app.use(i18n);
app.use(router);

// Load persisted settings (language, currency, date format, logo) and the
// licence verdict, apply the locale/direction, then mount.
//
// Both run before the first paint so the shell never flashes the wrong language
// or briefly shows a licensed screen it is about to replace.
//
// Mounting is unconditional: a settings failure means the app falls back to its
// defaults, which is far better than a blank window. `.finally()` alone left the
// rejection unhandled, so the reason never surfaced anywhere — hence the
// explicit catch. The licence store swallows its own failure the same way, and
// fails closed.
//
// Watching starts after the first verdict has landed, so a push cannot be
// overwritten by an older `load()` still in flight.
const settings = useSettingsStore(pinia);
const license = useLicenseStore(pinia);
Promise.all([
  settings.load().catch((e: unknown) => {
    console.error("failed to load settings; starting with defaults:", e);
  }),
  license.load(),
]).finally(() => {
  // Not awaited: the licence is re-evaluated in Rust while the app runs, and
  // this is how the window hears about it — but a listener that fails to
  // register must cost a stale screen, never a window that never opens.
  void license.watch();
  app.mount("#app");
});
