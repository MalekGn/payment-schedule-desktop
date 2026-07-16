<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import EmptyState from "@/components/ui/EmptyState.vue";
import { useFormat } from "@/composables/useFormat";
import { api } from "@/api";
import type { Payment } from "@/types/models";

const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();
const payments = ref<Payment[]>([]);
const loading = ref(true);

onMounted(async () => {
  payments.value = await api.listAllPayments();
  loading.value = false;
});
</script>

<template>
  <div class="page">
    <div class="card">
      <div class="card-header"><h2>{{ t("paiements.title") }}</h2></div>
      <p class="partial-note">{{ t("paiements.partialInfo") }}</p>
      <EmptyState v-if="!loading && payments.length === 0" icon="card" :title="t('paiements.empty')" />
      <table v-else class="table">
        <thead>
          <tr>
            <th>{{ t("paiements.columns.date") }}</th>
            <th>{{ t("paiements.columns.reference") }}</th>
            <th>{{ t("paiements.columns.tranche") }}</th>
            <th>{{ t("paiements.columns.amount") }}</th>
            <th>{{ t("paiements.columns.note") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="pay in payments" :key="pay.id">
            <td class="tabular">{{ fmt.date(pay.paymentDate) }}</td>
            <td>
              <a class="row-link" href="#" @click.prevent="router.push({ name: 'achat-detail', params: { id: pay.purchaseId } })">
                {{ pay.purchaseReference }}
              </a>
            </td>
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
