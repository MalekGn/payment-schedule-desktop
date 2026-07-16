<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import { useFormat } from "@/composables/useFormat";
import type { PurchaseSummary } from "@/types/models";

defineProps<{ purchases: PurchaseSummary[] }>();
const { t } = useI18n();
const fmt = useFormat();
const router = useRouter();

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
          <th>{{ t("dashboard.table.reference") }}</th>
          <th>{{ t("dashboard.table.client") }}</th>
          <th>{{ t("dashboard.table.product") }}</th>
          <th>{{ t("dashboard.table.purchaseDate") }}</th>
          <th>{{ t("dashboard.table.totalAmount") }}</th>
          <th>{{ t("common.status") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="p in purchases" :key="p.id">
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
