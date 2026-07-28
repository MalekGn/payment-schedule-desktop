<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import { useFormat } from "@/composables/useFormat";
import { useSort } from "@/composables/useSort";
import type { Installment, PurchaseDetail } from "@/types/models";

const props = defineProps<{
  detail: PurchaseDetail;
  // When true, every installment gets the update action (purchase detail page).
  // When false (dashboard), only actionable (late/partial) ones do.
  fullActions?: boolean;
  showHeader?: boolean;
}>();
const emit = defineEmits<{ updateInstallment: [installment: Installment] }>();

const { t } = useI18n();
const fmt = useFormat();
const router = useRouter();

const { sort, sorted: sortedInstallments } = useSort(() => props.detail.installments, {
  tranche: (i) => i.index,
  dueDate: (i) => i.dueDate,
  amount: (i) => i.amount,
  remaining: (i) => i.amount - i.paidAmount,
  status: (i) => i.status,
  paymentDate: (i) => i.paidDate,
});

/**
 * Whether this row offers the update action.
 *
 * `update_installment` refuses an archived purchase outright, so offering it
 * there is a button that can only ever produce an error toast. On the purchase
 * page every row qualifies — including settled ones, whose collected figure and
 * payment date stay editable. The dashboard card is a preview, so it keeps to
 * the rows that actually need attention.
 */
function canUpdate(i: Installment): boolean {
  if (props.detail.purchase.archivedAt) return false;
  if (props.fullActions) return true;
  return i.status === "late" || i.status === "partial";
}

function goToClient() {
  router.push({ name: "client-detail", params: { id: props.detail.client.id } });
}
function goToPurchase() {
  router.push({ name: "achat-detail", params: { id: props.detail.purchase.id } });
}
</script>

<template>
  <section class="card">
    <div v-if="showHeader !== false" class="card-header">
      <h2>{{ t("dashboard.purchaseDetail") }}</h2>
      <StatusBadge :status="detail.status" />
    </div>

    <div class="detail-top">
      <div class="detail-ident">
        <div class="product-thumb">
          <AppIcon name="washer" :size="42" :stroke-width="1.6" />
        </div>
        <div class="ident-text">
          <span class="ident-ref">{{ detail.purchase.reference }}</span>
          <span class="ident-product">{{ detail.purchase.productLabel }}</span>
          <span class="ident-client-label">{{ t("dashboard.detail.client") }}</span>
          <a class="ident-client" href="#" @click.prevent="goToClient">
            {{ detail.client.firstName }} {{ detail.client.lastName }}
          </a>
          <span class="ident-line"
            ><AppIcon name="phone" :size="14" /> {{ detail.client.phone }}</span
          >
          <span class="ident-line"
            ><AppIcon name="map-pin" :size="14" /> {{ detail.client.address }}</span
          >
          <span v-if="detail.client.email" class="ident-line">
            <AppIcon name="mail" :size="14" /> {{ detail.client.email }}
          </span>
        </div>
      </div>

      <div class="detail-metrics">
        <div class="metric">
          <span class="metric-label">{{ t("dashboard.detail.purchaseDate") }}</span>
          <span class="metric-value tabular">{{ fmt.date(detail.purchase.purchaseDate) }}</span>
        </div>
        <div class="metric">
          <span class="metric-label">{{ t("dashboard.detail.installmentCount") }}</span>
          <span class="metric-value tabular">{{ detail.purchase.installmentCount }}</span>
        </div>
        <div class="metric">
          <span class="metric-label">{{ t("dashboard.detail.totalAmount") }}</span>
          <span class="metric-value tabular">{{ fmt.money(detail.purchase.totalPrice) }}</span>
        </div>
        <div class="metric">
          <span class="metric-label">{{ t("dashboard.detail.paidAmount") }}</span>
          <span class="metric-value tabular">{{ fmt.money(detail.totalPaid) }}</span>
        </div>
        <div class="remaining-box">
          <span class="remaining-label">{{ t("dashboard.detail.remainingToPay") }}</span>
          <span class="remaining-value tabular">{{ fmt.money(detail.remaining) }}</span>
        </div>
      </div>
    </div>

    <table class="table inst-table">
      <thead>
        <tr>
          <SortHeader :sort="sort" field="tranche" :label="t('dashboard.detail.tranche')" />
          <SortHeader :sort="sort" field="dueDate" :label="t('dashboard.detail.dueDate')" />
          <SortHeader :sort="sort" field="amount" :label="t('dashboard.detail.amount')" />
          <SortHeader :sort="sort" field="remaining" :label="t('common.remaining')" />
          <SortHeader :sort="sort" field="status" :label="t('common.status')" />
          <SortHeader :sort="sort" field="paymentDate" :label="t('dashboard.detail.paymentDate')" />
          <th class="col-action">{{ t("common.actions") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="i in sortedInstallments" :key="i.id" :class="{ 'is-late': i.status === 'late' }">
          <td class="tabular">{{ i.index }}/{{ detail.purchase.installmentCount }}</td>
          <td class="tabular">{{ fmt.date(i.dueDate) }}</td>
          <td class="tabular">{{ fmt.money(i.amount) }}</td>
          <td class="tabular strong">{{ fmt.money(i.amount - i.paidAmount) }}</td>
          <td><StatusBadge :status="i.status" feminine /></td>
          <td class="tabular muted">{{ i.paidDate ? fmt.date(i.paidDate) : "—" }}</td>
          <td class="col-action">
            <div class="row-actions">
              <button
                v-if="canUpdate(i)"
                class="btn btn--primary btn--sm"
                type="button"
                @click="emit('updateInstallment', i)"
              >
                {{ t("dashboard.detail.updatePayment") }}
              </button>
              <a
                v-else-if="i.status === 'paid'"
                class="row-link"
                href="#"
                @click.prevent="goToPurchase"
              >
                {{ t("dashboard.detail.view") }}
              </a>
              <span v-else class="muted">—</span>
            </div>
          </td>
        </tr>
      </tbody>
    </table>

    <div class="detail-foot">
      <button class="btn btn--ghost btn--sm" type="button" @click="goToPurchase">
        <AppIcon name="clock" :size="16" />
        {{ t("dashboard.detail.paymentHistory") }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.detail-top {
  display: flex;
  justify-content: space-between;
  gap: 24px;
  padding: 4px 24px 18px;
  flex-wrap: wrap;
}
.detail-ident {
  display: flex;
  gap: 16px;
}
.product-thumb {
  width: 92px;
  height: 92px;
  border-radius: 12px;
  background: linear-gradient(135deg, #eef1f5, #e2e6ec);
  color: #8a94a6;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.ident-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.ident-ref {
  font-size: 15px;
  font-weight: 700;
  color: var(--text);
}
.ident-product {
  font-size: 13.5px;
  color: var(--text-secondary);
  margin-bottom: 6px;
}
.ident-client-label {
  font-size: 12px;
  color: var(--text-muted);
}
.ident-client {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--primary);
  margin-bottom: 3px;
}
.ident-line {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12.5px;
  color: var(--text-secondary);
}
.ident-line :deep(.app-icon) {
  color: var(--text-muted);
}
.detail-metrics {
  display: flex;
  align-items: flex-start;
  gap: 26px;
  flex-wrap: wrap;
}
.metric {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.metric-label {
  font-size: 12px;
  color: var(--text-muted);
}
.metric-value {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
}
.remaining-box {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 10px 16px;
  border: 1px solid var(--warning-border);
  background: var(--warning-bg);
  border-radius: 10px;
}
.remaining-label {
  font-size: 12px;
  color: var(--warning-text);
  font-weight: 500;
}
.remaining-value {
  font-size: 16px;
  font-weight: 700;
  color: var(--warning-text);
  white-space: nowrap;
}
.inst-table th {
  background: #fafbfc;
}
.col-action {
  text-align: end;
}
/* Flex rather than inline spacing so the pair mirrors under `dir="rtl"`. */
.row-actions {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 6px;
}
.detail-foot {
  padding: 16px 24px;
}
</style>
