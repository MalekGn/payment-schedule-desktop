<script setup lang="ts">
/**
 * Rapports — aggregated figures over a date range.
 *
 * Two populations of figure share this screen and must never be read as one
 * statement, which is why they are labelled separately: the KPI row's sales and
 * collections are *historical* (what happened between the two dates), while
 * outstanding, overdue, the aging table and the client risk table are a
 * *snapshot* taken as of `range.asOf` — always today. The backend echoes `asOf`
 * back precisely so this page can say so out loud.
 *
 * The aggregation itself is deliberately not done here: see the note on
 * `api.getReport`.
 */
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { api } from "@/api";
import AppIcon from "@/components/ui/AppIcon.vue";
import BarChart, { type BarChartSeries } from "@/components/ui/BarChart.vue";
import DatePicker from "@/components/ui/DatePicker.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import KpiCard from "@/components/ui/KpiCard.vue";
import LoadError from "@/components/ui/LoadError.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import { useFormat } from "@/composables/useFormat";
import { useCsvExport } from "@/composables/useCsvExport";
import { useLoader } from "@/composables/useLoader";
import { useSort } from "@/composables/useSort";
import { buildReportCsv } from "@/lib/csv";
import { todayIso } from "@/lib/finance";
import type { Report } from "@/types/models";

const { t } = useI18n();
const fmt = useFormat();
const { saveCsv } = useCsvExport();

// --- range ----------------------------------------------------------------

type PresetKey = "thisMonth" | "lastMonth" | "thisQuarter" | "thisYear" | "custom";

const PRESETS: PresetKey[] = ["thisMonth", "lastMonth", "thisQuarter", "thisYear"];

const iso = (y: number, m: number, d: number) =>
  `${String(y).padStart(4, "0")}-${String(m).padStart(2, "0")}-${String(d).padStart(2, "0")}`;

/** Last day of month `m` (1-based) in year `y`. Day 0 of the next month. */
const monthEnd = (y: number, m: number) => new Date(Date.UTC(y, m, 0)).getUTCDate();

/**
 * A preset's range. Whole calendar periods rather than "up to today", so the
 * chart shows what is still to fall due in the current period alongside what has
 * already come in — a month cut off at today would look like a shortfall.
 */
function presetRange(key: PresetKey): { from: string; to: string } {
  const now = todayIso();
  const y = Number(now.slice(0, 4));
  const m = Number(now.slice(5, 7));
  switch (key) {
    case "lastMonth": {
      const py = m === 1 ? y - 1 : y;
      const pm = m === 1 ? 12 : m - 1;
      return { from: iso(py, pm, 1), to: iso(py, pm, monthEnd(py, pm)) };
    }
    case "thisQuarter": {
      const start = Math.floor((m - 1) / 3) * 3 + 1;
      const end = start + 2;
      return { from: iso(y, start, 1), to: iso(y, end, monthEnd(y, end)) };
    }
    case "thisYear":
      return { from: iso(y, 1, 1), to: iso(y, 12, 31) };
    case "thisMonth":
    default:
      return { from: iso(y, m, 1), to: iso(y, m, monthEnd(y, m)) };
  }
}

const initial = presetRange("thisMonth");
const dateFrom = ref(initial.from);
const dateTo = ref(initial.to);
const preset = ref<PresetKey>("thisMonth");

function applyPreset(key: PresetKey) {
  const range = presetRange(key);
  dateFrom.value = range.from;
  dateTo.value = range.to;
  preset.value = key;
  void load();
}

/**
 * Reload after a manual date edit. Guarded on both fields being present and in
 * order so the intermediate state while someone is still picking the second date
 * does not fire a request the backend will refuse.
 */
function onManualRange() {
  preset.value = "custom";
  if (dateFrom.value && dateTo.value && dateFrom.value <= dateTo.value) void load();
}

// --- data -----------------------------------------------------------------

const report = ref<Report | null>(null);

const {
  loading,
  error: loadError,
  run: load,
} = useLoader(async () => {
  report.value = await api.getReport({ dateFrom: dateFrom.value, dateTo: dateTo.value });
});
onMounted(load);

// --- chart ----------------------------------------------------------------

/**
 * Axis labels. The full ISO key would be unreadable at day granularity and
 * redundant at year, so each granularity gets the shortest form that stays
 * unambiguous within the range.
 */
function tickLabel(period: string): string {
  if (period.length === 10) return `${period.slice(8, 10)}/${period.slice(5, 7)}`;
  if (period.length === 7) return `${period.slice(5, 7)}/${period.slice(0, 4)}`;
  return period;
}

const chartLabels = computed(() =>
  (report.value?.collections ?? []).map((p) => tickLabel(p.period)),
);

const chartSeries = computed<BarChartSeries[]>(() => [
  {
    label: t("rapports.chart.collected"),
    values: (report.value?.collections ?? []).map((p) => p.collected),
    color: "var(--success)",
  },
  {
    label: t("rapports.chart.due"),
    values: (report.value?.collections ?? []).map((p) => p.due),
    color: "var(--primary)",
  },
]);

/** Nothing to draw — every bar in every series is zero. */
const chartIsEmpty = computed(() =>
  (report.value?.collections ?? []).every((p) => p.collected === 0 && p.due === 0),
);

// --- tables ---------------------------------------------------------------

// The aging table is deliberately not sortable: its row order *is* information
// (least to most overdue), and letting a click scramble it would lose that.
const agingTotal = computed(() =>
  (report.value?.aging ?? []).reduce((sum, b) => sum + b.amount, 0),
);

function agingShare(amount: number): string {
  if (agingTotal.value <= 0) return "0%";
  return `${Math.round((amount / agingTotal.value) * 100)}%`;
}

const clientRows = computed(() => report.value?.topClients ?? []);
const { sort: clientSort, sorted: sortedClients } = useSort(clientRows, {
  client: (r) => r.clientName,
  outstanding: (r) => r.outstanding,
  overdue: (r) => r.overdue,
  overdueCount: (r) => r.overdueCount,
});

const productRows = computed(() => report.value?.topProducts ?? []);
const { sort: productSort, sorted: sortedProducts } = useSort(productRows, {
  product: (r) => r.productLabel,
  purchaseCount: (r) => r.purchaseCount,
  amount: (r) => r.totalAmount,
});

// --- export ---------------------------------------------------------------

/**
 * Download the report as CSV. Escaping and the formula-injection guard live in
 * `@/lib/csv`; this is only the browser plumbing, matching Impayés.
 */
async function exportCsv() {
  if (!report.value) return;
  const csv = buildReportCsv(report.value, {
    section: {
      totals: t("rapports.section.totals"),
      collections: t("rapports.section.collections"),
      aging: t("rapports.section.aging"),
      clients: t("rapports.section.clients"),
      products: t("rapports.section.products"),
    },
    figure: t("rapports.csv.figure"),
    value: t("rapports.csv.value"),
    totals: {
      range: t("rapports.csv.range"),
      asOf: t("rapports.asOf"),
      salesCount: t("rapports.kpi.salesCount"),
      salesAmount: t("rapports.kpi.sales"),
      collected: t("rapports.kpi.collected"),
      paymentCount: t("rapports.kpi.paymentCount"),
      outstandingNow: t("rapports.kpi.outstanding"),
      overdueNow: t("rapports.kpi.overdue"),
      newClients: t("rapports.kpi.newClients"),
    },
    period: t("rapports.columns.period"),
    collected: t("rapports.chart.collected"),
    due: t("rapports.chart.due"),
    bucket: t("rapports.columns.bucket"),
    count: t("rapports.columns.count"),
    amount: t("echeances.columns.amount"),
    client: t("impayes.columns.client"),
    outstanding: t("rapports.columns.outstanding"),
    overdue: t("rapports.columns.overdue"),
    overdueCount: t("rapports.columns.overdueCount"),
    product: t("rapports.columns.product"),
    purchaseCount: t("rapports.columns.purchaseCount"),
    agingBucket: Object.fromEntries(
      (report.value.aging ?? []).map((b) => [b.bucket, t(`rapports.aging.${b.bucket}`)]),
    ),
  });

  // Named for the range it covers, so successive exports don't overwrite.
  await saveCsv(`rapport-${dateFrom.value}-${dateTo.value}.csv`, csv);
}
</script>

<template>
  <div class="page">
    <LoadError v-if="loadError" :message="loadError" @retry="load" />

    <template v-else>
      <div class="card">
        <div class="card-header">
          <div>
            <h2>{{ t("rapports.title") }}</h2>
            <p class="subtitle">{{ t("rapports.subtitle") }}</p>
          </div>
          <button
            class="btn btn--ghost"
            type="button"
            :disabled="!report || loading"
            @click="exportCsv"
          >
            <AppIcon name="download" :size="16" />
            {{ t("rapports.export") }}
          </button>
        </div>

        <div class="range-bar">
          <div class="tabs">
            <button
              v-for="p in PRESETS"
              :key="p"
              class="tab"
              :class="{ 'tab--active': preset === p }"
              type="button"
              @click="applyPreset(p)"
            >
              {{ t(`rapports.preset.${p}`) }}
            </button>
          </div>
          <div class="range-dates">
            <label class="range-field">
              <span>{{ t("filters.from") }}</span>
              <DatePicker v-model="dateFrom" :max="dateTo" @update:model-value="onManualRange" />
            </label>
            <label class="range-field">
              <span>{{ t("filters.to") }}</span>
              <DatePicker v-model="dateTo" :min="dateFrom" @update:model-value="onManualRange" />
            </label>
          </div>
        </div>
      </div>

      <template v-if="report">
        <!-- Period figures: what happened between the two dates. -->
        <section class="kpi-grid" :aria-label="t('rapports.section.totals')">
          <KpiCard
            icon="cart"
            tone="blue"
            :label="t('rapports.kpi.sales')"
            :value="fmt.money(report.totals.salesAmount)"
            :sub="t('rapports.kpi.salesSub', { count: report.totals.salesCount })"
          />
          <KpiCard
            icon="banknote"
            tone="green"
            :label="t('rapports.kpi.collected')"
            :value="fmt.money(report.totals.collected)"
            :sub="t('rapports.kpi.collectedSub', { count: report.totals.paymentCount })"
          />
          <KpiCard
            icon="card"
            tone="purple"
            :label="t('rapports.kpi.outstanding')"
            :value="fmt.money(report.totals.outstandingNow)"
            :sub="t('rapports.asOfDate', { date: fmt.date(report.range.asOf) })"
          />
          <KpiCard
            icon="alert"
            tone="red"
            :label="t('rapports.kpi.overdue')"
            :value="fmt.money(report.totals.overdueNow)"
            :sub="t('rapports.asOfDate', { date: fmt.date(report.range.asOf) })"
          />
          <KpiCard
            icon="users"
            tone="orange"
            :label="t('rapports.kpi.newClients')"
            :value="fmt.number(report.totals.newClients)"
            :sub="t('rapports.kpi.inPeriod')"
          />
        </section>

        <div class="card">
          <div class="card-header">
            <div>
              <h2>{{ t("rapports.section.collections") }}</h2>
              <p class="subtitle">
                {{
                  t("rapports.chart.subtitle", {
                    from: fmt.date(report.range.from),
                    to: fmt.date(report.range.to),
                  })
                }}
              </p>
            </div>
          </div>
          <EmptyState
            v-if="chartIsEmpty"
            icon="report"
            :title="t('rapports.empty.title')"
            :hint="t('rapports.empty.text')"
          />
          <BarChart
            v-else
            :labels="chartLabels"
            :series="chartSeries"
            :title="t('rapports.section.collections')"
            :format="fmt.money"
          />
        </div>

        <!-- Balance figures: a snapshot, not a period. -->
        <div class="card">
          <div class="card-header">
            <div>
              <h2>{{ t("rapports.section.aging") }}</h2>
              <p class="subtitle">
                {{ t("rapports.asOfDate", { date: fmt.date(report.range.asOf) }) }}
              </p>
            </div>
          </div>
          <div class="table-scroll">
            <table class="table">
              <thead>
                <tr>
                  <th>{{ t("rapports.columns.bucket") }}</th>
                  <th class="ta-end">{{ t("rapports.columns.count") }}</th>
                  <th class="ta-end">{{ t("echeances.columns.amount") }}</th>
                  <th class="ta-end">{{ t("rapports.columns.share") }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="b in report.aging" :key="b.bucket">
                  <td>{{ t(`rapports.aging.${b.bucket}`) }}</td>
                  <td class="ta-end tabular">{{ fmt.number(b.count) }}</td>
                  <td class="ta-end tabular">{{ fmt.money(b.amount) }}</td>
                  <td class="ta-end tabular">{{ agingShare(b.amount) }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <div class="split">
          <div class="card">
            <div class="card-header">
              <div>
                <h2>{{ t("rapports.section.clients") }}</h2>
                <p class="subtitle">
                  {{ t("rapports.asOfDate", { date: fmt.date(report.range.asOf) }) }}
                </p>
              </div>
            </div>
            <EmptyState
              v-if="sortedClients.length === 0"
              icon="users"
              :title="t('rapports.empty.clients')"
            />
            <div v-else class="table-scroll">
              <table class="table">
                <thead>
                  <tr>
                    <SortHeader
                      :sort="clientSort"
                      field="client"
                      :label="t('impayes.columns.client')"
                    />
                    <SortHeader
                      :sort="clientSort"
                      field="outstanding"
                      :label="t('rapports.columns.outstanding')"
                      align="end"
                    />
                    <SortHeader
                      :sort="clientSort"
                      field="overdue"
                      :label="t('rapports.columns.overdue')"
                      align="end"
                    />
                    <SortHeader
                      :sort="clientSort"
                      field="overdueCount"
                      :label="t('rapports.columns.overdueCount')"
                      align="end"
                    />
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="c in sortedClients" :key="c.clientId">
                    <td>
                      <RouterLink class="row-link" :to="`/clients/${c.clientId}`">
                        {{ c.clientName }}
                      </RouterLink>
                    </td>
                    <td class="ta-end tabular">{{ fmt.money(c.outstanding) }}</td>
                    <td class="ta-end tabular danger">{{ fmt.money(c.overdue) }}</td>
                    <td class="ta-end tabular">{{ fmt.number(c.overdueCount) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>

          <div class="card">
            <div class="card-header">
              <div>
                <h2>{{ t("rapports.section.products") }}</h2>
                <p class="subtitle">{{ t("rapports.kpi.inPeriod") }}</p>
              </div>
            </div>
            <EmptyState
              v-if="sortedProducts.length === 0"
              icon="cart"
              :title="t('rapports.empty.products')"
            />
            <div v-else class="table-scroll">
              <table class="table">
                <thead>
                  <tr>
                    <SortHeader
                      :sort="productSort"
                      field="product"
                      :label="t('rapports.columns.product')"
                    />
                    <SortHeader
                      :sort="productSort"
                      field="purchaseCount"
                      :label="t('rapports.columns.purchaseCount')"
                      align="end"
                    />
                    <SortHeader
                      :sort="productSort"
                      field="amount"
                      :label="t('echeances.columns.amount')"
                      align="end"
                    />
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="p in sortedProducts" :key="p.productLabel">
                    <td>{{ p.productLabel }}</td>
                    <td class="ta-end tabular">{{ fmt.number(p.purchaseCount) }}</td>
                    <td class="ta-end tabular">{{ fmt.money(p.totalAmount) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </template>
    </template>
  </div>
</template>

<style scoped>
.range-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-4);
  padding: 0 var(--space-5) var(--space-5);
}
.range-dates {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
}
.range-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  font-size: 12px;
  color: var(--text-secondary);
  font-weight: 500;
}

.kpi-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: var(--space-4);
}

.split {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(380px, 1fr));
  gap: var(--space-4);
  align-items: start;
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

/* Logical end, so the numeric columns stay on the correct side in Arabic. */
.ta-end {
  text-align: end;
}

.danger {
  color: var(--danger-strong);
}

@media (max-width: 900px) {
  .range-bar {
    align-items: stretch;
  }
}
</style>
