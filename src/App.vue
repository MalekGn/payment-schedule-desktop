<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import AppSidebar from "@/components/layout/AppSidebar.vue";
import AppHeader from "@/components/layout/AppHeader.vue";
import AppToasts from "@/components/ui/AppToasts.vue";
import LicenseRequiredPanel from "@/components/LicenseRequiredPanel.vue";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { useLicenseStore } from "@/stores/license";

const ui = useUiStore();
const stats = useStatsStore();
const license = useLicenseStore();
const route = useRoute();
const { t } = useI18n();

/**
 * Whether the current route's content is withheld pending a licence.
 *
 * The single gate site for the whole app: routes opt in with `meta.licensed`
 * (see `router/index.ts`) rather than each view checking for itself.
 */
const blocked = computed(() => route.meta.licensed === true && !license.isLicensed);

// The dashboard aggregate is a licensed read, so an unlicensed install would
// only get a refusal toast on every launch. Skip it and let the gate explain.
onMounted(() => {
  if (license.isLicensed) stats.refresh();
});
// The backend re-evaluates the licence while the app runs, so `blocked` above can
// flip under a user who is mid-task. It swapping the page silently would read as
// a bug; an error toast persists until dismissed, so the reason is still on
// screen when they look up. Only the losing direction is announced — regaining a
// licence already has its own confirmation on the import path.
watch(
  () => license.isLicensed,
  (now, before) => {
    if (before && !now) ui.notify(t("license.lapsed"), "error");
  },
);
// Reset any per-page header-title override on navigation.
watch(
  () => route.fullPath,
  () => {
    ui.pageTitle = null;
  },
);
</script>

<template>
  <div class="app-shell" :class="{ 'sidebar-collapsed': !ui.sidebarOpen }">
    <AppSidebar v-show="ui.sidebarOpen" />
    <div class="app-main">
      <AppHeader />
      <main class="app-content">
        <LicenseRequiredPanel v-if="blocked" />
        <RouterView v-else v-slot="{ Component }">
          <component :is="Component" />
        </RouterView>
      </main>
    </div>
    <AppToasts />
  </div>
</template>

<style scoped>
.app-shell {
  display: flex;
  height: 100vh;
  overflow: hidden;
}
.app-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.app-content {
  flex: 1;
  overflow-y: auto;
  padding: 26px 28px;
}
</style>
