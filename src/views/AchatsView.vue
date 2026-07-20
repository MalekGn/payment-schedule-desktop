<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import DatePicker from "@/components/ui/DatePicker.vue";
import NewPurchaseModal from "@/components/NewPurchaseModal.vue";
import { useFormat } from "@/composables/useFormat";
import { useSort } from "@/composables/useSort";
import { api } from "@/api";
import type { PurchaseDetail, PurchaseSummary } from "@/types/models";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const fmt = useFormat();

const purchases = ref<PurchaseSummary[]>([]);
const search = ref("");
const dateFrom = ref("");
const dateTo = ref("");
const loading = ref(true);
const showModal = ref(false);

const filtered = computed(() => {
  const n = search.value.trim().toLowerCase();
  return purchases.value.filter((p) => {
    if (n && !`${p.reference} ${p.clientName} ${p.productLabel}`.toLowerCase().includes(n)) {
      return false;
    }
    if (dateFrom.value && p.purchaseDate < dateFrom.value) return false;
    if (dateTo.value && p.purchaseDate > dateTo.value) return false;
    return true;
  });
});

const { sort, sorted } = useSort(filtered, {
  reference: (p) => p.reference,
  client: (p) => p.clientName,
  product: (p) => p.productLabel,
  date: (p) => p.purchaseDate,
  total: (p) => p.totalPrice,
  remaining: (p) => p.remaining,
  status: (p) => p.status,
});

async function load() {
  loading.value = true;
  purchases.value = await api.listPurchases();
  loading.value = false;
}

onMounted(() => {
  load();
  if (route.query.new === "1") {
    showModal.value = true;
    router.replace({ query: {} });
  }
});

function onSaved(detail: PurchaseDetail) {
  showModal.value = false;
  router.push({ name: "achat-detail", params: { id: detail.purchase.id } });
}
</script>

<template>
  <div class="page">
    <div class="toolbar">
      <div class="search-box">
        <AppIcon name="search" :size="18" class="muted" />
        <input v-model="search" class="search-input" :placeholder="t('achats.searchPlaceholder')" />
      </div>
      <div class="date-range">
        <DatePicker v-model="dateFrom" :max="dateTo || undefined" :placeholder="t('filters.from')" />
        <span class="range-sep">–</span>
        <DatePicker v-model="dateTo" :min="dateFrom || undefined" :placeholder="t('filters.to')" />
      </div>
      <button class="btn btn--primary" type="button" @click="showModal = true">
        <AppIcon name="plus" :size="18" /> {{ t("achats.new") }}
      </button>
    </div>

    <div class="card">
      <EmptyState v-if="!loading && purchases.length === 0" icon="cart" :title="t('achats.empty')" />
      <table v-else class="table">
        <thead>
          <tr>
            <SortHeader :sort="sort" field="reference" :label="t('achats.columns.reference')" />
            <SortHeader :sort="sort" field="client" :label="t('achats.columns.client')" />
            <SortHeader :sort="sort" field="product" :label="t('achats.columns.product')" />
            <SortHeader :sort="sort" field="date" :label="t('achats.columns.date')" />
            <SortHeader :sort="sort" field="total" :label="t('achats.columns.total')" />
            <SortHeader :sort="sort" field="remaining" :label="t('achats.columns.remaining')" />
            <SortHeader :sort="sort" field="status" :label="t('achats.columns.status')" />
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="p in sorted"
            :key="p.id"
            class="clickable"
            @click="router.push({ name: 'achat-detail', params: { id: p.id } })"
          >
            <td><span class="row-link">{{ p.reference }}</span></td>
            <td>{{ p.clientName }}</td>
            <td class="ellipsis">{{ p.productLabel }}</td>
            <td class="tabular">{{ fmt.date(p.purchaseDate) }}</td>
            <td class="tabular">{{ fmt.money(p.totalPrice) }}</td>
            <td class="tabular strong">{{ fmt.money(p.remaining) }}</td>
            <td><StatusBadge :status="p.status" /></td>
          </tr>
        </tbody>
      </table>
    </div>

    <NewPurchaseModal v-if="showModal" @close="showModal = false" @saved="onSaved" />
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 14px;
}
.search-box {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  max-width: 460px;
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
.date-range {
  display: flex;
  align-items: center;
  gap: 8px;
}
.range-sep {
  color: var(--text-muted);
}
.clickable {
  cursor: pointer;
}
.clickable:hover {
  background: var(--bg);
}
.ellipsis {
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
