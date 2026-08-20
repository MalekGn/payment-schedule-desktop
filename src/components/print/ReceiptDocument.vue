<script setup lang="ts">
/**
 * Reçu de paiement — what the client takes away when cash crosses the counter.
 *
 * The amount received is the one figure that is historical and fixed: it is
 * what this ledger entry recorded, and it never changes. Everything else on the
 * receipt is a balance, and a balance moves — so the remaining figure carries
 * its as-of date. Reprint this receipt six months later and it will show what is
 * owed *then*, which is honest only because the document says so.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import DocumentSection from "@/components/print/DocumentSection.vue";
import FieldGrid from "@/components/print/FieldGrid.vue";
import { useFormat } from "@/composables/useFormat";
import { todayIso } from "@/lib/finance";
import type { Payment, PurchaseDetail } from "@/types/models";

const props = defineProps<{ detail: PurchaseDetail; payment: Payment }>();

const { t } = useI18n();
const fmt = useFormat();

const fields = computed(() => {
  const c = props.detail.client;
  const p = props.detail.purchase;
  return [
    { label: t("print.field.client"), value: `${c.firstName} ${c.lastName}` },
    { label: t("clients.columns.phone"), value: c.phone || "—" },
    { label: t("print.field.purchase"), value: p.reference },
    { label: t("print.field.product"), value: p.productLabel },
    {
      label: t("print.column.tranche"),
      value: `${props.payment.installmentIndex}/${p.installmentCount}`,
    },
    { label: t("paiements.columns.date"), value: fmt.date(props.payment.paymentDate) },
  ];
});
</script>

<template>
  <DocumentSection>
    <FieldGrid :fields="fields" />
  </DocumentSection>

  <DocumentSection>
    <div class="amount-box">
      <span class="amount-label">{{ t("print.amountReceived") }}</span>
      <span class="amount-value tabular">{{ fmt.money(payment.amount) }}</span>
    </div>
    <p v-if="payment.note" class="note">
      {{ t("common.note") }} : <span>{{ payment.note }}</span>
    </p>
  </DocumentSection>

  <DocumentSection :title="t('print.section.balance')">
    <table class="doc-table">
      <tbody>
        <tr>
          <td>{{ t("print.field.total") }}</td>
          <td class="num tabular">{{ fmt.money(detail.purchase.totalPrice) }}</td>
        </tr>
        <tr>
          <td>{{ t("print.totalPaidToDate") }}</td>
          <td class="num tabular">{{ fmt.money(detail.totalPaid) }}</td>
        </tr>
        <tr class="strong-row">
          <td>{{ t("common.remaining") }}</td>
          <td class="num tabular">{{ fmt.money(detail.remaining) }}</td>
        </tr>
      </tbody>
    </table>
    <p class="as-of">{{ t("print.balanceAsOf", { date: fmt.date(todayIso()) }) }}</p>
  </DocumentSection>

  <DocumentSection>
    <div class="signature">
      <span class="signature-label">{{ t("print.signature.shop") }}</span>
      <span class="signature-line"></span>
    </div>
  </DocumentSection>
</template>

<style scoped>
.amount-box {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-4);
  padding: var(--space-4) var(--space-5);
  border: 2px solid #111827;
  border-radius: 6px;
}
.amount-label {
  font-size: 13px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.amount-value {
  font-size: 24px;
  font-weight: 700;
  white-space: nowrap;
}
.note {
  font-size: 12px;
  color: #4b5563;
  margin-top: var(--space-3);
}
.note span {
  font-weight: 600;
  color: #111827;
}
.signature {
  display: flex;
  flex-direction: column;
  gap: 34px;
  max-width: 260px;
  margin-inline-start: auto;
  margin-top: var(--space-6);
}
.signature-label {
  font-size: 12px;
  font-weight: 600;
  color: #4b5563;
}
.signature-line {
  border-top: 1px solid #111827;
}
</style>
