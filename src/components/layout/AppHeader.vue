<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import { useClickOutside } from "@/composables/useClickOutside";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { useSettingsStore } from "@/stores/settings";
import { SUPPORTED_LOCALES, type AppLocale } from "@/i18n";

const { t } = useI18n();
const route = useRoute();
const ui = useUiStore();
const stats = useStatsStore();
const settings = useSettingsStore();

// --- Language switcher ---
const LANGUAGE_LABELS: Record<AppLocale, string> = {
  fr: "Français",
  en: "English",
  ar: "العربية",
};
const langOpen = ref(false);
const langRef = ref<HTMLElement | null>(null);

async function pickLanguage(lang: AppLocale) {
  langOpen.value = false;
  if (lang !== settings.language) {
    await settings.setLanguage(lang);
    ui.notify(t("settings.saved"));
  }
}

useClickOutside(langRef, () => (langOpen.value = false));

// Route name → i18n nav key; detail routes fall back to the ui override.
const NAV_KEY: Record<string, string> = {
  dashboard: "nav.dashboard",
  achats: "nav.achats",
  clients: "nav.clients",
  paiements: "nav.paiements",
  echeances: "nav.echeances",
  impayes: "nav.impayes",
  alertes: "nav.alertes",
  rapports: "nav.rapports",
  parametres: "nav.parametres",
  "not-found": "notFound.title",
};

const title = computed(() => {
  if (ui.pageTitle) return ui.pageTitle;
  const key = NAV_KEY[String(route.name)];
  return key ? t(key) : t("app.name");
});

const alertCount = computed(() => stats.overdueInstallments);
</script>

<template>
  <header class="header">
    <button
      class="icon-btn menu-btn"
      type="button"
      :aria-label="'menu'"
      @click="ui.toggleSidebar()"
    >
      <AppIcon name="menu" :size="22" />
    </button>
    <h1 class="page-title">{{ title }}</h1>

    <div class="spacer" />

    <div ref="langRef" class="lang">
      <button
        class="lang-btn"
        type="button"
        :aria-label="t('settings.language')"
        :aria-expanded="langOpen"
        @click="langOpen = !langOpen"
      >
        <AppIcon name="globe" :size="19" />
        <span class="lang-current">{{ LANGUAGE_LABELS[settings.language] }}</span>
        <AppIcon name="chevron-down" :size="15" class="muted" />
      </button>
      <ul v-if="langOpen" class="lang-menu" role="menu">
        <li v-for="l in SUPPORTED_LOCALES" :key="l">
          <button
            class="lang-option"
            :class="{ 'is-active': l === settings.language }"
            type="button"
            role="menuitem"
            @click="pickLanguage(l)"
          >
            <span>{{ LANGUAGE_LABELS[l] }}</span>
            <AppIcon v-if="l === settings.language" name="check" :size="16" />
          </button>
        </li>
      </ul>
    </div>

    <button class="icon-btn bell" type="button" aria-label="notifications">
      <AppIcon name="bell" :size="21" />
      <span v-if="alertCount > 0" class="bell-badge">{{ alertCount }}</span>
    </button>

    <div class="user">
      <div class="avatar">A</div>
      <span class="user-name">{{ t("header.admin") }}</span>
      <AppIcon name="chevron-down" :size="16" class="muted" />
    </div>
  </header>
</template>

<style scoped>
.header {
  height: var(--header-h);
  min-height: var(--header-h);
  background: var(--surface);
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0 28px;
}
.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border: none;
  border-radius: 10px;
  background: transparent;
  color: var(--text-secondary);
  transition:
    background 0.13s ease,
    color 0.13s ease;
  position: relative;
}
.icon-btn:hover {
  background: var(--bg);
  color: var(--text);
}
.page-title {
  font-size: 21px;
  font-weight: 700;
  letter-spacing: -0.015em;
  color: var(--text);
}
.lang {
  position: relative;
}
.lang-btn {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 7px 10px;
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  background: var(--surface);
  color: var(--text-secondary);
  font-weight: 600;
  font-size: 13.5px;
  transition:
    background 0.13s ease,
    border-color 0.13s ease;
}
.lang-btn:hover {
  background: var(--bg);
  color: var(--text);
}
.lang-current {
  min-width: 58px;
  text-align: start;
}
.lang-menu {
  position: absolute;
  top: calc(100% + 6px);
  inset-inline-end: 0;
  min-width: 168px;
  list-style: none;
  margin: 0;
  padding: 6px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  box-shadow: var(--shadow-pop);
  z-index: 200;
}
.lang-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 9px 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  font-size: 14px;
  font-weight: 500;
  text-align: start;
}
.lang-option:hover {
  background: var(--bg);
}
.lang-option.is-active {
  color: var(--primary);
  font-weight: 600;
}
.bell-badge {
  position: absolute;
  top: 5px;
  inset-inline-end: 5px;
  min-width: 17px;
  height: 17px;
  padding: 0 4px;
  border-radius: 999px;
  background: var(--danger);
  color: #fff;
  font-size: 10.5px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 2px solid var(--surface);
}
.user {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 5px 8px;
  border-radius: 10px;
  cursor: pointer;
}
.user:hover {
  background: var(--bg);
}
.avatar {
  width: 36px;
  height: 36px;
  border-radius: 50%;
  background: linear-gradient(135deg, #6366f1, #2563eb);
  color: #fff;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 15px;
}
.user-name {
  font-weight: 600;
  font-size: 14.5px;
  color: var(--text);
}
</style>
