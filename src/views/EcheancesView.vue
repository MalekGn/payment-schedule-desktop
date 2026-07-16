<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import { useFormat } from "@/composables/useFormat";
import { api } from "@/api";
import type { ScheduleRow } from "@/types/models";

const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();

type Filter = "all" | "overdue" | "upcoming" | "paid";
const rows = ref<ScheduleRow[]>([]);
const filter = ref<Filter>("all");
const loading = ref(true);

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "echeances.filter.all" },
  { key: "overdue", label: "echeances.filter.overdue" },
  { key: "upcoming", label: "echeances.filter.upcoming" },
  { key: "paid", label: "echeances.filter.paid" },
];

const filtered = computed(() => {
  switch (filter.value) {
    case "overdue":
      return rows.value.filter((r) => r.status === "late");
    case "upcoming":
      return rows.value.filter((r) => r.status === "pending" || r.status === "partial");
    case "paid":
      return rows.value.filter((r) => r.status === "paid");
    default:
      return rows.value;
  }
});

onMounted(async () => {
  rows.value = await api.listSchedule();
  loading.value = false;
});
</script>

<template>
  <div class="page">
    <div class="card">
      <div class="card-header">
        <div>
          <h2>{{ t("echeances.title") }}</h2>
          <p class="subtitle">{{ t("echeances.subtitle") }}</p>
        </div>
        <div class="tabs">
          <button
            v-for="f in FILTERS"
            :key="f.key"
            class="tab"
            :class="{ 'tab--active': filter === f.key }"
            type="button"
            @click="filter = f.key"
          >
            {{ t(f.label) }}
          </button>
        </div>
      </div>

      <EmptyState v-if="!loading && filtered.length === 0" icon="calendar" :title="t('echeances.empty')" />
      <table v-else class="table">
        <thead>
          <tr>
            <th>{{ t("echeances.columns.reference") }}</th>
            <th>{{ t("echeances.columns.client") }}</th>
            <th>{{ t("echeances.columns.tranche") }}</th>
            <th>{{ t("echeances.columns.dueDate") }}</th>
            <th>{{ t("echeances.columns.amount") }}</th>
            <th>{{ t("echeances.columns.status") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="r in filtered"
            :key="r.installmentId"
            class="clickable"
            :class="{ 'is-late': r.status === 'late' }"
            @click="router.push({ name: 'achat-detail', params: { id: r.purchaseId } })"
          >
            <td><span class="row-link">{{ r.reference }}</span></td>
            <td>{{ r.clientName }}</td>
            <td class="tabular">{{ r.index }}/{{ r.installmentCount }}</td>
            <td class="tabular">{{ fmt.date(r.dueDate) }}</td>
            <td class="tabular strong">{{ fmt.money(r.amount) }}</td>
            <td><StatusBadge :status="r.status" feminine /></td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.subtitle {
  font-size: 13px;
  color: var(--text-muted);
  margin-top: 2px;
  font-weight: 400;
}
.tabs {
  display: flex;
  gap: 4px;
  background: var(--bg);
  padding: 4px;
  border-radius: 10px;
}
.tab {
  padding: 7px 14px;
  border: none;
  background: transparent;
  border-radius: 7px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-secondary);
}
.tab--active {
  background: var(--surface);
  color: var(--primary);
  box-shadow: var(--shadow-card);
}
.clickable {
  cursor: pointer;
}
.clickable:hover {
  background: var(--bg);
}
</style>
