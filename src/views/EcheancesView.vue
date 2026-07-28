<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import ListFilterBar from "@/components/ui/ListFilterBar.vue";
import LoadError from "@/components/ui/LoadError.vue";
import { useFormat } from "@/composables/useFormat";
import { useLoader } from "@/composables/useLoader";
import { useSort } from "@/composables/useSort";
import { api } from "@/api";
import type { ScheduleRow } from "@/types/models";

const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();

type Filter = "all" | "overdue" | "upcoming" | "paid";
const rows = ref<ScheduleRow[]>([]);
const filter = ref<Filter>("all");

const search = ref("");
const amountMin = ref("");
const amountMax = ref("");
const dateFrom = ref("");
const dateTo = ref("");

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "echeances.filter.all" },
  { key: "overdue", label: "echeances.filter.overdue" },
  { key: "upcoming", label: "echeances.filter.upcoming" },
  { key: "paid", label: "echeances.filter.paid" },
];

function matchesStatus(r: ScheduleRow): boolean {
  switch (filter.value) {
    case "overdue":
      return r.status === "late";
    case "upcoming":
      return r.status === "pending" || r.status === "partial";
    case "paid":
      return r.status === "paid";
    default:
      return true;
  }
}

const filtered = computed(() => {
  const needle = search.value.trim().toLowerCase();
  const min = amountMin.value === "" ? null : Number(amountMin.value);
  const max = amountMax.value === "" ? null : Number(amountMax.value);
  return rows.value.filter((r) => {
    if (!matchesStatus(r)) return false;
    if (needle && !`${r.reference} ${r.clientName}`.toLowerCase().includes(needle)) return false;
    if (min != null && r.amount < min) return false;
    if (max != null && r.amount > max) return false;
    if (dateFrom.value && r.dueDate < dateFrom.value) return false;
    if (dateTo.value && r.dueDate > dateTo.value) return false;
    return true;
  });
});

const { sort, sorted } = useSort(filtered, {
  reference: (r) => r.reference,
  client: (r) => r.clientName,
  tranche: (r) => r.index,
  dueDate: (r) => r.dueDate,
  amount: (r) => r.amount,
  status: (r) => r.status,
});

const {
  loading,
  error: loadError,
  run: load,
} = useLoader(async () => {
  rows.value = await api.listSchedule();
});
onMounted(load);
</script>

<template>
  <div class="page">
    <LoadError v-if="loadError" :message="loadError" @retry="load" />

    <template v-else>
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

        <ListFilterBar
          v-model:search="search"
          v-model:amount-min="amountMin"
          v-model:amount-max="amountMax"
          v-model:date-from="dateFrom"
          v-model:date-to="dateTo"
          show-amount
        />

        <EmptyState
          v-if="!loading && filtered.length === 0"
          icon="calendar"
          :title="t('echeances.empty')"
        />
        <div v-else class="table-scroll">
          <table class="table">
            <thead>
              <tr>
                <SortHeader
                  :sort="sort"
                  field="reference"
                  :label="t('echeances.columns.reference')"
                />
                <SortHeader :sort="sort" field="client" :label="t('echeances.columns.client')" />
                <SortHeader :sort="sort" field="tranche" :label="t('echeances.columns.tranche')" />
                <SortHeader :sort="sort" field="dueDate" :label="t('echeances.columns.dueDate')" />
                <SortHeader :sort="sort" field="amount" :label="t('echeances.columns.amount')" />
                <SortHeader :sort="sort" field="status" :label="t('echeances.columns.status')" />
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="r in sorted"
                :key="r.installmentId"
                class="clickable"
                :class="{ 'is-late': r.status === 'late' }"
                @click="router.push({ name: 'achat-detail', params: { id: r.purchaseId } })"
              >
                <td>
                  <span class="row-link">{{ r.reference }}</span>
                </td>
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
