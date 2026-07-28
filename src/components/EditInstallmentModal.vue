<script setup lang="ts">
// Update one installment: what is owed, and what has been collected against it.
//
// This replaced the old payment modal, so it is the single place an installment
// changes. Its fields split into two halves under opposite rules:
//
//  * **The schedule** — amount and due date — is editable until the installment
//    settles, after which it is history. The purchase total never moves: a
//    changed amount is absorbed by the other unsettled installments, previewed
//    live below so the whole consequence is visible before saving. A due date is
//    clamped to its neighbours' dates, which is what keeps position order and
//    date order the same thing.
//  * **The money** — paid amount, payment date and note — is editable only once
//    the *previous* installment is settled, because cash is collected in order.
//    The paid amount is absolute (the running total collected), and the backend
//    turns the difference into a correction entry in the payment ledger.
//
// Every rule is re-checked by `update_installment` in
// `src-tauri/src/commands.rs`; the gating here is to explain, not to enforce.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import BaseModal from "@/components/ui/BaseModal.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import DatePicker from "@/components/ui/DatePicker.vue";
import { useFormat } from "@/composables/useFormat";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { toUserMessage } from "@/lib/errors";
import { api } from "@/api";
import { rebalanceAmounts, todayIso } from "@/lib/finance";
import type { Installment, InstallmentEdit, PurchaseDetail } from "@/types/models";

const props = defineProps<{
  installment: Installment;
  /** Every installment of the purchase, in position order. */
  siblings: Installment[];
  installmentCount: number;
  purchaseReference: string;
}>();
const emit = defineEmits<{ close: []; saved: [detail: PurchaseDetail] }>();

const { t } = useI18n();
const fmt = useFormat();
const ui = useUiStore();
const stats = useStatsStore();

const amount = ref<number | string>(props.installment.amount);
const dueDate = ref(props.installment.dueDate);
const paidAmount = ref<number | string>(props.installment.paidAmount);
const paymentDate = ref(props.installment.paidDate ?? todayIso());
const note = ref("");
const error = ref("");
const saving = ref(false);
const confirming = ref(false);

const pos = computed(() => props.siblings.findIndex((i) => i.id === props.installment.id));
const previous = computed(() => (pos.value > 0 ? props.siblings[pos.value - 1] : null));
const next = computed(() =>
  pos.value >= 0 && pos.value + 1 < props.siblings.length ? props.siblings[pos.value + 1] : null,
);
const settled = computed(() => props.installment.status === "paid");

/** A settled installment's schedule is history: neither number moves. */
const scheduleLocked = computed(() => settled.value);
/** Cash is collected in order, so it cannot be recorded out of order. */
const moneyLocked = computed(() => previous.value !== null && previous.value.status !== "paid");

/** Parse a money field, or null while it is empty or not a number. */
function parseMoney(value: number | string): number | null {
  const n = Number(value);
  return value !== "" && Number.isFinite(n) ? Math.round(n) : null;
}
const parsedAmount = computed(() => parseMoney(amount.value));
const parsedPaid = computed(() => parseMoney(paidAmount.value));

const amountChanged = computed(
  () => parsedAmount.value !== null && parsedAmount.value !== props.installment.amount,
);
const paidChanged = computed(
  () => parsedPaid.value !== null && parsedPaid.value !== props.installment.paidAmount,
);

/** What the rest of the schedule becomes if this amount is saved. */
const rebalanced = computed(() => {
  if (!amountChanged.value || scheduleLocked.value) return null;
  const paidAmounts = props.siblings.map((i) => i.paidAmount);
  // The edited row's floor is what this edit lands on, matching the backend.
  if (parsedPaid.value !== null && !moneyLocked.value) paidAmounts[pos.value] = parsedPaid.value;
  return rebalanceAmounts(
    props.siblings.map((i) => i.amount),
    paidAmounts,
    pos.value,
    parsedAmount.value!,
  );
});

/** Only the siblings the rebalance actually moves, for the preview list. */
const affected = computed(() => {
  const nextAmounts = rebalanced.value;
  if (!nextAmounts) return [];
  return props.siblings
    .map((inst, i) => ({ inst, to: nextAmounts[i] }))
    .filter(({ inst, to }) => inst.id !== props.installment.id && inst.amount !== to);
});

/**
 * Whether this edit leaves a payment for a date or a note to describe — either
 * one already recorded, or one this edit is about to create.
 */
const hasPayment = computed(() => (parsedPaid.value ?? props.installment.paidAmount) > 0);

/** The remaining balance this edit would leave on the installment. */
const remainingAfter = computed(
  () => (parsedAmount.value ?? props.installment.amount) - (parsedPaid.value ?? 0),
);

/**
 * Why the form as typed cannot be saved, or "" when it can. Mirrors the backend
 * codes so a refusal shows up as the fields are edited rather than after a round
 * trip.
 */
const problem = computed(() => {
  const finalAmount = parsedAmount.value ?? props.installment.amount;
  const finalPaid = parsedPaid.value ?? props.installment.paidAmount;
  if (parsedAmount.value !== null && parsedAmount.value < 0) {
    return t("achats.installmentEdit.negativeAmount");
  }
  if (parsedPaid.value !== null && parsedPaid.value < 0) {
    return t("achats.installmentEdit.negativeAmount");
  }
  if (finalPaid > finalAmount) {
    return paidChanged.value
      ? t("errors.paidAboveAmount", { amount: finalAmount })
      : t("errors.belowPaid", { paidAmount: props.installment.paidAmount });
  }
  if (amountChanged.value && rebalanced.value === null) return t("errors.noRebalanceRoom");
  return "";
});

const canSave = computed(() => !saving.value && problem.value === "");

/** Requirement 3: modifying a settled installment is a deliberate second step. */
const needsConfirm = computed(() => settled.value);

function attemptSave() {
  error.value = "";
  if (!canSave.value) {
    error.value = problem.value;
    return;
  }
  if (needsConfirm.value) {
    confirming.value = true;
    return;
  }
  void save();
}

async function save() {
  confirming.value = false;
  saving.value = true;
  error.value = "";
  try {
    // Send only what actually changed: an omitted field is left alone, which
    // keeps a schedule edit from tripping the money rules and vice versa.
    const edit: InstallmentEdit = {};
    if (!scheduleLocked.value) {
      if (amountChanged.value) edit.amount = parsedAmount.value!;
      if (dueDate.value !== props.installment.dueDate) edit.dueDate = dueDate.value;
    }
    // A date and a note describe a ledger entry, so they only travel when there
    // is one for them to land on.
    if (!moneyLocked.value) {
      if (paidChanged.value) edit.paidAmount = parsedPaid.value!;
      if (hasPayment.value) {
        if (paidChanged.value || paymentDate.value !== props.installment.paidDate) {
          edit.paymentDate = paymentDate.value;
        }
        if (note.value.trim()) edit.note = note.value.trim();
      }
    }
    const detail = await api.updateInstallment(props.installment.id, edit);
    await stats.refresh();
    ui.notify(t("achats.installmentEdit.saved"));
    emit("saved", detail);
  } catch (e) {
    error.value = toUserMessage(e, t);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <BaseModal
    :title="
      t('achats.installmentEdit.title', {
        index: installment.index,
        count: installmentCount,
      })
    "
    @close="emit('close')"
  >
    <div class="edit-grid">
      <div class="edit-info">
        <span class="muted">{{ purchaseReference }}</span>
        <div class="edit-figures">
          <div>
            <span class="edit-fig-label">{{ t("paiements.dueAmount") }}</span>
            <span class="edit-fig tabular">{{ fmt.money(installment.amount) }}</span>
          </div>
          <div>
            <span class="edit-fig-label">{{ t("paiements.alreadyPaid") }}</span>
            <span class="edit-fig tabular">{{ fmt.money(installment.paidAmount) }}</span>
          </div>
          <div>
            <span class="edit-fig-label">{{ t("common.remaining") }}</span>
            <span class="edit-fig tabular strong">{{ fmt.money(remainingAfter) }}</span>
          </div>
        </div>
      </div>

      <form class="edit-form" @submit.prevent="attemptSave">
        <fieldset class="field-group" :disabled="scheduleLocked">
          <legend>{{ t("achats.installmentEdit.scheduleSection") }}</legend>
          <!-- Inside the group, so the reason reads as belonging to the fields
               it disables rather than to whatever precedes it. -->
          <p v-if="scheduleLocked" class="lock-note">
            {{ t("achats.installmentEdit.settledNote") }}
          </p>
          <div class="field">
            <label for="inst-amount">{{ t("dashboard.detail.amount") }}</label>
            <input id="inst-amount" v-model="amount" type="number" min="0" class="input" />
          </div>
          <div class="field">
            <label>{{ t("dashboard.detail.dueDate") }}</label>
            <!-- Clamped to the neighbours so position order and date order can
                 never diverge; the picker greys out every day outside them. -->
            <DatePicker
              v-model="dueDate"
              :min="previous?.dueDate"
              :max="next?.dueDate"
              :disabled="scheduleLocked"
            />
          </div>
        </fieldset>

        <fieldset class="field-group" :disabled="moneyLocked">
          <legend>{{ t("achats.installmentEdit.moneySection") }}</legend>
          <p v-if="moneyLocked" class="lock-note">
            {{ t("achats.installmentEdit.lockedByPrevious", { index: previous!.index }) }}
          </p>
          <div class="field">
            <label for="inst-paid">{{ t("paiements.alreadyPaid") }}</label>
            <input id="inst-paid" v-model="paidAmount" type="number" min="0" class="input" />
            <span class="field-hint">{{ t("achats.installmentEdit.paidAmountHint") }}</span>
          </div>
          <div class="field">
            <label>{{ t("dashboard.detail.paymentDate") }}</label>
            <!-- A date needs a payment to describe. With nothing collected and
                 nothing being collected there is no ledger entry to carry it,
                 and the backend refuses with NO_PAYMENT_TO_DATE. -->
            <DatePicker
              v-model="paymentDate"
              :max="todayIso()"
              :disabled="moneyLocked || !hasPayment"
            />
          </div>
          <div class="field">
            <label for="inst-note"
              >{{ t("paiements.note") }}
              <span class="muted">({{ t("common.optional") }})</span></label
            >
            <input id="inst-note" v-model="note" type="text" class="input" />
          </div>
        </fieldset>

        <div v-if="affected.length > 0" class="rebalance" role="status">
          <p class="rebalance-title">{{ t("achats.installmentEdit.rebalanceTitle") }}</p>
          <ul>
            <li v-for="row in affected" :key="row.inst.id">
              <span class="tabular">{{ row.inst.index }}</span>
              <span class="tabular muted">{{ fmt.money(row.inst.amount) }}</span>
              <!-- U+2192 is not auto-mirrored by bidi, so it has to be flipped
                   explicitly or it points back at the old value in Arabic. -->
              <span class="rebalance-arrow icon-flip" aria-hidden="true">→</span>
              <span class="tabular strong">{{ fmt.money(row.to) }}</span>
            </li>
          </ul>
          <p class="rebalance-note">{{ t("achats.installmentEdit.rebalanceNote") }}</p>
        </div>

        <span v-if="problem || error" class="field-error">{{ problem || error }}</span>
      </form>
    </div>

    <template #footer>
      <button class="btn btn--ghost" type="button" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
      <button class="btn btn--primary" type="button" :disabled="!canSave" @click="attemptSave">
        {{ t("common.save") }}
      </button>
    </template>
  </BaseModal>

  <ConfirmDialog
    v-if="confirming"
    :title="t('achats.installmentEdit.confirmTitle')"
    :message="t('achats.installmentEdit.confirmText', { index: installment.index })"
    :confirm-label="t('common.save')"
    danger
    @close="confirming = false"
    @confirm="save"
  />
</template>

<style scoped>
.edit-grid {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.edit-info {
  background: var(--bg);
  border-radius: 12px;
  padding: 16px;
}
.edit-figures {
  display: flex;
  gap: 24px;
  margin-top: 10px;
}
.edit-fig-label {
  display: block;
  font-size: 12px;
  color: var(--text-muted);
}
.edit-fig {
  font-size: 15px;
  font-weight: 600;
}
.edit-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
/* A fieldset is what lets one `disabled` cover a whole half of the form, so a
   locked group cannot be half-editable by accident. */
.field-group {
  display: flex;
  flex-direction: column;
  gap: 14px;
  border: none;
  padding: 0;
  margin: 0;
}
.field-group:disabled {
  opacity: 0.55;
}
.field-group legend {
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-muted);
  padding: 0;
  margin-block-end: 8px;
}
.lock-note {
  font-size: 12.5px;
  color: var(--text-muted);
  line-height: 1.4;
}
.field-hint {
  font-size: 12.5px;
  color: var(--text-muted);
  line-height: 1.4;
}
.rebalance {
  background: var(--bg);
  border-radius: 10px;
  padding: 12px 14px;
}
.rebalance-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-secondary);
}
.rebalance ul {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.rebalance li {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 13px;
}
.rebalance li > span:first-child {
  min-width: 1.5em;
  color: var(--text-muted);
}
/* `transform` is a no-op on an inline box, and `.icon-flip` mirrors this in RTL. */
.rebalance-arrow {
  display: inline-block;
}
.rebalance-note {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.4;
}
</style>
