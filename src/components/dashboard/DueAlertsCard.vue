<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import { useFormat } from "@/composables/useFormat";
import type { DueAlert } from "@/types/models";

defineProps<{ alerts: DueAlert[] }>();
const { t } = useI18n();
const fmt = useFormat();
const router = useRouter();

function open(id: number) {
  router.push({ name: "achat-detail", params: { id } });
}
</script>

<template>
  <section class="card">
    <div class="card-header">
      <h2>{{ t("dashboard.dueAlerts") }}</h2>
      <RouterLink class="card-link" to="/echeances">{{ t("common.viewAllF") }}</RouterLink>
    </div>

    <EmptyState v-if="alerts.length === 0" icon="calendar" :title="t('dashboard.empty.alerts')" />

    <ul v-else class="alert-list">
      <li v-for="a in alerts" :key="`${a.purchaseId}-${a.index}`" class="alert-row">
        <span class="alert-icon"><AppIcon name="calendar" :size="18" /></span>
        <div class="alert-text">
          <span class="alert-title">
            <a class="row-link" href="#" @click.prevent="open(a.purchaseId)">{{ a.reference }}</a>
            <span class="muted"> — {{ a.clientName }}</span>
          </span>
          <span class="alert-sub">
            {{ t("dashboard.alert.tranche", { index: a.index, count: a.installmentCount }) }}
            <span class="dot">·</span>
            {{ t("dashboard.alert.dueOn", { date: fmt.date(a.dueDate) }) }}
          </span>
        </div>
        <span class="alert-late">{{ t("dashboard.alert.daysLate", a.daysLate) }}</span>
      </li>
    </ul>
  </section>
</template>

<style scoped>
.alert-list {
  list-style: none;
  margin: 0;
  padding: 0 8px 8px;
}
.alert-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border-radius: 10px;
}
.alert-row:hover {
  background: #fdf7f7;
}
.alert-icon {
  width: 34px;
  height: 34px;
  border-radius: 9px;
  background: var(--danger-bg);
  color: var(--danger-strong);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.alert-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  min-width: 0;
}
.alert-title {
  font-size: 14px;
  font-weight: 600;
}
.alert-sub {
  font-size: 12.5px;
  color: var(--text-secondary);
}
.dot {
  margin: 0 4px;
  color: var(--text-muted);
}
.alert-late {
  font-size: 13px;
  font-weight: 600;
  color: var(--danger-strong);
  white-space: nowrap;
}
</style>
