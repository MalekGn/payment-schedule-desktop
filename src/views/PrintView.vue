<script setup lang="ts">
/**
 * Route component for every printable document.
 *
 * Renders without the app shell — see the `meta.print` branch in `App.vue` for
 * why. One view rather than three because the three documents differ only in
 * what they load and which component they hand it to; the sheet, the letterhead,
 * the action bar and the failure states are identical.
 */
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { api } from "@/api";
import PrintDocument from "@/components/print/PrintDocument.vue";
import ReceiptDocument from "@/components/print/ReceiptDocument.vue";
import ScheduleDocument from "@/components/print/ScheduleDocument.vue";
import StatementDocument from "@/components/print/StatementDocument.vue";
import AppIcon from "@/components/ui/AppIcon.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import LoadError from "@/components/ui/LoadError.vue";
import { useBack } from "@/composables/useBack";
import { useDocumentTitle } from "@/composables/useDocumentTitle";
import { useLoader } from "@/composables/useLoader";
import { clientPart, documentFilename } from "@/lib/filename";
import { todayIso } from "@/lib/finance";
import type { ClientDetail, Payment, PurchaseDetail } from "@/types/models";

export type PrintKind = "schedule" | "receipt" | "statement";

const props = defineProps<{
  kind: PrintKind;
  /** Purchase id for a schedule or receipt; client id for a statement. */
  id: string;
  /** Receipt only: which payment in that purchase's ledger. */
  paymentId?: string;
}>();

const { t } = useI18n();
const goBack = useBack("/");

const purchase = ref<PurchaseDetail | null>(null);
const client = ref<ClientDetail | null>(null);
const payments = ref<Payment[]>([]);
/** The addressed record does not exist — a recoverable message, not an error. */
const notFound = ref(false);

const payment = computed(() =>
  props.paymentId ? (payments.value.find((p) => p.id === Number(props.paymentId)) ?? null) : null,
);

const title = computed(() => t(`print.title.${props.kind}`));

/**
 * The document's own reference, shown under the title on the letterhead.
 *
 * A statement has none: it covers a client rather than one numbered record, and
 * the client's name is already in the body.
 */
const reference = computed(() =>
  props.kind === "statement" ? undefined : purchase.value?.purchase.reference,
);

/**
 * The name the print dialog will suggest for the saved file.
 *
 * ASCII and locale-independent on purpose — see `src/lib/filename.ts`. `null`
 * until the data is in, so a half-loaded or missing document never renames the
 * window.
 *
 * Note this is *not* `title` above: that one is translated, because it is
 * printed on the letterhead where the reader is the client.
 */
const documentName = computed<string | null>(() => {
  if (notFound.value) return null;

  if (props.kind === "statement") {
    const c = client.value?.client;
    if (!c) return null;
    return documentFilename("Releve", clientPart(`${c.firstName} ${c.lastName}`, c.id), todayIso());
  }

  const detail = purchase.value;
  if (!detail) return null;

  if (props.kind === "receipt") {
    const pay = payment.value;
    if (!pay) return null;
    // Dated by the payment, not by today: reprinting last month's receipt must
    // not produce a file that claims to be this month's.
    return documentFilename(
      "Recu",
      detail.purchase.reference,
      `T${pay.installmentIndex}`,
      pay.paymentDate,
    );
  }

  return documentFilename(
    "Echeancier",
    detail.purchase.reference,
    clientPart(`${detail.client.firstName} ${detail.client.lastName}`, detail.client.id),
  );
});

useDocumentTitle(documentName);

/** Back lands on the record the document was printed from. */
const backTo = computed(() =>
  props.kind === "statement" ? `/clients/${props.id}` : `/achats/${props.id}`,
);

const {
  loading,
  error: loadError,
  run: load,
} = useLoader(async () => {
  notFound.value = false;
  const id = Number(props.id);
  // A non-numeric id would otherwise reach the backend as NaN and come back as
  // an opaque failure rather than "no such record".
  if (!Number.isInteger(id) || id <= 0) {
    notFound.value = true;
    return;
  }

  if (props.kind === "statement") {
    client.value = await api.getClientDetail(id);
    payments.value = await api.listPaymentsForClient(id);
    return;
  }

  purchase.value = await api.getPurchaseDetail(id);
  if (props.kind === "receipt") {
    payments.value = await api.listPaymentsForPurchase(id);
    // A payment id that belongs to a different purchase must not render a
    // document with the wrong client's name on it.
    if (!payment.value) notFound.value = true;
  }
});
onMounted(load);
// All three print routes share this component, so vue-router reuses the instance
// when one is navigated to from another and `onMounted` does not run again.
// Nothing in the UI links between them today, but the routes are plain URLs and
// the failure mode is a document showing another record's data.
watch(() => [props.kind, props.id, props.paymentId], load);
</script>

<template>
  <LoadError v-if="loadError" :message="loadError" @retry="load" />

  <div v-else-if="notFound" class="print-missing">
    <EmptyState icon="report" :title="t('print.missing')" />
    <button class="btn btn--ghost" type="button" @click="goBack">
      <AppIcon name="arrow-left" :size="16" class="icon-flip" /> {{ t("common.back") }}
    </button>
  </div>

  <PrintDocument v-else-if="!loading" :title="title" :reference="reference" :back-to="backTo">
    <ScheduleDocument v-if="kind === 'schedule' && purchase" :detail="purchase" />
    <ReceiptDocument
      v-else-if="kind === 'receipt' && purchase && payment"
      :detail="purchase"
      :payment="payment"
    />
    <StatementDocument
      v-else-if="kind === 'statement' && client"
      :detail="client"
      :payments="payments"
    />
  </PrintDocument>
</template>

<style scoped>
.print-missing {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--space-4);
  background: var(--bg);
}
</style>
