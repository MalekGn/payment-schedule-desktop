<script setup lang="ts">
// Record what has been collected against one installment.
//
// This modal owns the *money* and nothing else. What is owed — the amount and
// the due date — is shown here read-only and edited in one place, the purchase
// editor, so a schedule change is always judged against the whole schedule.
// `update_installment` refuses both fields outright, so this is a structural
// split rather than a UI convention.
//
// The two rules the money follows:
//
//  * The paid amount is editable only once the *previous* installment is
//    settled, because cash is collected in order. It is absolute (the running
//    total collected), and the backend turns the difference into a correction
//    entry in the payment ledger.
//  * A payment date is history once recorded. It can only be given to date the
//    entry this save is about to create, which is why the field opens up only
//    while the collected figure is being moved.
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
import { todayIso } from "@/lib/finance";
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

const paidAmount = ref<number | string>(props.installment.paidAmount);
// Today, not the date already on record: this field now only ever dates the
// ledger entry this save creates, and that entry is being made now. It stays
// editable so a payment taken last week can still be dated honestly.
const paymentDate = ref(todayIso());
const note = ref("");
const error = ref("");
const saving = ref(false);
const confirming = ref(false);

const pos = computed(() => props.siblings.findIndex((i) => i.id === props.installment.id));
const previous = computed(() => (pos.value > 0 ? props.siblings[pos.value - 1] : null));
const settled = computed(() => props.installment.status === "paid");

/** Cash is collected in order, so it cannot be recorded out of order. */
const moneyLocked = computed(() => previous.value !== null && previous.value.status !== "paid");

/** Parse a money field, or null while it is empty or not a number. */
function parseMoney(value: number | string): number | null {
  const n = Number(value);
  return value !== "" && Number.isFinite(n) ? Math.round(n) : null;
}
const parsedPaid = computed(() => parseMoney(paidAmount.value));

const paidChanged = computed(
  () => parsedPaid.value !== null && parsedPaid.value !== props.installment.paidAmount,
);

/**
 * Whether this edit leaves a payment for a note to describe — either one
 * already recorded, or one this edit is about to create.
 */
const hasPayment = computed(() => (parsedPaid.value ?? props.installment.paidAmount) > 0);

/**
 * A payment date describes the ledger entry this save creates, so it is only
 * open while the collected figure is moving. Anything already recorded keeps
 * the date it was collected on — the backend refuses with `PAYMENT_DATE_LOCKED`.
 */
const paymentDateLocked = computed(() => moneyLocked.value || !paidChanged.value);

/** The remaining balance this edit would leave on the installment. */
const remainingAfter = computed(() => props.installment.amount - (parsedPaid.value ?? 0));

/**
 * Why the form as typed cannot be saved, or "" when it can. Mirrors the backend
 * codes so a refusal shows up as the fields are edited rather than after a round
 * trip.
 */
const problem = computed(() => {
  if (parsedPaid.value !== null && parsedPaid.value < 0) {
    return t("achats.installmentEdit.negativeAmount");
  }
  if ((parsedPaid.value ?? props.installment.paidAmount) > props.installment.amount) {
    return t("errors.paidAboveAmount", { amount: props.installment.amount });
  }
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
    // Send only what actually changed: an omitted field is left alone. `amount`
    // and `dueDate` never travel — the backend refuses them outright.
    const edit: InstallmentEdit = {};
    if (!moneyLocked.value) {
      if (paidChanged.value) {
        edit.paidAmount = parsedPaid.value!;
        // Dates the entry this save creates. Without a moved figure there is no
        // entry to date, and re-dating an existing one is refused.
        edit.paymentDate = paymentDate.value;
      }
      if (hasPayment.value && note.value.trim()) edit.note = note.value.trim();
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
            <span class="edit-fig-label">{{ t("dashboard.detail.dueDate") }}</span>
            <span class="edit-fig tabular">{{ fmt.date(installment.dueDate) }}</span>
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
        <!-- The two figures above are the schedule, and the schedule is edited
             as a whole in the purchase editor — never one row at a time. -->
        <p class="edit-note">{{ t("achats.installmentEdit.scheduleElsewhere") }}</p>
      </div>

      <form class="edit-form" @submit.prevent="attemptSave">
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
            <!-- A date describes the ledger entry this save creates, so it only
                 opens while the collected figure is moving. A date already on
                 record is history; the backend refuses PAYMENT_DATE_LOCKED. -->
            <DatePicker v-model="paymentDate" :max="todayIso()" :disabled="paymentDateLocked" />
            <span v-if="paymentDateLocked && installment.paidDate" class="field-hint">
              {{ t("achats.installmentEdit.paymentDateRecorded") }}
            </span>
          </div>
          <div class="field">
            <label for="inst-note"
              >{{ t("paiements.note") }}
              <span class="muted">({{ t("common.optional") }})</span></label
            >
            <input id="inst-note" v-model="note" type="text" class="input" />
          </div>
        </fieldset>

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
  flex-wrap: wrap;
  gap: 24px;
  margin-top: 10px;
}
.edit-note {
  margin-top: 12px;
  font-size: 12.5px;
  color: var(--text-muted);
  line-height: 1.4;
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
</style>
