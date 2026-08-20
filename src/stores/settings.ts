import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api, isTauri } from "@/api";
import { applyLocale, isSupportedLocale, resolveOsLocale, type AppLocale } from "@/i18n";
import { dayDiff, todayIso } from "@/lib/finance";
import type { Settings, SettingsPatch } from "@/types/models";

const DEFAULTS: Settings = {
  language: "fr",
  currencyCode: "TND",
  dateFormat: "dd/MM/yyyy",
  logoPath: null,
  shopName: "",
  shopInfo: "",
  alertSoonDays: 7,
  languageIsDefault: true,
  lastBackupAt: null,
  lastAutoBackupAt: null,
  autoBackupEnabled: true,
  autoBackupFrequency: "daily",
  autoBackupTime: "17:00",
};

export const DATE_FORMATS = ["dd/MM/yyyy", "MM/dd/yyyy", "yyyy-MM-dd", "dd-MM-yyyy"];
export const CURRENCIES = ["TND", "EUR", "USD", "FCFA", "DZD", "MAD"];
/** Allowed bounds for the "due soon" alert window (days). */
export const ALERT_SOON_DAYS_MIN = 1;
export const ALERT_SOON_DAYS_MAX = 90;
/**
 * Age (in days) at which the Settings page starts nudging about the backup.
 *
 * Backups are manual and nothing else in the app ever mentions them, so an
 * install that is never nudged is an install that is never backed up. A month
 * is long enough not to nag and short enough that the loss window stays small.
 */
export const BACKUP_STALE_DAYS = 30;
/** Cadences the schedule offers. Mirrors `BACKUP_FREQUENCIES` in `db.rs`. */
export const BACKUP_FREQUENCIES = ["daily", "weekly", "monthly"];

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
  const shopInfo = computed(() => settings.value.shopInfo);
  const alertSoonDays = computed(() => settings.value.alertSoonDays);
  const lastBackupAt = computed(() => settings.value.lastBackupAt);
  const lastAutoBackupAt = computed(() => settings.value.lastAutoBackupAt);
  const autoBackupEnabled = computed(() => settings.value.autoBackupEnabled);
  const autoBackupFrequency = computed(() => settings.value.autoBackupFrequency);
  const autoBackupTime = computed(() => settings.value.autoBackupTime);

  /**
   * Whether to nudge the user about backing up: never done, or done long
   * enough ago that the data since then is worth more than the reminder costs.
   */
  const backupIsStale = computed(() => {
    // Reads `lastBackupAt` only, never `lastAutoBackupAt`. The automatic
    // snapshots live in the app-data directory beside the database, so one disk
    // failure, one theft or one ransomware run takes both. This nudge asks for a
    // copy that leaves the machine; letting an automatic one silence it would
    // tell the user they are covered when they are not.
    const last = settings.value.lastBackupAt;
    if (!last) return true;
    return dayDiff(todayIso(), last) >= BACKUP_STALE_DAYS;
  });

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

  /**
   * Snapshot the database to `dest`, then adopt the settings the command
   * returns — they carry the new `lastBackupAt`, which is what clears the
   * staleness nudge without a second round trip.
   */
  async function backupDatabase(dest: string) {
    settings.value = await api.backupDatabase(dest);
  }

  return {
    settings,
    loaded,
    language,
    currencyCode,
    dateFormat,
    logoPath,
    shopName,
    shopInfo,
    alertSoonDays,
    lastBackupAt,
    lastAutoBackupAt,
    autoBackupEnabled,
    autoBackupFrequency,
    autoBackupTime,
    backupIsStale,
    load,
    update,
    setLanguage,
    setLogoFromPath,
    clearLogo,
    backupDatabase,
  };
});
