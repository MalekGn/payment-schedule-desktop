<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import PurchaseDetailCard from "@/components/dashboard/PurchaseDetailCard.vue";
import EditInstallmentModal from "@/components/EditInstallmentModal.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import { useFormat } from "@/composables/useFormat";
import { useSort } from "@/composables/useSort";
import { useBack } from "@/composables/useBack";
import { useUiStore } from "@/stores/ui";
import { api } from "@/api";
import type { Installment, Payment, PurchaseDetail } from "@/types/models";

const props = defineProps<{ id: string }>();
const { t } = useI18n();
const fmt = useFormat();
const ui = useUiStore();
const goBack = useBack("/achats");

const detail = ref<PurchaseDetail | null>(null);
const payments = ref<Payment[]>([]);
const editTarget = ref<Installment | null>(null);
const notFound = ref(false);

const { sort, sorted: sortedPayments } = useSort(payments, {
  date: (p) => p.paymentDate,
  tranche: (p) => p.installmentIndex,
  amount: (p) => p.amount,
  note: (p) => p.note,
});

async function load() {
  const pid = Number(props.id);
  try {
    detail.value = await api.getPurchaseDetail(pid);
    payments.value = await api.listPaymentsForPurchase(pid);
    ui.pageTitle = detail.value.purchase.reference;
  } catch {
    notFound.value = true;
  }
}
onMounted(load);

async function onSaved(updated: PurchaseDetail) {
  editTarget.value = null;
  detail.value = updated;
  payments.value = await api.listPaymentsForPurchase(updated.purchase.id);
}
</script>

<template>
  <div v-if="notFound" class="page">
    <button class="back-link" type="button" @click="goBack">
      <AppIcon name="arrow-left" :size="16" class="icon-flip" /> {{ t("common.back") }}
    </button>
    <div class="card">
      <EmptyState icon="cart" :title="t('notFound.purchaseMissing')" />
    </div>
  </div>

  <div v-else-if="detail" class="page">
    <button class="back-link" type="button" @click="goBack">
      <AppIcon name="arrow-left" :size="16" class="icon-flip" /> {{ t("common.back") }}
    </button>

    <!-- Reachable by URL or from the archive tab, so an archived purchase
         must not read as live. Actions stay on the Achats list. -->
    <div v-if="detail.purchase.archivedAt" class="archived-banner" role="status">
      <AppIcon name="archive" :size="18" />
      <span>{{ t("achats.archivedOn", { date: fmt.date(detail.purchase.archivedAt) }) }}</span>
    </div>
    <PurchaseDetailCard :detail="detail" full-actions @update-installment="editTarget = $event" />

    <section class="card">
      <div class="card-header">
        <h2>{{ t("dashboard.detail.paymentHistory") }}</h2>
      </div>
      <EmptyState v-if="payments.length === 0" icon="card" :title="t('paiements.empty')" />
      <div v-else class="table-scroll">
        <table class="table">
          <thead>
            <tr>
              <SortHeader :sort="sort" field="date" :label="t('paiements.columns.date')" />
              <SortHeader :sort="sort" field="tranche" :label="t('paiements.columns.tranche')" />
              <SortHeader :sort="sort" field="amount" :label="t('paiements.columns.amount')" />
              <SortHeader :sort="sort" field="note" :label="t('paiements.columns.note')" />
            </tr>
          </thead>
          <tbody>
            <tr v-for="pay in sortedPayments" :key="pay.id">
              <td class="tabular">{{ fmt.date(pay.paymentDate) }}</td>
              <td class="tabular">
                {{ pay.installmentIndex }}/{{ detail.purchase.installmentCount }}
              </td>
              <td class="tabular strong">{{ fmt.money(pay.amount) }}</td>
              <td class="muted">{{ pay.note || "—" }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <EditInstallmentModal
      v-if="editTarget"
      :installment="editTarget"
      :siblings="detail.installments"
      :installment-count="detail.purchase.installmentCount"
      :purchase-reference="detail.purchase.reference"
      @close="editTarget = null"
      @saved="onSaved"
    />
  </div>
</template>

<style scoped>
.archived-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-radius: 10px;
  background: var(--neutral-bg);
  color: var(--text-secondary);
  font-weight: 600;
}
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
