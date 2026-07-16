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
const settings = useSettingsStore(pinia);
settings.load().finally(() => app.mount("#app"));
