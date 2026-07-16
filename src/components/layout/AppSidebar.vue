<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import { useStatsStore } from "@/stores/stats";
import { useSettingsStore } from "@/stores/settings";
import { resolveLogoSrc } from "@/lib/assets";

const { t } = useI18n();
const router = useRouter();
const stats = useStatsStore();
const settings = useSettingsStore();

interface NavItem {
  name: string;
  route: string;
  icon: string;
  badge?: () => number;
  badgeKind?: "danger" | "warning";
}

const items: NavItem[] = [
  { name: "dashboard", route: "/", icon: "home" },
  { name: "achats", route: "/achats", icon: "cart" },
  { name: "clients", route: "/clients", icon: "users" },
  { name: "paiements", route: "/paiements", icon: "card" },
  { name: "echeances", route: "/echeances", icon: "calendar" },
  { name: "impayes", route: "/impayes", icon: "alert", badge: () => stats.overdueClients, badgeKind: "danger" },
  { name: "alertes", route: "/alertes", icon: "bell", badge: () => stats.overdueInstallments, badgeKind: "warning" },
  { name: "rapports", route: "/rapports", icon: "report" },
  { name: "parametres", route: "/parametres", icon: "settings" },
];

const logoSrc = computed(() => resolveLogoSrc(settings.logoPath));

function newPurchase() {
  router.push({ name: "achats", query: { new: "1" } });
}
</script>

<template>
  <aside class="sidebar">
    <div class="brand">
      <div class="brand-logo">
        <img v-if="logoSrc" :src="logoSrc" alt="" class="brand-logo-img" />
        <AppIcon v-else name="washer" :size="26" :stroke-width="1.8" />
      </div>
      <div class="brand-text">
        <span class="brand-line1">{{ t("app.title") }}</span>
        <span class="brand-line2">{{ t("app.titleLine2") }}</span>
      </div>
    </div>

    <button class="new-purchase" type="button" @click="newPurchase">
      <AppIcon name="plus" :size="18" />
      <span>{{ t("sidebar.newPurchase") }}</span>
    </button>

    <nav class="nav">
      <RouterLink
        v-for="item in items"
        :key="item.name"
        :to="item.route"
        class="nav-item"
        active-class="is-active"
        :exact-active-class="item.route === '/' ? 'is-active' : ''"
      >
        <span class="nav-icon"><AppIcon :name="item.icon" :size="19" /></span>
        <span class="nav-label">{{ t(`nav.${item.name}`) }}</span>
        <span
          v-if="item.badge && item.badge() > 0"
          class="nav-badge"
          :class="`nav-badge--${item.badgeKind}`"
          >{{ item.badge() }}</span
        >
      </RouterLink>
    </nav>

    <div class="help-card">
      <div class="help-head">
        <AppIcon name="help" :size="18" />
        <span>{{ t("sidebar.helpTitle") }}</span>
      </div>
      <p class="help-text">{{ t("sidebar.helpText") }}</p>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: var(--sidebar-w);
  min-width: var(--sidebar-w);
  height: 100vh;
  background: linear-gradient(180deg, var(--navy-800) 0%, var(--navy-900) 100%);
  color: var(--sidebar-text);
  display: flex;
  flex-direction: column;
  padding: 20px 16px;
  gap: 18px;
  overflow-y: auto;
}

.brand {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 4px 6px;
}
.brand-logo {
  width: 42px;
  height: 42px;
  border-radius: 11px;
  background: #fff;
  color: var(--navy-800);
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}
.brand-logo-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.brand-text {
  display: flex;
  flex-direction: column;
  line-height: 1.15;
}
.brand-line1,
.brand-line2 {
  color: #fff;
  font-weight: 700;
  font-size: 16px;
  letter-spacing: -0.01em;
}

.new-purchase {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  width: 100%;
  padding: 12px 14px;
  border: none;
  border-radius: 10px;
  background: var(--primary);
  color: #fff;
  font-size: 14.5px;
  font-weight: 600;
  box-shadow: 0 6px 16px rgba(37, 99, 235, 0.35);
  transition: background 0.15s ease;
}
.new-purchase:hover {
  background: var(--primary-hover);
}

.nav {
  display: flex;
  flex-direction: column;
  gap: 3px;
  margin-top: 2px;
}
.nav-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border-radius: 9px;
  color: var(--sidebar-text);
  font-size: 14.5px;
  font-weight: 500;
  text-decoration: none;
  transition: background 0.13s ease, color 0.13s ease;
}
.nav-item:hover {
  background: var(--navy-hover);
  color: #fff;
  text-decoration: none;
}
.nav-item.is-active {
  background: var(--navy-active);
  color: #fff;
  font-weight: 600;
}
.nav-icon {
  display: inline-flex;
  color: var(--sidebar-text-muted);
}
.nav-item:hover .nav-icon,
.nav-item.is-active .nav-icon {
  color: #fff;
}
.nav-label {
  flex: 1;
}
.nav-badge {
  min-width: 22px;
  height: 22px;
  padding: 0 6px;
  border-radius: 999px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 700;
  color: #fff;
}
.nav-badge--danger {
  background: var(--danger);
}
.nav-badge--warning {
  background: var(--warning);
}

.help-card {
  margin-top: auto;
  background: rgba(255, 255, 255, 0.06);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 12px;
  padding: 14px 16px;
}
.help-head {
  display: flex;
  align-items: center;
  gap: 8px;
  color: #fff;
  font-weight: 600;
  font-size: 14px;
}
.help-text {
  margin-top: 6px;
  font-size: 12.5px;
  color: var(--sidebar-text-muted);
  line-height: 1.4;
}
</style>
