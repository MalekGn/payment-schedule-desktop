<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import BaseModal from "@/components/ui/BaseModal.vue";
import DatePicker from "@/components/ui/DatePicker.vue";
import { useFormat } from "@/composables/useFormat";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { toUserMessage } from "@/lib/errors";
import { api } from "@/api";
import { addInterval, rebalanceAmounts, splitAmounts, todayIso } from "@/lib/finance";
import type { ClientSummary, IntervalKind, PurchaseDetail } from "@/types/models";

/** Editing when a purchase is supplied, creating otherwise (cf. `ClientForm`). */
const props = defineProps<{ purchase?: PurchaseDetail | null }>();
const emit = defineEmits<{ close: []; saved: [detail: PurchaseDetail] }>();

const editing = computed(() => props.purchase != null);
/**
 * This form is the one place an installment's amount or due date changes — the
 * tranche editor deals only in money — so it stays usable after cash has been
 * taken, unlike before.
 *
 * What locks is narrower than it used to be. A *settled* tranche is history and
 * neither of its numbers may move (the backend refuses `AMOUNT_LOCKED` /
 * `DUE_DATE_LOCKED`), which in turn locks everything the whole schedule is
 * regenerated from: the anchor date, the count and the cadence all rewrite
 * every row's due date, settled ones included. The total price stays open — the
 * difference is absorbed by the tranches that are still owed.
 */
const settledCount = computed(
  () => props.purchase?.installments.filter((i) => i.paidAmount >= i.amount).length ?? 0,
);
const anchorLocked = computed(() => settledCount.value > 0);

const { t } = useI18n();
const fmt = useFormat();
const ui = useUiStore();
const stats = useStatsStore();

const clients = ref<ClientSummary[]>([]);
const saving = ref(false);

const form = reactive({
  clientId: (props.purchase ? String(props.purchase.purchase.clientId) : "") as string,
  productLabel: props.purchase?.purchase.productLabel ?? "",
  totalPrice: (props.purchase?.purchase.totalPrice ?? null) as number | null,
  installmentCount: props.purchase?.purchase.installmentCount ?? 3,
  intervalKind: (props.purchase?.purchase.intervalKind ?? "monthly") as IntervalKind,
  intervalDays: props.purchase?.purchase.intervalDays ?? 30,
  purchaseDate: props.purchase?.purchase.purchaseDate ?? todayIso(),
});
const inlineClient = reactive({ firstName: "", lastName: "", phone: "", address: "", email: "" });

interface Row {
  amount: number;
  dueDate: string;
  /** What has been collected on this tranche; 0 on a purchase being created. */
  paidAmount: number;
  /** Settled, so neither number may move. */
  locked: boolean;
}
const rows = ref<Row[]>([]);
const manualAmounts = ref(false);
const errors = reactive<Record<string, string>>({});

/**
 * The amounts as last agreed on, so a typed row can be rebalanced against what
 * the others were *before* the keystroke rather than after it.
 */
const committed = ref<number[]>([]);

onMounted(async () => {
  // Explicit rather than relying on the gateway default: an archived client
  // must not be selectable, or archiving them would not stop new debt.
  clients.value = await api.listClients("active");
  if (props.purchase) {
    // Seed the rows from the stored schedule rather than recomputing: an edit
    // that leaves them untouched must send back exactly what is on record, or
    // the backend reads it as a reschedule.
    rows.value = props.purchase.installments.map((i) => ({
      amount: i.amount,
      dueDate: i.dueDate,
      paidAmount: i.paidAmount,
      locked: i.paidAmount >= i.amount,
    }));
    committed.value = rows.value.map((r) => r.amount);
    manualAmounts.value = true;
    return;
  }
  rebuild();
});

function rebuild() {
  const count = Math.max(1, Math.floor(form.installmentCount || 1));
  const total = Math.max(0, Math.round(form.totalPrice ?? 0));
  const amounts =
    manualAmounts.value && rows.value.length === count
      ? rows.value.map((r) => r.amount)
      : splitAmounts(total, count);
  const previous = rows.value;
  rows.value = Array.from({ length: count }, (_, i) => {
    // A settled tranche is history and is not ours to regenerate, whatever the
    // anchor fields above say. They are disabled in that case anyway; this is
    // the belt to their braces.
    const prior = previous[i];
    if (prior?.locked) return prior;
    return {
      amount: amounts[i] ?? 0,
      dueDate: addInterval(form.purchaseDate, form.intervalKind, form.intervalDays, i),
      paidAmount: prior?.paidAmount ?? 0,
      locked: false,
    };
  });
  committed.value = rows.value.map((r) => r.amount);
}

// Only the fields the *dates* derive from rebuild the rows. The total is not one
// of them: rebuilding on it would regenerate every due date from the anchor and
// throw away the ones typed by hand, which now matters because this form is the
// only place a due date can be typed at all.
watch(
  () => [form.installmentCount, form.intervalKind, form.intervalDays, form.purchaseDate],
  rebuild,
);

// A new total re-splits the amounts only while they are still automatic; once a
// row has been typed the sum check asks the user to reconcile it, or `recompute`
// does it for them.
watch(
  () => form.totalPrice,
  () => {
    if (!manualAmounts.value) rebuild();
  },
);

/**
 * Absorb a typed amount into the other tranches so the purchase total holds.
 *
 * Only while editing: a purchase being created has no settled rows and no total
 * worth defending yet, so free typing plus the sum check below reads better.
 * `rebalanceAmounts` is the same routine the backend mirrors — it spends the
 * later tranches first and never asks a settled one to give anything up. When
 * it cannot (`null`), the typed figure is left standing and the sum check
 * explains why it does not add up.
 */
function onAmountEdit(index: number) {
  manualAmounts.value = true;
  if (!editing.value) return;
  const typed = Math.round(Number(rows.value[index].amount) || 0);
  const next = rebalanceAmounts(
    committed.value,
    rows.value.map((r) => r.paidAmount),
    index,
    typed,
  );
  if (!next) return;
  rows.value.forEach((row, i) => {
    row.amount = next[i];
  });
  committed.value = next;
}

/**
 * Re-split the total evenly. Settled tranches keep what they are worth, so only
 * the pool left over after them is shared out.
 */
function recompute() {
  const locked = rows.value.filter((r) => r.locked);
  if (locked.length === 0) {
    manualAmounts.value = false;
    rebuild();
    return;
  }
  const pool = Math.max(
    0,
    Math.round(form.totalPrice ?? 0) - locked.reduce((s, r) => s + r.amount, 0),
  );
  const open = rows.value.filter((r) => !r.locked);
  const parts = splitAmounts(pool, open.length);
  open.forEach((row, i) => {
    row.amount = parts[i];
  });
  committed.value = rows.value.map((r) => r.amount);
}

const sum = computed(() => rows.value.reduce((s, r) => s + (Number(r.amount) || 0), 0));
const sumMatches = computed(() => sum.value === Math.round(form.totalPrice ?? 0));
/** A tranche can never be worth less than what has already been collected on it. */
const belowPaid = computed(() => rows.value.find((r) => (Number(r.amount) || 0) < r.paidAmount));

function validate(): boolean {
  for (const k of Object.keys(errors)) delete errors[k];
  if (editing.value) {
    // The client is fixed on an edit; only the fields below can be wrong.
  } else if (form.clientId === "new") {
    if (!inlineClient.firstName.trim()) errors.firstName = t("validation.required");
    if (!inlineClient.lastName.trim()) errors.lastName = t("validation.required");
  } else if (!form.clientId) {
    errors.client = t("validation.required");
  }
  if (!form.productLabel.trim()) errors.product = t("validation.required");
  if (!form.totalPrice || form.totalPrice <= 0) errors.totalPrice = t("validation.positive");
  if (!form.installmentCount || form.installmentCount < 1)
    errors.installmentCount = t("validation.minInstallments");
  if (form.intervalKind === "custom" && (!form.intervalDays || form.intervalDays < 1))
    errors.intervalDays = t("validation.positive");
  if (!sumMatches.value) errors.sum = t("validation.sumMismatch");
  else if (belowPaid.value) {
    errors.sum = t("errors.belowPaid", { paidAmount: belowPaid.value.paidAmount });
  }
  return Object.keys(errors).length === 0;
}

async function submit() {
  if (!validate()) return;
  saving.value = true;
  try {
    const payload = {
      productLabel: form.productLabel,
      totalPrice: Math.round(form.totalPrice ?? 0),
      installmentCount: rows.value.length,
      intervalKind: form.intervalKind,
      intervalDays: form.intervalKind === "custom" ? form.intervalDays : null,
      purchaseDate: form.purchaseDate,
      installments: rows.value.map((r, i) => ({
        index: i + 1,
        amount: Math.round(r.amount),
        dueDate: r.dueDate,
      })),
    };

    if (props.purchase) {
      // `clientId` is ignored by the backend on update — a purchase cannot
      // change hands — but the type wants it, so send the one on record.
      const detail = await api.updatePurchase(props.purchase.purchase.id, {
        ...payload,
        clientId: props.purchase.purchase.clientId,
      });
      await stats.refresh();
      ui.notify(t("common.save"));
      emit("saved", detail);
      return;
    }

    let clientId: number;
    if (form.clientId === "new") {
      const created = await api.createClient({
        firstName: inlineClient.firstName,
        lastName: inlineClient.lastName,
        phone: inlineClient.phone,
        address: inlineClient.address,
        email: inlineClient.email.trim() || null,
      });
      clientId = created.id;
    } else {
      clientId = Number(form.clientId);
    }

    const detail = await api.createPurchase({ ...payload, clientId });
    await stats.refresh();
    ui.notify(t("common.save"));
    emit("saved", detail);
  } catch (e) {
    ui.notify(toUserMessage(e, t), "error");
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <BaseModal
    :title="editing ? t('achats.form.editTitle') : t('achats.form.title')"
    wide
    @close="emit('close')"
  >
    <form class="purchase-form" @submit.prevent="submit">
      <div class="grid-2">
        <div class="field">
          <label for="np-client">{{ t("achats.form.client") }}</label>
          <select
            id="np-client"
            v-model="form.clientId"
            class="select"
            :class="{ 'input--error': errors.client }"
            :disabled="editing"
          >
            <option value="" disabled>{{ t("achats.form.selectClient") }}</option>
            <option v-if="!editing" value="new">➕ {{ t("achats.form.newClient") }}</option>
            <option v-for="c in clients" :key="c.id" :value="String(c.id)">
              {{ c.firstName }} {{ c.lastName }}
            </option>
          </select>
          <span v-if="errors.client" class="field-error">{{ errors.client }}</span>
        </div>
        <div class="field">
          <label>{{ t("achats.form.purchaseDate") }}</label>
          <DatePicker v-model="form.purchaseDate" :disabled="anchorLocked" />
        </div>
      </div>

      <!-- Why part of the form is read-only, said once rather than per field. -->
      <p v-if="anchorLocked" class="locked-note">
        <AppIcon name="alert" :size="16" />
        {{ t("achats.form.settledRowsLocked", { count: settledCount }) }}
      </p>

      <div v-if="!editing && form.clientId === 'new'" class="inline-client">
        <div class="grid-2">
          <div class="field">
            <label>{{ t("clients.form.firstName") }}</label>
            <input
              v-model="inlineClient.firstName"
              class="input"
              :class="{ 'input--error': errors.firstName }"
            />
            <span v-if="errors.firstName" class="field-error">{{ errors.firstName }}</span>
          </div>
          <div class="field">
            <label>{{ t("clients.form.lastName") }}</label>
            <input
              v-model="inlineClient.lastName"
              class="input"
              :class="{ 'input--error': errors.lastName }"
            />
            <span v-if="errors.lastName" class="field-error">{{ errors.lastName }}</span>
          </div>
        </div>
        <div class="grid-2">
          <div class="field">
            <label>{{ t("clients.form.phone") }}</label>
            <input
              v-model="inlineClient.phone"
              class="input"
              :placeholder="t('clients.form.phonePlaceholder')"
            />
          </div>
          <div class="field">
            <label>{{ t("clients.form.address") }}</label>
            <input v-model="inlineClient.address" class="input" />
          </div>
        </div>
      </div>

      <div class="field">
        <label for="np-product">{{ t("achats.form.product") }}</label>
        <input
          id="np-product"
          v-model="form.productLabel"
          class="input"
          :class="{ 'input--error': errors.product }"
          :placeholder="t('achats.form.productPlaceholder')"
        />
        <span v-if="errors.product" class="field-error">{{ errors.product }}</span>
      </div>

      <div class="grid-4">
        <div class="field">
          <label for="np-total">{{ t("achats.form.totalPrice") }}</label>
          <input
            id="np-total"
            v-model.number="form.totalPrice"
            type="number"
            min="1"
            class="input"
            :class="{ 'input--error': errors.totalPrice }"
          />
          <span v-if="errors.totalPrice" class="field-error">{{ errors.totalPrice }}</span>
        </div>
        <div class="field">
          <label for="np-count">{{ t("achats.form.installmentCount") }}</label>
          <input
            id="np-count"
            v-model.number="form.installmentCount"
            type="number"
            min="1"
            max="60"
            class="input"
            :class="{ 'input--error': errors.installmentCount }"
            :disabled="anchorLocked"
          />
          <span v-if="errors.installmentCount" class="field-error">{{
            errors.installmentCount
          }}</span>
        </div>
        <div class="field">
          <label for="np-interval">{{ t("achats.form.interval") }}</label>
          <select
            id="np-interval"
            v-model="form.intervalKind"
            class="select"
            :disabled="anchorLocked"
          >
            <option value="weekly">{{ t("achats.interval.weekly") }}</option>
            <option value="monthly">{{ t("achats.interval.monthly") }}</option>
            <option value="custom">{{ t("achats.interval.custom") }}</option>
          </select>
        </div>
        <div class="field">
          <label for="np-days" :class="{ disabled: form.intervalKind !== 'custom' }">{{
            t("achats.form.intervalDays")
          }}</label>
          <input
            id="np-days"
            v-model.number="form.intervalDays"
            type="number"
            min="1"
            class="input"
            :disabled="anchorLocked || form.intervalKind !== 'custom'"
            :class="{ 'input--error': errors.intervalDays }"
          />
        </div>
      </div>

      <div class="installments">
        <div class="inst-head">
          <span class="field-label">{{ t("achats.form.installments") }}</span>
          <button class="btn btn--ghost btn--sm" type="button" @click="recompute">
            {{ t("achats.form.recompute") }}
          </button>
        </div>
        <div class="inst-rows">
          <!-- A settled tranche is shown but not editable: seeing what it is
               worth is the point, and the backend refuses to move it anyway. -->
          <div v-for="(r, i) in rows" :key="i" class="inst-row" :class="{ locked: r.locked }">
            <span class="inst-idx tabular">{{ i + 1 }}/{{ rows.length }}</span>
            <input
              v-model.number="r.amount"
              type="number"
              :min="r.paidAmount"
              class="input inst-amount"
              :disabled="r.locked"
              @change="onAmountEdit(i)"
            />
            <DatePicker v-model="r.dueDate" :disabled="r.locked" />
            <AppIcon
              v-if="r.locked"
              name="check"
              :size="15"
              class="inst-lock"
              :aria-label="t('achats.form.rowLocked')"
            />
          </div>
        </div>
        <div
          class="inst-sum"
          :class="{ ok: sumMatches && !belowPaid, bad: !sumMatches || belowPaid }"
        >
          <span
            >{{ t("achats.form.sumLabel") }}:
            <strong class="tabular">{{ fmt.money(sum) }}</strong></span
          >
          <span v-if="!sumMatches">{{
            t("achats.form.sumMismatch", {
              sum: fmt.number(sum),
              total: fmt.number(form.totalPrice ?? 0),
            })
          }}</span>
          <span v-else-if="belowPaid">{{
            t("errors.belowPaid", { paidAmount: belowPaid.paidAmount })
          }}</span>
          <span v-else>✓ {{ t("achats.form.sumOk") }}</span>
        </div>
      </div>
    </form>

    <template #footer>
      <button class="btn btn--ghost" type="button" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
      <button class="btn btn--primary" type="button" :disabled="saving" @click="submit">
        {{ editing ? t("achats.form.saveEdit") : t("achats.form.submit") }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.purchase-form {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.grid-2 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
}
.grid-4 {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 14px;
}
.locked-note {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--warning-bg);
  color: var(--warning-text);
  font-size: 13px;
  line-height: 1.45;
}
.inline-client {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 16px;
  background: var(--bg);
  border-radius: 12px;
}
.field-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-secondary);
}
label.disabled {
  color: var(--text-muted);
}
.installments {
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 14px 16px;
}
.inst-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
.inst-rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-height: 240px;
  overflow-y: auto;
}
.inst-row {
  display: grid;
  /* The last track holds the settled marker, and stays reserved on every row so
     the amount and date columns line up whether or not one is locked. */
  grid-template-columns: 60px 1fr 1fr 18px;
  gap: 10px;
  align-items: center;
}
.inst-row.locked {
  opacity: 0.7;
}
.inst-lock {
  color: var(--success);
}
.inst-idx {
  font-weight: 600;
  color: var(--text-secondary);
  font-size: 13px;
}
.inst-sum {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px dashed var(--border-strong);
  font-size: 13px;
}
.inst-sum.ok {
  color: var(--success);
}
.inst-sum.bad {
  color: var(--danger-strong);
}
</style>
