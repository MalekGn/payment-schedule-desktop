<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import BaseModal from "@/components/ui/BaseModal.vue";
import DatePicker from "@/components/ui/DatePicker.vue";
import { useFormat } from "@/composables/useFormat";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { api } from "@/api";
import { addInterval, splitAmounts, todayIso } from "@/lib/finance";
import type { ClientSummary, IntervalKind, PurchaseDetail } from "@/types/models";

const emit = defineEmits<{ close: []; saved: [detail: PurchaseDetail] }>();

const { t } = useI18n();
const fmt = useFormat();
const ui = useUiStore();
const stats = useStatsStore();

const clients = ref<ClientSummary[]>([]);
const saving = ref(false);

const form = reactive({
  clientId: "" as string, // "" = none, "new" = inline
  productLabel: "",
  totalPrice: null as number | null,
  installmentCount: 3,
  intervalKind: "monthly" as IntervalKind,
  intervalDays: 30,
  purchaseDate: todayIso(),
});
const inlineClient = reactive({ firstName: "", lastName: "", phone: "", address: "", email: "" });

interface Row {
  amount: number;
  dueDate: string;
}
const rows = ref<Row[]>([]);
const manualAmounts = ref(false);
const errors = reactive<Record<string, string>>({});

onMounted(async () => {
  clients.value = await api.listClients();
  rebuild();
});

function rebuild() {
  const count = Math.max(1, Math.floor(form.installmentCount || 1));
  const total = Math.max(0, Math.round(form.totalPrice ?? 0));
  const amounts =
    manualAmounts.value && rows.value.length === count
      ? rows.value.map((r) => r.amount)
      : splitAmounts(total, count);
  rows.value = Array.from({ length: count }, (_, i) => ({
    amount: amounts[i] ?? 0,
    dueDate: addInterval(form.purchaseDate, form.intervalKind, form.intervalDays, i),
  }));
}

watch(
  () => [
    form.totalPrice,
    form.installmentCount,
    form.intervalKind,
    form.intervalDays,
    form.purchaseDate,
  ],
  rebuild,
);

function onAmountEdit() {
  manualAmounts.value = true;
}
function recompute() {
  manualAmounts.value = false;
  rebuild();
}

const sum = computed(() => rows.value.reduce((s, r) => s + (Number(r.amount) || 0), 0));
const sumMatches = computed(() => sum.value === Math.round(form.totalPrice ?? 0));

function validate(): boolean {
  for (const k of Object.keys(errors)) delete errors[k];
  if (form.clientId === "new") {
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
  return Object.keys(errors).length === 0;
}

async function submit() {
  if (!validate()) return;
  saving.value = true;
  try {
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

    const detail = await api.createPurchase({
      clientId,
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
    });
    await stats.refresh();
    ui.notify(t("common.save"));
    emit("saved", detail);
  } catch (e) {
    ui.notify(String(e), "error");
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <BaseModal :title="t('achats.form.title')" wide @close="emit('close')">
    <form class="purchase-form" @submit.prevent="submit">
      <div class="grid-2">
        <div class="field">
          <label for="np-client">{{ t("achats.form.client") }}</label>
          <select
            id="np-client"
            v-model="form.clientId"
            class="select"
            :class="{ 'input--error': errors.client }"
          >
            <option value="" disabled>{{ t("achats.form.selectClient") }}</option>
            <option value="new">➕ {{ t("achats.form.newClient") }}</option>
            <option v-for="c in clients" :key="c.id" :value="String(c.id)">
              {{ c.firstName }} {{ c.lastName }}
            </option>
          </select>
          <span v-if="errors.client" class="field-error">{{ errors.client }}</span>
        </div>
        <div class="field">
          <label>{{ t("achats.form.purchaseDate") }}</label>
          <DatePicker v-model="form.purchaseDate" />
        </div>
      </div>

      <div v-if="form.clientId === 'new'" class="inline-client">
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
          />
          <span v-if="errors.installmentCount" class="field-error">{{
            errors.installmentCount
          }}</span>
        </div>
        <div class="field">
          <label for="np-interval">{{ t("achats.form.interval") }}</label>
          <select id="np-interval" v-model="form.intervalKind" class="select">
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
            :disabled="form.intervalKind !== 'custom'"
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
          <div v-for="(r, i) in rows" :key="i" class="inst-row">
            <span class="inst-idx tabular">{{ i + 1 }}/{{ rows.length }}</span>
            <input
              v-model.number="r.amount"
              type="number"
              min="0"
              class="input inst-amount"
              @input="onAmountEdit"
            />
            <DatePicker v-model="r.dueDate" />
          </div>
        </div>
        <div class="inst-sum" :class="{ ok: sumMatches, bad: !sumMatches }">
          <span
            >{{ t("achats.form.sumLabel") }}:
            <strong class="tabular">{{ fmt.money(sum) }}</strong></span
          >
          <span v-if="sumMatches">✓ {{ t("achats.form.sumOk") }}</span>
          <span v-else>{{
            t("achats.form.sumMismatch", {
              sum: fmt.number(sum),
              total: fmt.number(form.totalPrice ?? 0),
            })
          }}</span>
        </div>
      </div>
    </form>

    <template #footer>
      <button class="btn btn--ghost" type="button" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
      <button class="btn btn--primary" type="button" :disabled="saving" @click="submit">
        {{ t("achats.form.submit") }}
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
  grid-template-columns: 60px 1fr 1fr;
  gap: 10px;
  align-items: center;
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
