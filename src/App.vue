<script setup lang="ts">
import { onMounted, watch } from "vue";
import { useRoute } from "vue-router";
import AppSidebar from "@/components/layout/AppSidebar.vue";
import AppHeader from "@/components/layout/AppHeader.vue";
import AppToasts from "@/components/ui/AppToasts.vue";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";

const ui = useUiStore();
const stats = useStatsStore();
const route = useRoute();

onMounted(() => stats.refresh());
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
        <RouterView v-slot="{ Component }">
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
