<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import NewPurchaseModal from "@/components/NewPurchaseModal.vue";
import { useFormat } from "@/composables/useFormat";
import { api } from "@/api";
import type { PurchaseDetail, PurchaseSummary } from "@/types/models";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const fmt = useFormat();

const purchases = ref<PurchaseSummary[]>([]);
const search = ref("");
const loading = ref(true);
const showModal = ref(false);

const filtered = computed(() => {
  const n = search.value.trim().toLowerCase();
  if (!n) return purchases.value;
  return purchases.value.filter((p) =>
    `${p.reference} ${p.clientName} ${p.productLabel}`.toLowerCase().includes(n),
  );
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
      <button class="btn btn--primary" type="button" @click="showModal = true">
        <AppIcon name="plus" :size="18" /> {{ t("achats.new") }}
      </button>
    </div>

    <div class="card">
      <EmptyState v-if="!loading && purchases.length === 0" icon="cart" :title="t('achats.empty')" />
      <table v-else class="table">
        <thead>
          <tr>
            <th>{{ t("achats.columns.reference") }}</th>
            <th>{{ t("achats.columns.client") }}</th>
            <th>{{ t("achats.columns.product") }}</th>
            <th>{{ t("achats.columns.date") }}</th>
            <th>{{ t("achats.columns.total") }}</th>
            <th>{{ t("achats.columns.remaining") }}</th>
            <th>{{ t("achats.columns.status") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="p in filtered"
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
