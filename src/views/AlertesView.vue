<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import ListFilterBar from "@/components/ui/ListFilterBar.vue";
import { useFormat } from "@/composables/useFormat";
import { useSort } from "@/composables/useSort";
import { useSettingsStore } from "@/stores/settings";
import { api } from "@/api";
import { buildAlerts, type AlertKind, type AlertRow } from "@/lib/alerts";
import { todayIso } from "@/lib/finance";
import type { ScheduleRow } from "@/types/models";

const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();
const settings = useSettingsStore();

type Filter = "all" | AlertKind;
const schedule = ref<ScheduleRow[]>([]);
const filter = ref<Filter>("all");
const loading = ref(true);

// The "due soon" horizon is a user setting, so derive alerts reactively: editing
// it in Settings re-classifies the visible rows without a reload.
const alerts = computed(() => buildAlerts(schedule.value, todayIso(), settings.alertSoonDays));

const search = ref("");
const amountMin = ref("");
const amountMax = ref("");
const dateFrom = ref("");
const dateTo = ref("");

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "alertes.filter.all" },
  { key: "overdue", label: "alertes.filter.overdue" },
  { key: "dueToday", label: "alertes.filter.dueToday" },
  { key: "dueSoon", label: "alertes.filter.dueSoon" },
];

// Summary tiles: count + total remaining per alert kind, over the full set
// (independent of the active tab / list filters).
const summary = computed(() => {
  const acc: Record<AlertKind, { count: number; total: number }> = {
    overdue: { count: 0, total: 0 },
    dueToday: { count: 0, total: 0 },
    dueSoon: { count: 0, total: 0 },
  };
  for (const a of alerts.value) {
    acc[a.kind].count += 1;
    acc[a.kind].total += a.remaining;
  }
  return acc;
});

const filtered = computed(() => {
  const needle = search.value.trim().toLowerCase();
  const min = amountMin.value === "" ? null : Number(amountMin.value);
  const max = amountMax.value === "" ? null : Number(amountMax.value);
  return alerts.value.filter((a) => {
    if (filter.value !== "all" && a.kind !== filter.value) return false;
    if (needle && !`${a.reference} ${a.clientName}`.toLowerCase().includes(needle)) return false;
    if (min != null && a.remaining < min) return false;
    if (max != null && a.remaining > max) return false;
    if (dateFrom.value && a.dueDate < dateFrom.value) return false;
    if (dateTo.value && a.dueDate > dateTo.value) return false;
    return true;
  });
});

const { sort, sorted } = useSort(filtered, {
  reference: (a) => a.reference,
  client: (a) => a.clientName,
  tranche: (a) => a.index,
  dueDate: (a) => a.dueDate,
  amount: (a) => a.remaining,
  // Overdue days count up, upcoming days count down — sort by the signed
  // distance so the most urgent (most overdue) sit at one end.
  timing: (a) => (a.kind === "overdue" ? -a.days : a.days),
});

function timingLabel(a: AlertRow): string {
  if (a.kind === "overdue") return t("alertes.timing.daysLate", a.days);
  if (a.kind === "dueToday") return t("alertes.timing.today");
  return t("alertes.timing.inDays", a.days);
}

onMounted(async () => {
  schedule.value = await api.listSchedule();
  loading.value = false;
});
</script>

<template>
  <div class="page">
    <div class="summary">
      <button
        class="tile card"
        :class="{ 'tile--active': filter === 'overdue' }"
        type="button"
        @click="filter = filter === 'overdue' ? 'all' : 'overdue'"
      >
        <span class="tile-icon tile-icon--red"><AppIcon name="alert" :size="22" /></span>
        <span class="tile-body">
          <span class="tile-value tabular">{{ summary.overdue.count }}</span>
          <span class="tile-label">{{ t("alertes.summary.overdue") }}</span>
          <span class="tile-sub tabular">{{ fmt.money(summary.overdue.total) }}</span>
        </span>
      </button>
      <button
        class="tile card"
        :class="{ 'tile--active': filter === 'dueToday' }"
        type="button"
        @click="filter = filter === 'dueToday' ? 'all' : 'dueToday'"
      >
        <span class="tile-icon tile-icon--orange"><AppIcon name="calendar" :size="22" /></span>
        <span class="tile-body">
          <span class="tile-value tabular">{{ summary.dueToday.count }}</span>
          <span class="tile-label">{{ t("alertes.summary.dueToday") }}</span>
          <span class="tile-sub tabular">{{ fmt.money(summary.dueToday.total) }}</span>
        </span>
      </button>
      <button
        class="tile card"
        :class="{ 'tile--active': filter === 'dueSoon' }"
        type="button"
        @click="filter = filter === 'dueSoon' ? 'all' : 'dueSoon'"
      >
        <span class="tile-icon tile-icon--blue"><AppIcon name="bell" :size="22" /></span>
        <span class="tile-body">
          <span class="tile-value tabular">{{ summary.dueSoon.count }}</span>
          <span class="tile-label">{{ t("alertes.summary.dueSoon", { days: settings.alertSoonDays }) }}</span>
          <span class="tile-sub tabular">{{ fmt.money(summary.dueSoon.total) }}</span>
        </span>
      </button>
    </div>

    <div class="card">
      <div class="card-header">
        <div>
          <h2>{{ t("alertes.title") }}</h2>
          <p class="subtitle">{{ t("alertes.subtitle") }}</p>
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

      <ListFilterBar
        v-model:search="search"
        v-model:amount-min="amountMin"
        v-model:amount-max="amountMax"
        v-model:date-from="dateFrom"
        v-model:date-to="dateTo"
        show-amount
      />

      <EmptyState v-if="!loading && filtered.length === 0" icon="bell" :title="t('alertes.empty')" />
      <table v-else class="table">
        <thead>
          <tr>
            <SortHeader :sort="sort" field="reference" :label="t('alertes.columns.reference')" />
            <SortHeader :sort="sort" field="client" :label="t('alertes.columns.client')" />
            <SortHeader :sort="sort" field="tranche" :label="t('alertes.columns.tranche')" />
            <SortHeader :sort="sort" field="dueDate" :label="t('alertes.columns.dueDate')" />
            <SortHeader :sort="sort" field="amount" :label="t('alertes.columns.amount')" />
            <SortHeader :sort="sort" field="timing" :label="t('alertes.columns.timing')" />
            <th>{{ t("common.status") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="a in sorted"
            :key="a.installmentId"
            class="clickable"
            :class="{ 'is-late': a.kind === 'overdue' }"
            @click="router.push({ name: 'achat-detail', params: { id: a.purchaseId } })"
          >
            <td><span class="row-link">{{ a.reference }}</span></td>
            <td>{{ a.clientName }}</td>
            <td class="tabular">{{ a.index }}/{{ a.installmentCount }}</td>
            <td class="tabular">{{ fmt.date(a.dueDate) }}</td>
            <td class="tabular strong">{{ fmt.money(a.remaining) }}</td>
            <td>
              <span class="timing" :class="`timing--${a.kind}`">{{ timingLabel(a) }}</span>
            </td>
            <td><StatusBadge :status="a.status" feminine /></td>
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
.summary {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
}
.tile {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 18px 20px;
  text-align: start;
  border: 1px solid transparent;
  cursor: pointer;
}
.tile:hover {
  border-color: var(--border-strong);
}
.tile--active {
  border-color: var(--primary);
}
.tile-icon {
  width: 48px;
  height: 48px;
  border-radius: 13px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  flex-shrink: 0;
}
.tile-icon--red {
  background: #ef4444;
}
.tile-icon--orange {
  background: #f59e0b;
}
.tile-icon--blue {
  background: #2563eb;
}
.tile-body {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}
.tile-value {
  font-size: 22px;
  font-weight: 700;
  line-height: 1.1;
}
.tile-label {
  font-size: 13px;
  color: var(--text-secondary);
  font-weight: 600;
}
.tile-sub {
  font-size: 12.5px;
  color: var(--text-muted);
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
.timing {
  font-size: 12.5px;
  font-weight: 600;
  white-space: nowrap;
}
.timing--overdue {
  color: var(--danger-strong);
}
.timing--dueToday {
  color: var(--warning);
}
.timing--dueSoon {
  color: var(--text-secondary);
}
</style>
