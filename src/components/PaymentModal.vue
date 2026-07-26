<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import BaseModal from "@/components/ui/BaseModal.vue";
import DatePicker from "@/components/ui/DatePicker.vue";
import { useFormat } from "@/composables/useFormat";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { api } from "@/api";
import { todayIso } from "@/lib/finance";
import type { Installment, PurchaseDetail } from "@/types/models";

const props = defineProps<{
  installment: Installment;
  installmentCount: number;
  purchaseReference: string;
}>();
const emit = defineEmits<{ close: []; saved: [detail: PurchaseDetail] }>();

const { t } = useI18n();
const fmt = useFormat();
const ui = useUiStore();
const stats = useStatsStore();

const remaining = computed(() => props.installment.amount - props.installment.paidAmount);
const amount = ref<number | null>(remaining.value);
const paymentDate = ref(todayIso());
const note = ref("");
const error = ref("");
const saving = ref(false);

async function submit() {
  error.value = "";
  const value = Number(amount.value);
  if (!Number.isFinite(value) || value <= 0) {
    error.value = t("validation.positive");
    return;
  }
  saving.value = true;
  try {
    const detail = await api.recordPayment({
      installmentId: props.installment.id,
      amount: Math.round(value),
      paymentDate: paymentDate.value,
      note: note.value.trim() || null,
    });
    await stats.refresh();
    ui.notify(t("common.save"));
    emit("saved", detail);
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <BaseModal
    :title="t('paiements.recordFor', { index: installment.index, count: installmentCount })"
    @close="emit('close')"
  >
    <div class="pay-grid">
      <div class="pay-info">
        <span class="muted">{{ purchaseReference }}</span>
        <div class="pay-figures">
          <div>
            <span class="pay-fig-label">{{ t("paiements.dueAmount") }}</span>
            <span class="pay-fig tabular">{{ fmt.money(installment.amount) }}</span>
          </div>
          <div>
            <span class="pay-fig-label">{{ t("paiements.alreadyPaid") }}</span>
            <span class="pay-fig tabular">{{ fmt.money(installment.paidAmount) }}</span>
          </div>
          <div>
            <span class="pay-fig-label">{{ t("common.remaining") }}</span>
            <span class="pay-fig tabular strong">{{ fmt.money(remaining) }}</span>
          </div>
        </div>
      </div>

      <form class="pay-form" @submit.prevent="submit">
        <div class="field">
          <label for="pay-amount">{{ t("paiements.amount") }}</label>
          <input
            id="pay-amount"
            v-model="amount"
            type="number"
            min="1"
            class="input"
            :class="{ 'input--error': error }"
            autofocus
          />
          <span v-if="error" class="field-error">{{ error }}</span>
        </div>
        <div class="field">
          <label>{{ t("paiements.date") }}</label>
          <DatePicker v-model="paymentDate" />
        </div>
        <div class="field">
          <label for="pay-note"
            >{{ t("paiements.note") }}
            <span class="muted">({{ t("common.optional") }})</span></label
          >
          <input id="pay-note" v-model="note" type="text" class="input" />
        </div>
        <p class="partial-info">{{ t("paiements.partialInfo") }}</p>
      </form>
    </div>

    <template #footer>
      <button class="btn btn--ghost" type="button" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
      <button class="btn btn--primary" type="button" :disabled="saving" @click="submit">
        {{ t("common.save") }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.pay-grid {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.pay-info {
  background: var(--bg);
  border-radius: 12px;
  padding: 16px;
}
.pay-figures {
  display: flex;
  gap: 24px;
  margin-top: 10px;
}
.pay-fig-label {
  display: block;
  font-size: 12px;
  color: var(--text-muted);
}
.pay-fig {
  font-size: 15px;
  font-weight: 600;
}
.pay-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.partial-info {
  font-size: 12.5px;
  color: var(--text-muted);
  line-height: 1.4;
}
</style>
