<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import PurchaseDetailCard from "@/components/dashboard/PurchaseDetailCard.vue";
import PaymentModal from "@/components/PaymentModal.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import { useFormat } from "@/composables/useFormat";
import { useUiStore } from "@/stores/ui";
import { api } from "@/api";
import type { Installment, Payment, PurchaseDetail } from "@/types/models";

const props = defineProps<{ id: string }>();
const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();
const ui = useUiStore();

const detail = ref<PurchaseDetail | null>(null);
const payments = ref<Payment[]>([]);
const payTarget = ref<Installment | null>(null);

async function load() {
  const pid = Number(props.id);
  detail.value = await api.getPurchaseDetail(pid);
  payments.value = await api.listPaymentsForPurchase(pid);
  ui.pageTitle = detail.value.purchase.reference;
}
onMounted(load);

async function onSaved(updated: PurchaseDetail) {
  payTarget.value = null;
  detail.value = updated;
  payments.value = await api.listPaymentsForPurchase(updated.purchase.id);
}
</script>

<template>
  <div v-if="detail" class="page">
    <button class="back-link" type="button" @click="router.push('/achats')">
      <AppIcon name="arrow-left" :size="16" /> {{ t("nav.achats") }}
    </button>

    <PurchaseDetailCard :detail="detail" full-actions @pay="payTarget = $event" />

    <section class="card">
      <div class="card-header"><h2>{{ t("dashboard.detail.paymentHistory") }}</h2></div>
      <EmptyState v-if="payments.length === 0" icon="card" :title="t('paiements.empty')" />
      <table v-else class="table">
        <thead>
          <tr>
            <th>{{ t("paiements.columns.date") }}</th>
            <th>{{ t("paiements.columns.tranche") }}</th>
            <th>{{ t("paiements.columns.amount") }}</th>
            <th>{{ t("paiements.columns.note") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="pay in payments" :key="pay.id">
            <td class="tabular">{{ fmt.date(pay.paymentDate) }}</td>
            <td class="tabular">{{ pay.installmentIndex }}/{{ detail.purchase.installmentCount }}</td>
            <td class="tabular strong">{{ fmt.money(pay.amount) }}</td>
            <td class="muted">{{ pay.note || "—" }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <PaymentModal
      v-if="payTarget"
      :installment="payTarget"
      :installment-count="detail.purchase.installmentCount"
      :purchase-reference="detail.purchase.reference"
      @close="payTarget = null"
      @saved="onSaved"
    />
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.back-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: transparent;
  border: none;
  color: var(--text-secondary);
  font-weight: 600;
  font-size: 13.5px;
  align-self: flex-start;
}
.back-link:hover {
  color: var(--primary);
}
</style>
