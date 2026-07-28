<script setup lang="ts">
// Shared filter bar for list pages (Purchases, Payments, Due dates): a
// prominent free-text search (matches reference + client, like the Purchases
// page), an optional amount min/max range, and a From/To date range (via
// DatePicker). Every control is a v-model so the parent owns the values and
// applies the filtering. "Reset" clears them all.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import DatePicker from "@/components/ui/DatePicker.vue";

defineProps<{
  showAmount?: boolean;
  searchPlaceholder?: string;
}>();

const search = defineModel<string>("search", { default: "" });
const amountMin = defineModel<string>("amountMin", { default: "" });
const amountMax = defineModel<string>("amountMax", { default: "" });
const dateFrom = defineModel<string>("dateFrom", { default: "" });
const dateTo = defineModel<string>("dateTo", { default: "" });

const { t } = useI18n();

const isActive = computed(() =>
  Boolean(search.value || amountMin.value || amountMax.value || dateFrom.value || dateTo.value),
);

function reset() {
  search.value = "";
  amountMin.value = "";
  amountMax.value = "";
  dateFrom.value = "";
  dateTo.value = "";
}
</script>

<template>
  <div class="filter-bar">
    <div class="field field--search">
      <div class="search-box">
        <AppIcon name="search" :size="18" class="muted" />
        <input
          v-model="search"
          class="search-input"
          :placeholder="searchPlaceholder ?? t('filters.searchHint')"
        />
      </div>
    </div>

    <div v-if="showAmount" class="field">
      <label>{{ t("common.amount") }}</label>
      <div class="range-inputs">
        <input
          v-model="amountMin"
          type="number"
          min="0"
          class="input input--num"
          :placeholder="t('filters.min')"
        />
        <span class="range-sep">–</span>
        <input
          v-model="amountMax"
          type="number"
          min="0"
          class="input input--num"
          :placeholder="t('filters.max')"
        />
      </div>
    </div>

    <div class="field">
      <label>{{ t("filters.date") }}</label>
      <div class="range-inputs">
        <DatePicker
          v-model="dateFrom"
          :max="dateTo || undefined"
          :placeholder="t('filters.from')"
        />
        <span class="range-sep">–</span>
        <DatePicker v-model="dateTo" :min="dateFrom || undefined" :placeholder="t('filters.to')" />
      </div>
    </div>

    <button class="btn btn--ghost reset-btn" type="button" :disabled="!isActive" @click="reset">
      <AppIcon name="x" :size="15" /> {{ t("filters.reset") }}
    </button>
  </div>
</template>

<style scoped>
.filter-bar {
  display: flex;
  gap: 16px;
  align-items: flex-end;
  flex-wrap: wrap;
  padding: 16px 22px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.field > label {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-secondary);
}
.field--search {
  flex: 1;
  min-width: 220px;
  max-width: 460px;
}
.search-box {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 14px;
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
}
.search-input {
  border: none;
  outline: none;
  background: transparent;
  flex: 1;
  color: var(--text);
}
.range-inputs {
  display: flex;
  align-items: center;
  gap: 8px;
}
.range-sep {
  color: var(--text-muted);
}
.input--num {
  width: 92px;
}
.reset-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-inline-start: auto;
}
</style>
