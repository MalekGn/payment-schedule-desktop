<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import ListFilterBar from "@/components/ui/ListFilterBar.vue";
import LoadError from "@/components/ui/LoadError.vue";
import { buildImpayesCsv } from "@/lib/csv";
import { todayIso } from "@/lib/finance";
import { useFormat } from "@/composables/useFormat";
import { useLoader } from "@/composables/useLoader";
import { useSortState, sortRows } from "@/composables/useSort";
import { useContactActions } from "@/composables/useContactActions";
import { api } from "@/api";
import type { ImpayeClient, OverdueInstallment } from "@/types/models";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const fmt = useFormat();

const impayes = ref<ImpayeClient[]>([]);

// Same filter controls as Payments and Due dates (shared ListFilterBar), applied
// reactively client-side: search matches reference + client, amount min/max maps
// to the overdue remaining, and the From/To range filters on the due date.
const search = ref("");
const amountMin = ref("");
const amountMax = ref("");
const dateFrom = ref("");
const dateTo = ref("");

// One shared sort applied to every client's overdue-installments table.
const sort = useSortState();
const instAccessors = {
  reference: (i: OverdueInstallment) => i.purchaseReference,
  tranche: (i: OverdueInstallment) => i.index,
  dueDate: (i: OverdueInstallment) => i.dueDate,
  amount: (i: OverdueInstallment) => i.remaining,
  since: (i: OverdueInstallment) => i.daysLate,
};
const sortedInstallments = (rows: OverdueInstallment[]) => sortRows(rows, instAccessors, sort);

// Filter each client's overdue installments, drop clients left with none, and
// recompute their totals so the per-client header stays accurate.
const filtered = computed<ImpayeClient[]>(() => {
  const needle = search.value.trim().toLowerCase();
  const min = amountMin.value === "" ? null : Number(amountMin.value);
  const max = amountMax.value === "" ? null : Number(amountMax.value);
  const out: ImpayeClient[] = [];
  for (const c of impayes.value) {
    const installments = c.installments.filter((i) => {
      if (needle && !`${i.purchaseReference} ${c.clientName}`.toLowerCase().includes(needle))
        return false;
      if (min != null && i.remaining < min) return false;
      if (max != null && i.remaining > max) return false;
      if (dateFrom.value && i.dueDate < dateFrom.value) return false;
      if (dateTo.value && i.dueDate > dateTo.value) return false;
      return true;
    });
    if (installments.length === 0) continue;
    out.push({
      ...c,
      installments,
      totalOverdue: installments.reduce((s, i) => s + i.remaining, 0),
      overdueCount: installments.length,
    });
  }
  return out;
});

const {
  loading,
  error: loadError,
  run: load,
} = useLoader(async () => {
  impayes.value = await api.listImpayes();
  // Deep-link from the dashboard overdue panel: pre-fill the search with the
  // client's name so the unified search filters down to them.
  const qid = route.query.client ? Number(route.query.client) : null;
  if (qid) {
    const match = impayes.value.find((c) => c.clientId === qid);
    if (match) search.value = match.clientName;
  }
});
onMounted(load);

const contact = useContactActions();

/**
 * Download the filtered overdue list as a spreadsheet.
 *
 * Escaping and formula-injection defence live in `@/lib/csv`; this only does
 * the browser plumbing. Headers reuse the same keys as the on-screen table
 * above — the file used to be hard-coded French, so an Arabic or English user
 * got a localized UI and a French export.
 */
function exportCsv() {
  const csv = buildImpayesCsv(filtered.value, {
    client: t("impayes.columns.client"),
    phone: t("clients.columns.phone"),
    reference: t("echeances.columns.reference"),
    installment: t("echeances.columns.tranche"),
    dueDate: t("echeances.columns.dueDate"),
    amount: t("echeances.columns.amount"),
    daysLate: t("impaye.since"),
  });

  const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  // Dated so successive exports don't silently overwrite one another.
  a.download = `impayes-${todayIso()}.csv`;
  a.click();
  URL.revokeObjectURL(url);
}
</script>

<template>
  <div class="page">
    <LoadError v-if="loadError" :message="loadError" @retry="load" />

    <template v-else>
      <div class="card">
        <div class="card-header">
          <div>
            <h2>{{ t("impayes.title") }}</h2>
            <p class="subtitle">{{ t("impayes.subtitle") }}</p>
          </div>
          <button v-if="filtered.length" class="btn btn--ghost" type="button" @click="exportCsv">
            <AppIcon name="download" :size="16" /> {{ t("common.export") }}
          </button>
        </div>
        <ListFilterBar
          v-model:search="search"
          v-model:amount-min="amountMin"
          v-model:amount-max="amountMax"
          v-model:date-from="dateFrom"
          v-model:date-to="dateTo"
          show-amount
        />
      </div>

      <div v-if="!loading && filtered.length === 0" class="card">
        <EmptyState :title="t('impayes.empty')" :hint="t('impayes.emptyHint')" />
      </div>

      <div v-else class="impaye-cards">
        <section v-for="c in filtered" :key="c.clientId" class="card impaye-card">
          <div class="impaye-head">
            <div class="impaye-who">
              <span class="impaye-name">{{ c.clientName }}</span>
              <span class="impaye-contact">
                <AppIcon name="phone" :size="13" /> {{ c.phone }}
                <template v-if="c.address"
                  ><span class="sep">·</span><AppIcon name="map-pin" :size="13" />
                  {{ c.address }}</template
                >
              </span>
            </div>
            <div class="impaye-right">
              <div class="impaye-total-box">
                <span class="impaye-total-label">{{ t("impayes.totalOverdue") }}</span>
                <span class="impaye-total tabular">{{ fmt.money(c.totalOverdue) }}</span>
              </div>
              <div class="impaye-actions">
                <button
                  class="contact-btn contact-btn--call"
                  type="button"
                  :title="t('impaye.call')"
                  @click="contact.call(c.phone)"
                >
                  <AppIcon name="phone" :size="17" />
                </button>
                <button
                  class="contact-btn contact-btn--msg"
                  type="button"
                  :title="t('impaye.message')"
                  @click="contact.message(c.phone)"
                >
                  <AppIcon name="message" :size="17" />
                </button>
                <button
                  class="contact-btn contact-btn--view"
                  type="button"
                  :title="t('common.view')"
                  @click="router.push({ name: 'client-detail', params: { id: c.clientId } })"
                >
                  <AppIcon name="users" :size="17" />
                </button>
              </div>
            </div>
          </div>

          <table class="table inner-table">
            <thead>
              <tr>
                <SortHeader
                  :sort="sort"
                  field="reference"
                  :label="t('echeances.columns.reference')"
                />
                <SortHeader :sort="sort" field="tranche" :label="t('echeances.columns.tranche')" />
                <SortHeader :sort="sort" field="dueDate" :label="t('echeances.columns.dueDate')" />
                <SortHeader :sort="sort" field="amount" :label="t('echeances.columns.amount')" />
                <SortHeader :sort="sort" field="since" :label="t('impaye.since')" />
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="i in sortedInstallments(c.installments)"
                :key="i.installmentId"
                class="is-late"
              >
                <td>
                  <a
                    class="row-link"
                    href="#"
                    @click.prevent="
                      router.push({ name: 'achat-detail', params: { id: i.purchaseId } })
                    "
                  >
                    {{ i.purchaseReference }}
                  </a>
                </td>
                <td class="tabular">{{ i.index }}/{{ i.installmentCount }}</td>
                <td class="tabular">{{ fmt.date(i.dueDate) }}</td>
                <td class="tabular strong">{{ fmt.money(i.remaining) }}</td>
                <td class="late-cell">{{ t("dashboard.alert.daysLate", i.daysLate) }}</td>
              </tr>
            </tbody>
          </table>
        </section>
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
.impaye-cards {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.impaye-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  padding: 18px 22px;
  gap: 16px;
  flex-wrap: wrap;
}
.impaye-who {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.impaye-name {
  font-size: 16px;
  font-weight: 700;
}
.impaye-contact {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-secondary);
}
.impaye-contact :deep(.app-icon) {
  color: var(--text-muted);
}
.sep {
  margin: 0 6px;
  color: var(--text-muted);
}
.impaye-right {
  display: flex;
  align-items: center;
  gap: 18px;
}
.impaye-total-box {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
}
.impaye-total-label {
  font-size: 11.5px;
  color: var(--text-muted);
}
.impaye-total {
  font-size: 18px;
  font-weight: 700;
  color: var(--danger-strong);
}
.impaye-actions {
  display: flex;
  gap: 8px;
}
.contact-btn {
  width: 38px;
  height: 34px;
  border-radius: 9px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
}
.contact-btn--call {
  background: #e7f8ef;
  color: var(--success);
  border-color: #c7efd9;
}
.contact-btn--msg {
  background: var(--primary-soft);
  color: var(--primary);
  border-color: #d3e2fd;
}
.contact-btn--view {
  background: var(--bg);
  color: var(--text-secondary);
  border-color: var(--border-strong);
}
.inner-table {
  border-top: 1px solid var(--border);
}
.inner-table th {
  background: #fafbfc;
}
.late-cell {
  color: var(--danger-strong);
  font-weight: 600;
  font-size: 13px;
}
</style>
