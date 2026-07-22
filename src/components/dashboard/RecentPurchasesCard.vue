<script setup lang="ts">
import { toRef } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import { useFormat } from "@/composables/useFormat";
import { useSort } from "@/composables/useSort";
import type { PurchaseSummary } from "@/types/models";

const props = defineProps<{ purchases: PurchaseSummary[] }>();
const { t } = useI18n();
const fmt = useFormat();
const router = useRouter();

const { sort, sorted } = useSort(toRef(props, "purchases"), {
  reference: (p) => p.reference,
  client: (p) => p.clientName,
  product: (p) => p.productLabel,
  purchaseDate: (p) => p.purchaseDate,
  total: (p) => p.totalPrice,
  status: (p) => p.status,
});

function open(id: number) {
  router.push({ name: "achat-detail", params: { id } });
}
</script>

<template>
  <section class="card">
    <div class="card-header">
      <h2>{{ t("dashboard.recentPurchases") }}</h2>
      <RouterLink class="card-link" to="/achats">{{ t("common.viewAll") }}</RouterLink>
    </div>

    <EmptyState v-if="purchases.length === 0" icon="cart" :title="t('dashboard.empty.purchases')" />

    <table v-else class="table recent-table">
      <thead>
        <tr>
          <SortHeader :sort="sort" field="reference" :label="t('dashboard.table.reference')" />
          <SortHeader :sort="sort" field="client" :label="t('dashboard.table.client')" />
          <SortHeader :sort="sort" field="product" :label="t('dashboard.table.product')" />
          <SortHeader :sort="sort" field="purchaseDate" :label="t('dashboard.table.purchaseDate')" />
          <SortHeader :sort="sort" field="total" :label="t('dashboard.table.totalAmount')" />
          <SortHeader :sort="sort" field="status" :label="t('common.status')" />
        </tr>
      </thead>
      <tbody>
        <tr v-for="p in sorted" :key="p.id">
          <td>
            <a class="row-link" href="#" @click.prevent="open(p.id)">{{ p.reference }}</a>
          </td>
          <td>{{ p.clientName }}</td>
          <td class="ellipsis">{{ p.productLabel }}</td>
          <td class="tabular">{{ fmt.date(p.purchaseDate) }}</td>
          <td class="tabular">{{ fmt.money(p.totalPrice) }}</td>
          <td><StatusBadge :status="p.status" /></td>
        </tr>
      </tbody>
    </table>
  </section>
</template>

<style scoped>
/* Keep every column on a single line; only the product truncates. */
.recent-table td {
  white-space: nowrap;
}
.ellipsis {
  max-width: 170px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
