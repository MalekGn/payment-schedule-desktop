import { createApp } from "vue";
import { createPinia } from "pinia";
import "./style.css";
import App from "./App.vue";
import { i18n } from "@/i18n";
import { router } from "@/router";
import { useSettingsStore } from "@/stores/settings";

const app = createApp(App);
const pinia = createPinia();

app.use(pinia);
app.use(i18n);
app.use(router);

// Load persisted settings (language, currency, date format, logo) and apply the
// locale/direction before the first paint, then mount.
//
// Mounting is unconditional: a settings failure means the app falls back to its
// defaults, which is far better than a blank window. `.finally()` alone left the
// rejection unhandled, so the reason never surfaced anywhere — hence the
// explicit catch.
const settings = useSettingsStore(pinia);
settings
  .load()
  .catch((e: unknown) => {
    console.error("failed to load settings; starting with defaults:", e);
  })
  .finally(() => app.mount("#app"));
