<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import { useFormat } from "@/composables/useFormat";
import { useSortState, sortRows } from "@/composables/useSort";
import { api } from "@/api";
import type { ClientSummary, ImpayeClient, OverdueInstallment } from "@/types/models";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const fmt = useFormat();

const impayes = ref<ImpayeClient[]>([]);
const clients = ref<ClientSummary[]>([]);
const loading = ref(true);

const dateFrom = ref("");
const dateTo = ref("");
const clientId = ref<string>((route.query.client as string) ?? "");

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

async function load() {
  loading.value = true;
  impayes.value = await api.listImpayes({
    dateFrom: dateFrom.value || null,
    dateTo: dateTo.value || null,
    clientId: clientId.value ? Number(clientId.value) : null,
  });
  loading.value = false;
}

onMounted(async () => {
  clients.value = await api.listClients();
  await load();
});

function resetFilters() {
  dateFrom.value = "";
  dateTo.value = "";
  clientId.value = "";
  load();
}

const tel = (phone: string) => `tel:${phone.replace(/\s/g, "")}`;
const sms = (phone: string) => `sms:${phone.replace(/\s/g, "")}`;

function exportCsv() {
  const header = ["Client", "Téléphone", "N° Achat", "Tranche", "Échéance", "Montant", "Jours de retard"];
  const lines = [header.join(",")];
  for (const c of impayes.value) {
    for (const i of c.installments) {
      lines.push(
        [
          `"${c.clientName}"`,
          `"${c.phone}"`,
          i.purchaseReference,
          `${i.index}/${i.installmentCount}`,
          i.dueDate,
          i.remaining,
          i.daysLate,
        ].join(","),
      );
    }
  }
  const blob = new Blob(["﻿" + lines.join("\n")], { type: "text/csv;charset=utf-8;" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "impayes.csv";
  a.click();
  URL.revokeObjectURL(url);
}
</script>

<template>
  <div class="page">
    <div class="card filter-card">
      <div class="filters">
        <div class="field">
          <label>{{ t("impaye.dateRange") }}</label>
          <div class="range-inputs">
            <input v-model="dateFrom" type="date" class="input" />
            <span class="range-sep">–</span>
            <input v-model="dateTo" type="date" class="input" />
          </div>
        </div>
        <div class="field">
          <label>{{ t("common.client") }}</label>
          <select v-model="clientId" class="select">
            <option value="">{{ t("impaye.allClients") }}</option>
            <option v-for="c in clients" :key="c.id" :value="String(c.id)">
              {{ c.firstName }} {{ c.lastName }}
            </option>
          </select>
        </div>
        <div class="filter-actions">
          <button class="btn btn--primary" type="button" @click="load">{{ t("common.filter") }}</button>
          <button class="btn btn--ghost" type="button" @click="resetFilters">{{ t("common.all") }}</button>
          <button v-if="impayes.length" class="btn btn--ghost" type="button" @click="exportCsv">
            <AppIcon name="download" :size="16" /> {{ t("common.export") }}
          </button>
        </div>
      </div>
    </div>

    <div v-if="!loading && impayes.length === 0" class="card">
      <EmptyState :title="t('impayes.empty')" :hint="t('impayes.emptyHint')" />
    </div>

    <div v-else class="impaye-cards">
      <section v-for="c in impayes" :key="c.clientId" class="card impaye-card">
        <div class="impaye-head">
          <div class="impaye-who">
            <span class="impaye-name">{{ c.clientName }}</span>
            <span class="impaye-contact">
              <AppIcon name="phone" :size="13" /> {{ c.phone }}
              <template v-if="c.address"><span class="sep">·</span><AppIcon name="map-pin" :size="13" /> {{ c.address }}</template>
            </span>
          </div>
          <div class="impaye-right">
            <div class="impaye-total-box">
              <span class="impaye-total-label">{{ t("impayes.totalOverdue") }}</span>
              <span class="impaye-total tabular">{{ fmt.money(c.totalOverdue) }}</span>
            </div>
            <div class="impaye-actions">
              <a class="contact-btn contact-btn--call" :href="tel(c.phone)" :title="t('impaye.call')">
                <AppIcon name="phone" :size="17" />
              </a>
              <a class="contact-btn contact-btn--msg" :href="sms(c.phone)" :title="t('impaye.message')">
                <AppIcon name="message" :size="17" />
              </a>
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
              <SortHeader :sort="sort" field="reference" :label="t('echeances.columns.reference')" />
              <SortHeader :sort="sort" field="tranche" :label="t('echeances.columns.tranche')" />
              <SortHeader :sort="sort" field="dueDate" :label="t('echeances.columns.dueDate')" />
              <SortHeader :sort="sort" field="amount" :label="t('echeances.columns.amount')" />
              <SortHeader :sort="sort" field="since" :label="t('impaye.since')" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="i in sortedInstallments(c.installments)" :key="i.installmentId" class="is-late">
              <td>
                <a class="row-link" href="#" @click.prevent="router.push({ name: 'achat-detail', params: { id: i.purchaseId } })">
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
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.filter-card {
  padding: 18px 22px;
}
.filters {
  display: flex;
  gap: 18px;
  align-items: flex-end;
  flex-wrap: wrap;
}
.range-inputs {
  display: flex;
  align-items: center;
  gap: 8px;
}
.range-sep {
  color: var(--text-muted);
}
.filter-actions {
  display: flex;
  gap: 10px;
  margin-inline-start: auto;
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
