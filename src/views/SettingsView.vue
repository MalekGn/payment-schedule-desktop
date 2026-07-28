<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import {
  useSettingsStore,
  DATE_FORMATS,
  CURRENCIES,
  ALERT_SOON_DAYS_MIN,
  ALERT_SOON_DAYS_MAX,
} from "@/stores/settings";
import { useUiStore } from "@/stores/ui";
import { SUPPORTED_LOCALES, type AppLocale } from "@/i18n";
import { resolveLogoSrc } from "@/lib/assets";
import { formatDatePattern } from "@/composables/useFormat";
import { api, isTauri } from "@/api";
import { toUserMessage } from "@/lib/errors";
import { todayIso } from "@/lib/finance";

const { t } = useI18n();
const settings = useSettingsStore();
const ui = useUiStore();

const shopName = ref(settings.settings.shopName);
const shopInfo = ref(settings.settings.shopInfo);
const alertSoonDays = ref(settings.settings.alertSoonDays);
const fileInput = ref<HTMLInputElement | null>(null);

/**
 * Run a settings mutation and report the outcome honestly.
 *
 * Every handler on this page used to call `ui.notify(t("settings.saved"))`
 * unconditionally, so a rejected write still told the user their change had
 * been persisted.
 */
async function save(mutate: () => Promise<void>, successKey = "settings.saved") {
  try {
    await mutate();
    ui.notify(t(successKey));
  } catch (e) {
    ui.notify(toUserMessage(e, t), "error");
  }
}

const LANGUAGE_LABELS: Record<AppLocale, string> = {
  fr: "Français",
  en: "English",
  ar: "العربية",
};

async function onLanguage(e: Event) {
  const lang = (e.target as HTMLSelectElement).value as AppLocale;
  await save(() => settings.setLanguage(lang));
}

async function onCurrency(e: Event) {
  const currencyCode = (e.target as HTMLSelectElement).value;
  await save(() => settings.update({ currencyCode }));
}

async function onDateFormat(e: Event) {
  const dateFormat = (e.target as HTMLSelectElement).value;
  await save(() => settings.update({ dateFormat }));
}

async function onAlertSoonDays() {
  // Clamp to the shared bounds; fall back to the current value on empty/NaN.
  const raw = Math.round(Number(alertSoonDays.value));
  const value = Number.isFinite(raw)
    ? Math.min(ALERT_SOON_DAYS_MAX, Math.max(ALERT_SOON_DAYS_MIN, raw))
    : settings.alertSoonDays;
  alertSoonDays.value = value;
  await save(() => settings.update({ alertSoonDays: value }));
}

async function saveShop() {
  await save(() => settings.update({ shopName: shopName.value, shopInfo: shopInfo.value }));
}

async function pickLogo() {
  if (isTauri()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
    });
    if (typeof selected === "string") {
      await save(() => settings.setLogoFromPath(selected));
    }
  } else {
    fileInput.value?.click();
  }
}

function onFileChosen(e: Event) {
  const file = (e.target as HTMLInputElement).files?.[0];
  if (!file) return;
  const reader = new FileReader();
  reader.onload = async () => {
    await save(() => settings.setLogoFromPath(String(reader.result)));
  };
  reader.readAsDataURL(file);
}

async function removeLogo() {
  await save(() => settings.clearLogo());
}

/**
 * Write a snapshot of the database to a location the user picks.
 *
 * The only recovery path in the app: deleting a client cascades through their
 * purchases, installments and payments and cannot be undone.
 */
async function backupDatabase() {
  if (!isTauri()) return;
  const { save: saveDialog } = await import("@tauri-apps/plugin-dialog");
  const dest = await saveDialog({
    defaultPath: `payment-schedule-${todayIso()}.db`,
    filters: [{ name: "SQLite", extensions: ["db"] }],
  });
  if (typeof dest !== "string") return;
  await save(() => api.backupDatabase(dest), "settings.backupDone");
}

function dateSample(pattern: string): string {
  return formatDatePattern(todayIso(), pattern);
}
</script>

<template>
  <div class="settings">
    <section class="card set-card">
      <div class="card-header">
        <h2>{{ t("settings.general") }}</h2>
      </div>
      <div class="set-body">
        <div class="field">
          <label for="set-lang">{{ t("settings.language") }}</label>
          <select id="set-lang" class="select" :value="settings.language" @change="onLanguage">
            <option v-for="l in SUPPORTED_LOCALES" :key="l" :value="l">
              {{ LANGUAGE_LABELS[l] }}
            </option>
          </select>
          <span class="hint">{{ t("settings.languageHint") }}</span>
        </div>

        <div class="field">
          <label for="set-cur">{{ t("settings.currency") }}</label>
          <select id="set-cur" class="select" :value="settings.currencyCode" @change="onCurrency">
            <option v-for="c in CURRENCIES" :key="c" :value="c">{{ c }}</option>
          </select>
          <span class="hint">{{ t("settings.currencyHint") }}</span>
        </div>

        <div class="field">
          <label for="set-date">{{ t("settings.dateFormat") }}</label>
          <select id="set-date" class="select" :value="settings.dateFormat" @change="onDateFormat">
            <option v-for="f in DATE_FORMATS" :key="f" :value="f">
              {{ f }} — {{ dateSample(f) }}
            </option>
          </select>
        </div>

        <div class="field">
          <label for="set-alert">{{ t("settings.alertSoonDays") }}</label>
          <input
            id="set-alert"
            v-model="alertSoonDays"
            class="input"
            type="number"
            :min="ALERT_SOON_DAYS_MIN"
            :max="ALERT_SOON_DAYS_MAX"
            step="1"
            @change="onAlertSoonDays"
          />
          <span class="hint">{{ t("settings.alertSoonDaysHint") }}</span>
        </div>
      </div>
    </section>

    <section class="card set-card">
      <div class="card-header">
        <h2>{{ t("settings.shop") }}</h2>
      </div>
      <div class="set-body">
        <div class="logo-field">
          <span class="field-label">{{ t("settings.logo") }}</span>
          <div class="logo-row">
            <div class="logo-preview">
              <img v-if="settings.logoPath" :src="resolveLogoSrc(settings.logoPath) ?? ''" alt="" />
              <AppIcon v-else name="washer" :size="30" :stroke-width="1.6" />
            </div>
            <div class="logo-actions">
              <button class="btn btn--ghost btn--sm" type="button" @click="pickLogo">
                <AppIcon name="upload" :size="16" /> {{ t("settings.uploadLogo") }}
              </button>
              <button
                v-if="settings.logoPath"
                class="btn btn--ghost btn--sm"
                type="button"
                @click="removeLogo"
              >
                <AppIcon name="trash" :size="16" /> {{ t("settings.removeLogo") }}
              </button>
              <input
                ref="fileInput"
                type="file"
                accept="image/*"
                class="hidden-input"
                @change="onFileChosen"
              />
            </div>
          </div>
          <span class="hint">{{ t("settings.logoHint") }}</span>
        </div>

        <div class="field">
          <label for="set-shop">{{ t("settings.shopName") }}</label>
          <input id="set-shop" v-model="shopName" class="input" @blur="saveShop" />
        </div>
        <div class="field">
          <label for="set-info">{{ t("settings.shopInfo") }}</label>
          <textarea
            id="set-info"
            v-model="shopInfo"
            class="textarea"
            rows="3"
            :placeholder="t('settings.shopInfoPlaceholder')"
            @blur="saveShop"
          />
        </div>
        <button class="btn btn--primary" type="button" @click="saveShop">
          {{ t("common.saveChanges") }}
        </button>
      </div>
    </section>

    <!-- Desktop only: there is no database file to snapshot in the browser
         preview, which runs against the in-memory mock. -->
    <section v-if="isTauri()" class="card set-card">
      <div class="card-header">
        <h2>{{ t("settings.backup") }}</h2>
      </div>
      <div class="set-body">
        <div class="field">
          <button class="btn btn--ghost" type="button" @click="backupDatabase">
            <AppIcon name="download" :size="16" /> {{ t("settings.backupAction") }}
          </button>
          <span class="hint">{{ t("settings.backupHint") }}</span>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.settings {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
  gap: 20px;
  max-width: 900px;
}
.set-body {
  padding: 4px 22px 22px;
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.hint {
  font-size: 12px;
  color: var(--text-muted);
}
.field-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-secondary);
}
.logo-field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.logo-row {
  display: flex;
  align-items: center;
  gap: 16px;
}
.logo-preview {
  width: 64px;
  height: 64px;
  border-radius: 14px;
  background: var(--bg);
  border: 1px solid var(--border);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  overflow: hidden;
}
.logo-preview img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.logo-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.hidden-input {
  display: none;
}
.textarea {
  resize: vertical;
}
</style>
