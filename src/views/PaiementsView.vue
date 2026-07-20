<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import ListFilterBar from "@/components/ui/ListFilterBar.vue";
import { useFormat } from "@/composables/useFormat";
import { useSort } from "@/composables/useSort";
import { api } from "@/api";
import type { Payment } from "@/types/models";

const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();
const payments = ref<Payment[]>([]);
const loading = ref(true);

const search = ref("");
const amountMin = ref("");
const amountMax = ref("");
const dateFrom = ref("");
const dateTo = ref("");

const filtered = computed(() => {
  const needle = search.value.trim().toLowerCase();
  const min = amountMin.value === "" ? null : Number(amountMin.value);
  const max = amountMax.value === "" ? null : Number(amountMax.value);
  return payments.value.filter((p) => {
    if (needle && !`${p.purchaseReference} ${p.clientName}`.toLowerCase().includes(needle)) return false;
    if (min != null && p.amount < min) return false;
    if (max != null && p.amount > max) return false;
    if (dateFrom.value && p.paymentDate < dateFrom.value) return false;
    if (dateTo.value && p.paymentDate > dateTo.value) return false;
    return true;
  });
});

const { sort, sorted } = useSort(filtered, {
  date: (p) => p.paymentDate,
  reference: (p) => p.purchaseReference,
  client: (p) => p.clientName,
  tranche: (p) => p.installmentIndex,
  amount: (p) => p.amount,
  note: (p) => p.note,
});

onMounted(async () => {
  payments.value = await api.listAllPayments();
  loading.value = false;
});
</script>

<template>
  <div class="page">
    <div class="card">
      <div class="card-header"><h2>{{ t("paiements.title") }}</h2></div>
      <ListFilterBar
        v-model:search="search"
        v-model:amount-min="amountMin"
        v-model:amount-max="amountMax"
        v-model:date-from="dateFrom"
        v-model:date-to="dateTo"
        show-amount
      />
      <p class="partial-note">{{ t("paiements.partialInfo") }}</p>
      <EmptyState v-if="!loading && filtered.length === 0" icon="card" :title="t('paiements.empty')" />
      <table v-else class="table">
        <thead>
          <tr>
            <SortHeader :sort="sort" field="date" :label="t('paiements.columns.date')" />
            <SortHeader :sort="sort" field="reference" :label="t('paiements.columns.reference')" />
            <SortHeader :sort="sort" field="client" :label="t('common.client')" />
            <SortHeader :sort="sort" field="tranche" :label="t('paiements.columns.tranche')" />
            <SortHeader :sort="sort" field="amount" :label="t('paiements.columns.amount')" />
            <SortHeader :sort="sort" field="note" :label="t('paiements.columns.note')" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="pay in sorted" :key="pay.id">
            <td class="tabular">{{ fmt.date(pay.paymentDate) }}</td>
            <td>
              <a class="row-link" href="#" @click.prevent="router.push({ name: 'achat-detail', params: { id: pay.purchaseId } })">
                {{ pay.purchaseReference }}
              </a>
            </td>
            <td>{{ pay.clientName }}</td>
            <td class="tabular">{{ pay.installmentIndex }}</td>
            <td class="tabular strong">{{ fmt.money(pay.amount) }}</td>
            <td class="muted">{{ pay.note || "—" }}</td>
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
.partial-note {
  padding: 0 22px 12px;
  font-size: 12.5px;
  color: var(--text-muted);
}
</style>
