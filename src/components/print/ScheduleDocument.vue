<script setup lang="ts">
/**
 * Échéancier — the installment schedule handed to the client at the sale.
 *
 * This is the document the sale itself rests on, which is why it carries
 * signature blocks: it is the shop's and the client's shared record of what was
 * agreed. Every tranche is listed with what it is worth and when it falls due.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import DocumentSection from "@/components/print/DocumentSection.vue";
import FieldGrid from "@/components/print/FieldGrid.vue";
import { useFormat } from "@/composables/useFormat";
import { todayIso } from "@/lib/finance";
import type { PurchaseDetail } from "@/types/models";

const props = defineProps<{ detail: PurchaseDetail }>();

const { t } = useI18n();
const fmt = useFormat();

const clientFields = computed(() => {
  const c = props.detail.client;
  return [
    { label: t("print.field.client"), value: `${c.firstName} ${c.lastName}` },
    { label: t("clients.columns.phone"), value: c.phone || "—" },
    { label: t("print.field.address"), value: c.address || "—" },
  ];
});

const purchaseFields = computed(() => {
  const p = props.detail.purchase;
  return [
    { label: t("print.field.product"), value: p.productLabel },
    { label: t("print.field.purchaseDate"), value: fmt.date(p.purchaseDate) },
    { label: t("print.field.total"), value: fmt.money(p.totalPrice) },
    {
      label: t("print.field.installments"),
      value: t(`print.interval.${p.intervalKind}`, { count: p.installmentCount }),
    },
  ];
});
</script>

<template>
  <DocumentSection :title="t('print.section.client')">
    <FieldGrid :fields="clientFields" />
  </DocumentSection>

  <DocumentSection :title="t('print.section.purchase')">
    <FieldGrid :fields="purchaseFields" />
  </DocumentSection>

  <!-- Not wrapped in DocumentSection: a long schedule *should* break across
       pages, and the print stylesheet repeats the header row when it does. -->
  <section>
    <h2 class="doc-section-title">{{ t("print.section.schedule") }}</h2>
    <table class="doc-table">
      <thead>
        <tr>
          <th>{{ t("print.column.tranche") }}</th>
          <th>{{ t("echeances.columns.dueDate") }}</th>
          <th class="num">{{ t("echeances.columns.amount") }}</th>
          <th class="num">{{ t("common.paid") }}</th>
          <th class="num">{{ t("common.remaining") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="inst in detail.installments" :key="inst.id">
          <td class="tabular">{{ inst.index }}/{{ detail.purchase.installmentCount }}</td>
          <td class="tabular">{{ fmt.date(inst.dueDate) }}</td>
          <td class="num tabular">{{ fmt.money(inst.amount) }}</td>
          <td class="num tabular">{{ fmt.money(inst.paidAmount) }}</td>
          <td class="num tabular">{{ fmt.money(inst.amount - inst.paidAmount) }}</td>
        </tr>
      </tbody>
      <tfoot>
        <tr>
          <td colspan="2">{{ t("common.total") }}</td>
          <td class="num tabular">{{ fmt.money(detail.purchase.totalPrice) }}</td>
          <td class="num tabular">{{ fmt.money(detail.totalPaid) }}</td>
          <td class="num tabular">{{ fmt.money(detail.remaining) }}</td>
        </tr>
      </tfoot>
    </table>
    <!-- What is paid and owed moves after the document is printed, so the
         figures above are stamped rather than presented as timeless. -->
    <p class="as-of">{{ t("print.balanceAsOf", { date: fmt.date(todayIso()) }) }}</p>
  </section>

  <DocumentSection>
    <div class="signatures">
      <div class="signature">
        <span class="signature-label">{{ t("print.signature.shop") }}</span>
        <span class="signature-line"></span>
      </div>
      <div class="signature">
        <span class="signature-label">{{ t("print.signature.client") }}</span>
        <span class="signature-line"></span>
      </div>
    </div>
  </DocumentSection>
</template>

<style scoped>
.signatures {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-8);
  margin-top: var(--space-6);
}
.signature {
  display: flex;
  flex-direction: column;
  gap: 34px;
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
