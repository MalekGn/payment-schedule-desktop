import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api, isTauri } from "@/api";
import { applyLocale, isSupportedLocale, resolveOsLocale, type AppLocale } from "@/i18n";
import type { Settings, SettingsPatch } from "@/types/models";

const DEFAULTS: Settings = {
  language: "fr",
  currencyCode: "TND",
  dateFormat: "dd/MM/yyyy",
  logoPath: null,
  shopName: "",
  shopInfo: "",
  languageIsDefault: true,
};

export const DATE_FORMATS = ["dd/MM/yyyy", "MM/dd/yyyy", "yyyy-MM-dd", "dd-MM-yyyy"];
export const CURRENCIES = ["TND", "EUR", "USD", "FCFA", "DZD", "MAD"];

/** Detect the OS locale (Tauri OS plugin when available, else the browser). */
async function detectOsLocale(): Promise<AppLocale> {
  try {
    if (isTauri()) {
      const { locale } = await import("@tauri-apps/plugin-os");
      return resolveOsLocale(await locale());
    }
  } catch {
    /* fall through to browser */
  }
  return resolveOsLocale(typeof navigator !== "undefined" ? navigator.language : "fr");
}

export const useSettingsStore = defineStore("settings", () => {
  const settings = ref<Settings>({ ...DEFAULTS });
  const loaded = ref(false);

  const language = computed<AppLocale>(() =>
    isSupportedLocale(settings.value.language) ? settings.value.language : "fr",
  );
  const currencyCode = computed(() => settings.value.currencyCode);
  const dateFormat = computed(() => settings.value.dateFormat);
  const logoPath = computed(() => settings.value.logoPath);
  const shopName = computed(() => settings.value.shopName);

  async function load() {
    settings.value = await api.getSettings();
    // Fresh install: derive language from the OS locale until the user picks one.
    if (settings.value.languageIsDefault) {
      const os = await detectOsLocale();
      settings.value.language = os;
    }
    applyLocale(language.value);
    loaded.value = true;
  }

  async function update(patch: SettingsPatch) {
    settings.value = await api.updateSettings(patch);
    applyLocale(language.value);
  }

  async function setLanguage(lang: AppLocale) {
    await update({ language: lang });
  }

  async function setLogoFromPath(path: string) {
    settings.value = await api.setLogo(path);
  }

  async function clearLogo() {
    settings.value = await api.clearLogo();
  }

  return {
    settings,
    loaded,
    language,
    currencyCode,
    dateFormat,
    logoPath,
    shopName,
    load,
    update,
    setLanguage,
    setLogoFromPath,
    clearLogo,
  };
});
