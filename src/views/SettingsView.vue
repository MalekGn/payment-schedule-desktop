<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import {
  useSettingsStore,
  DATE_FORMATS,
  CURRENCIES,
  ALERT_SOON_DAYS_MIN,
  ALERT_SOON_DAYS_MAX,
  BACKUP_FREQUENCIES,
} from "@/stores/settings";
import { useUiStore } from "@/stores/ui";
import { useLicenseStore } from "@/stores/license";
import { SUPPORTED_LOCALES, type AppLocale } from "@/i18n";
import { resolveLogoSrc } from "@/lib/assets";
import { formatDatePattern, useFormat } from "@/composables/useFormat";
import { isTauri } from "@/api";
import { toUserMessage } from "@/lib/errors";
import { todayIso } from "@/lib/finance";
import type { BackupEntry } from "@/types/models";

const { t } = useI18n();
const settings = useSettingsStore();
const ui = useUiStore();
const license = useLicenseStore();
const fmt = useFormat();

/**
 * Everything on this page is a licensed setting except three: the language, the
 * licence section itself, and the backup. Language stays open deliberately —
 * locking someone out of a language they cannot read would make the licence
 * screen unusable — and the backend applies the same rule (`is_language_only`
 * in `commands.rs`). Backup stays open because a shop must always be able to
 * copy its own ledger; `backup_database` carries no gate either.
 */
const locked = computed(() => !license.isLicensed);

/** `"machineMismatch"` → `license.statusMachineMismatch`. */
const statusLabel = computed(() => {
  const tag = license.status;
  return t(`license.status${tag.charAt(0).toUpperCase()}${tag.slice(1)}`);
});

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

/**
 * The shop name is not edited here: it comes from the licence (`licensee`) and
 * is shown beside the logo in the sidebar. Only the free-form shop info is
 * user-owned, so it is the only field this writes.
 */
async function saveShopInfo() {
  await save(() => settings.update({ shopInfo: shopInfo.value }));
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
  await save(() => settings.backupDatabase(dest), "settings.backupDone");
}

/**
 * The backup line under the button: when it last happened, or that it never
 * has. Manual backups are invisible otherwise — nothing else in the app ever
 * says whether this install has a copy of its data.
 */
const backupStatus = computed(() =>
  settings.lastBackupAt
    ? t("settings.backupLast", { date: fmt.date(settings.lastBackupAt) })
    : t("settings.backupNever"),
);

/**
 * The automatic-copy line. Shown as plain information, never as reassurance:
 * these snapshots live beside the database on the same disk, so they are a
 * safety net for mistakes, not for losing the machine. The nudge above is what
 * asks for a real backup, and it ignores this value entirely.
 */
/** Local copy of the time field, so a rejected value can be rolled back. */
const autoBackupTime = ref(settings.settings.autoBackupTime);

async function onAutoBackupEnabled(e: Event) {
  const enabled = (e.target as HTMLInputElement).checked;
  await save(() => settings.update({ autoBackupEnabled: enabled }));
}

async function onAutoBackupFrequency(e: Event) {
  const frequency = (e.target as HTMLSelectElement).value;
  await save(() => settings.update({ autoBackupFrequency: frequency }));
}

/**
 * `<input type="time">` can still hand us an empty string — the field is
 * clearable — and the backend refuses anything that is not `HH:MM`. Roll the
 * control back to what is stored rather than leaving the user looking at a
 * value that was never saved.
 */
async function onAutoBackupTime() {
  const value = autoBackupTime.value;
  if (!value) {
    autoBackupTime.value = settings.autoBackupTime;
    return;
  }
  await save(() => settings.update({ autoBackupTime: value }));
  autoBackupTime.value = settings.autoBackupTime;
}

const autoBackupStatus = computed(() =>
  settings.lastAutoBackupAt
    ? t("settings.backupAutoLast", { date: fmt.date(settings.lastAutoBackupAt) })
    : t("settings.backupAutoNone"),
);

// -- restore ---------------------------------------------------------------

/**
 * The snapshots `backups/` holds, newest first. Empty on a browser preview and
 * on an install whose backups directory cannot be read — the file picker below
 * is the path that needs neither.
 */
const backups = ref<BackupEntry[]>([]);
/** The chosen source, held while the confirmation is open. `null` when closed. */
const pendingRestore = ref<{ source: string; label: string } | null>(null);
/** Guards the confirm button: a restore swaps a file and must not be re-entered. */
const restoring = ref(false);

async function loadBackups() {
  if (!isTauri()) return;
  try {
    backups.value = await settings.listBackups();
  } catch (e) {
    // Never a toast: an unreadable backups directory costs the user the list,
    // not the feature, and the file picker still works.
    console.error("could not list the backups:", e);
  }
}

onMounted(loadBackups);

/** How a listed snapshot is named back to the user in the confirmation. */
function backupLabel(entry: BackupEntry): string {
  return `${fmt.date(entry.takenAt)} · ${t(`settings.backupKind_${entry.kind}`)}`;
}

/**
 * Size in kilobytes, rounded. Not `Intl` byte formatting: these files are all
 * in the same order of magnitude, and one consistent unit compares at a glance
 * where "0,25 Mo" against "980 Ko" does not.
 */
function backupSize(entry: BackupEntry): string {
  return t("settings.backupSize", { size: Math.max(1, Math.round(entry.sizeBytes / 1024)) });
}

function askRestoreFromList(entry: BackupEntry) {
  pendingRestore.value = { source: entry.path, label: backupLabel(entry) };
}

/**
 * Restore from a copy the user keeps off this machine — the case the listing
 * cannot serve, because a snapshot in `backups/` sits on the same disk as the
 * database it protects.
 */
async function pickBackupFile() {
  if (!isTauri()) return;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    multiple: false,
    filters: [{ name: "SQLite", extensions: ["db"] }],
  });
  if (typeof selected !== "string") return;
  pendingRestore.value = {
    source: selected,
    label: selected.split(/[\\/]/).pop() ?? selected,
  };
}

/**
 * Swap the database, then reload the whole WebView.
 *
 * The reload is not belt-and-braces: every store, route and computed in the app
 * is derived from a database that no longer exists, and there is no honest way
 * to reconcile them in place. Only a success reloads — a rejection leaves the
 * user exactly where they were, with their data untouched, which is the promise
 * the backend makes.
 */
async function confirmRestore() {
  const target = pendingRestore.value;
  if (!target || restoring.value) return;
  restoring.value = true;
  try {
    await settings.restoreDatabase(target.source);
    ui.notify(t("settings.restoreDone"));
    // Left armed deliberately, and the dialog left open: `reload()` only
    // *schedules* the navigation, so clearing the guard here would leave a
    // live Restaurer button in front of the user for the frames in between —
    // on the one action in the app that cannot be taken twice by accident.
    window.location.reload();
  } catch (e) {
    ui.notify(toUserMessage(e, t), "error");
    pendingRestore.value = null;
    restoring.value = false;
  }
}

function dateSample(pattern: string): string {
  return formatDatePattern(todayIso(), pattern);
}

// -- licence ---------------------------------------------------------------

/** Let the user hand their machine fingerprint to the vendor without retyping 64 hex chars. */
async function copyMachineId() {
  const id = license.machineId;
  if (!id) return;
  try {
    await navigator.clipboard.writeText(id);
    ui.notify(t("license.copied"));
  } catch (e) {
    // A denied clipboard permission must not look like success; the value is
    // still on screen and selectable.
    console.error("clipboard write failed:", e);
    ui.notify(toUserMessage(e, t), "error");
  }
}

async function pickLicense() {
  if (!isTauri()) return;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    multiple: false,
    filters: [{ name: "Licence", extensions: ["json"] }],
  });
  if (typeof selected !== "string") return;
  await save(() => license.importFrom(selected), "license.imported");
}
</script>

<template>
  <div class="settings">
    <section class="card set-card">
      <div class="card-header">
        <h2>{{ t("license.title") }}</h2>
      </div>
      <div class="set-body">
        <div class="lic-status" :class="`lic-status--${license.status}`">
          <AppIcon :name="license.isLicensed ? 'check' : 'lock'" :size="18" />
          <span>{{ t("license.status") }}:</span>
          <strong>{{ statusLabel }}</strong>
        </div>

        <template v-if="license.license">
          <div class="lic-row">
            <span class="lic-key">{{ t("license.licensee") }}</span>
            <span class="lic-val">{{ license.license.licensee }}</span>
          </div>
          <div class="lic-row">
            <span class="lic-key">{{ t("license.licenseId") }}</span>
            <span class="lic-val">{{ license.license.licenseId }}</span>
          </div>
          <div class="lic-row">
            <span class="lic-key">
              {{ license.status === "expired" ? t("license.expiredOn") : t("license.expiresAt") }}
            </span>
            <span class="lic-val">{{ fmt.date(license.license.expiresAt) }}</span>
          </div>
        </template>

        <div class="field">
          <span class="field-label">{{ t("license.machineId") }}</span>
          <div v-if="license.machineId" class="lic-machine">
            <code class="lic-fingerprint">{{ license.machineId }}</code>
            <button class="btn btn--ghost btn--sm" type="button" @click="copyMachineId">
              <AppIcon name="copy" :size="16" /> {{ t("license.copy") }}
            </button>
          </div>
          <span v-else class="hint">{{ t("license.machineIdUnavailable") }}</span>
          <span class="hint">{{ t("license.machineIdHint") }}</span>
        </div>

        <div v-if="isTauri()" class="field">
          <button class="btn btn--primary btn--sm" type="button" @click="pickLicense">
            <AppIcon name="upload" :size="16" /> {{ t("license.import") }}
          </button>
        </div>
      </div>
    </section>

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
          <select
            id="set-cur"
            class="select"
            :value="settings.currencyCode"
            :disabled="locked"
            @change="onCurrency"
          >
            <option v-for="c in CURRENCIES" :key="c" :value="c">{{ c }}</option>
          </select>
          <span class="hint">{{ t("settings.currencyHint") }}</span>
        </div>

        <div class="field">
          <label for="set-date">{{ t("settings.dateFormat") }}</label>
          <select
            id="set-date"
            class="select"
            :value="settings.dateFormat"
            :disabled="locked"
            @change="onDateFormat"
          >
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
            :disabled="locked"
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
              <button
                class="btn btn--ghost btn--sm"
                type="button"
                :disabled="locked"
                @click="pickLogo"
              >
                <AppIcon name="upload" :size="16" /> {{ t("settings.uploadLogo") }}
              </button>
              <button
                v-if="settings.logoPath"
                class="btn btn--ghost btn--sm"
                type="button"
                :disabled="locked"
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
          <label for="set-info">{{ t("settings.shopInfo") }}</label>
          <textarea
            id="set-info"
            v-model="shopInfo"
            :disabled="locked"
            class="textarea"
            rows="3"
            :placeholder="t('settings.shopInfoPlaceholder')"
            @blur="saveShopInfo"
          />
        </div>
        <button class="btn btn--primary" type="button" :disabled="locked" @click="saveShopInfo">
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
          <!-- Not `:disabled="locked"`, unlike every other control on this
               page: the backup command carries no licence gate either, so a
               shop whose licence expired can still copy its own ledger. -->
          <button class="btn btn--ghost" type="button" @click="backupDatabase">
            <AppIcon name="download" :size="16" /> {{ t("settings.backupAction") }}
          </button>
          <label class="check">
            <input
              type="checkbox"
              :checked="settings.autoBackupEnabled"
              :disabled="locked"
              @change="onAutoBackupEnabled"
            />
            {{ t("settings.backupAuto") }}
          </label>
          <div class="field">
            <label for="set-backup-frequency">{{ t("settings.backupFrequency") }}</label>
            <select
              id="set-backup-frequency"
              class="input"
              :value="settings.autoBackupFrequency"
              :disabled="locked || !settings.autoBackupEnabled"
              @change="onAutoBackupFrequency"
            >
              <option v-for="f in BACKUP_FREQUENCIES" :key="f" :value="f">
                {{ t(`settings.backupFrequency_${f}`) }}
              </option>
            </select>
          </div>
          <div class="field">
            <label for="set-backup-time">{{ t("settings.backupTime") }}</label>
            <input
              id="set-backup-time"
              v-model="autoBackupTime"
              type="time"
              class="input"
              :disabled="locked || !settings.autoBackupEnabled"
              @change="onAutoBackupTime"
            />
            <span class="hint">{{ t("settings.backupTimeHint") }}</span>
          </div>
          <span :class="['hint', { 'hint--warn': settings.backupIsStale }]">
            {{ backupStatus }}
          </span>
          <span class="hint">{{ t("settings.backupHint") }}</span>
          <span class="hint">{{ autoBackupStatus }}</span>
        </div>
      </div>
    </section>

    <!-- Its own card rather than a block inside Sauvegarde: this is the one
         destructive action on the page, and a header the user can scan for is
         what makes it findable at the moment they need it. Desktop only, for
         the same reason as the backup card. -->
    <section v-if="isTauri()" class="card set-card">
      <div class="card-header">
        <h2>{{ t("settings.restore") }}</h2>
      </div>
      <div class="set-body">
        <span class="hint">{{ t("settings.restoreHint") }}</span>

        <!-- Ungated, like the backup button above: recovery must not depend on
             the state of a licence. -->
        <button class="btn btn--ghost" type="button" @click="pickBackupFile">
          <AppIcon name="upload" :size="16" /> {{ t("settings.restoreFromFile") }}
        </button>

        <div class="field">
          <span class="field-label">{{ t("settings.restoreFromList") }}</span>
          <p v-if="backups.length === 0" class="hint">{{ t("settings.restoreEmpty") }}</p>
          <ul v-else class="snap-list">
            <li v-for="entry in backups" :key="entry.path" class="snap-row">
              <span class="snap-desc">
                <strong>{{ fmt.date(entry.takenAt) }}</strong>
                <span class="hint">
                  {{ t(`settings.backupKind_${entry.kind}`) }} · {{ backupSize(entry) }}
                </span>
              </span>
              <button class="btn btn--ghost" type="button" @click="askRestoreFromList(entry)">
                {{ t("settings.restoreAction") }}
              </button>
            </li>
          </ul>
        </div>
      </div>
    </section>

    <ConfirmDialog
      v-if="pendingRestore"
      danger
      :title="t('settings.restoreConfirmTitle')"
      :message="t('settings.restoreConfirmBody', { source: pendingRestore.label })"
      :confirm-label="t('settings.restoreAction')"
      :confirm-disabled="restoring"
      @close="pendingRestore = null"
      @confirm="confirmRestore"
    />
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
/* Muted is the wrong register for "you have no backup" — it is the one hint on
   this page the user is meant to act on. */
.check {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  font-size: 13px;
}
.hint--warn {
  color: var(--warning-text);
  font-weight: 600;
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

/* -- restore -- */
.snap-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.snap-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: 10px;
}
.snap-desc {
  display: flex;
  flex-direction: column;
  /* Logical, not `text-align: left`: the rows mirror wholesale under RTL. */
  align-items: flex-start;
  gap: 2px;
}

/* -- licence -- */
.lic-status {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--surface-2, rgba(127, 127, 127, 0.08));
  color: var(--text-secondary);
}
.lic-status strong {
  color: var(--text-primary);
}
.lic-status--valid {
  color: var(--success, #16a34a);
}
.lic-row {
  display: flex;
  gap: 12px;
  align-items: baseline;
}
.lic-key {
  min-width: 160px;
  color: var(--text-secondary);
}
.lic-val {
  font-weight: 600;
}
.lic-machine {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
/* The fingerprint is 64 hex characters; it must wrap rather than widen the card. */
.lic-fingerprint {
  font-family: ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.5;
  overflow-wrap: anywhere;
  user-select: all;
  padding: 6px 8px;
  border-radius: 8px;
  background: var(--surface-2, rgba(127, 127, 127, 0.08));
}
</style>
