<script setup lang="ts">
/**
 * Relevé client — a client's whole position on one page, for chasing an account.
 *
 * Archived purchases are listed separately from the totals, exactly as
 * `ClientDetailView` shows them: an archived purchase is off the books and is
 * not owed, so folding it into the balance would overstate the debt on a
 * document the shop may hand to the person who owes it.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import DocumentSection from "@/components/print/DocumentSection.vue";
import FieldGrid from "@/components/print/FieldGrid.vue";
import { useFormat } from "@/composables/useFormat";
import { todayIso } from "@/lib/finance";
import type { ClientDetail, Payment } from "@/types/models";

const props = defineProps<{ detail: ClientDetail; payments: Payment[] }>();

const { t } = useI18n();
const fmt = useFormat();

const clientFields = computed(() => {
  const c = props.detail.client;
  return [
    { label: t("print.field.client"), value: `${c.firstName} ${c.lastName}` },
    { label: t("clients.columns.phone"), value: c.phone || "—" },
    { label: t("print.field.address"), value: c.address || "—" },
    { label: t("print.field.email"), value: c.email || "—" },
  ];
});

/** Most recent first — the rows a shop reads on a statement are the newest. */
const orderedPayments = computed(() =>
  [...props.payments].sort((a, b) => b.paymentDate.localeCompare(a.paymentDate) || b.id - a.id),
);
</script>

<template>
  <DocumentSection :title="t('print.section.client')">
    <FieldGrid :fields="clientFields" />
  </DocumentSection>

  <section>
    <h2 class="doc-section-title">{{ t("print.section.purchases") }}</h2>
    <table class="doc-table">
      <thead>
        <tr>
          <th>{{ t("echeances.columns.reference") }}</th>
          <th>{{ t("common.product") }}</th>
          <th class="num">{{ t("print.field.total") }}</th>
          <th class="num">{{ t("common.paid") }}</th>
          <th class="num">{{ t("common.remaining") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="p in detail.purchases" :key="p.id">
          <td class="tabular">{{ p.reference }}</td>
          <td>{{ p.productLabel }}</td>
          <td class="num tabular">{{ fmt.money(p.totalPrice) }}</td>
          <td class="num tabular">{{ fmt.money(p.paidAmount) }}</td>
          <td class="num tabular">{{ fmt.money(p.remaining) }}</td>
        </tr>
      </tbody>
      <tfoot>
        <tr>
          <td colspan="2">{{ t("common.total") }}</td>
          <td class="num tabular">{{ fmt.money(detail.totalPurchased) }}</td>
          <td class="num tabular">{{ fmt.money(detail.totalPaid) }}</td>
          <td class="num tabular">{{ fmt.money(detail.totalOutstanding) }}</td>
        </tr>
      </tfoot>
    </table>
    <p class="as-of">{{ t("print.balanceAsOf", { date: fmt.date(todayIso()) }) }}</p>
  </section>

  <section v-if="orderedPayments.length">
    <h2 class="doc-section-title">{{ t("print.section.payments") }}</h2>
    <table class="doc-table">
      <thead>
        <tr>
          <th>{{ t("paiements.columns.date") }}</th>
          <th>{{ t("echeances.columns.reference") }}</th>
          <th>{{ t("print.column.tranche") }}</th>
          <th class="num">{{ t("echeances.columns.amount") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="pay in orderedPayments" :key="pay.id">
          <td class="tabular">{{ fmt.date(pay.paymentDate) }}</td>
          <td class="tabular">{{ pay.purchaseReference }}</td>
          <td class="tabular">{{ pay.installmentIndex }}</td>
          <td class="num tabular">{{ fmt.money(pay.amount) }}</td>
        </tr>
      </tbody>
    </table>
  </section>

  <DocumentSection v-if="detail.archivedPurchases.length" :title="t('print.section.archived')">
    <!-- Listed but excluded from every total above: an archived purchase is off
         the books and is not owed. -->
    <p class="archived-note">{{ t("print.archivedNote") }}</p>
    <table class="doc-table">
      <tbody>
        <tr v-for="p in detail.archivedPurchases" :key="p.id">
          <td class="tabular">{{ p.reference }}</td>
          <td>{{ p.productLabel }}</td>
          <td class="num tabular">{{ fmt.money(p.totalPrice) }}</td>
        </tr>
      </tbody>
    </table>
  </DocumentSection>
</template>

<style scoped>
.archived-note {
  font-size: 11px;
  color: #6b7280;
  margin-bottom: var(--space-2);
}
</style>
