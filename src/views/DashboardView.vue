<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import KpiCard from "@/components/ui/KpiCard.vue";
import RecentPurchasesCard from "@/components/dashboard/RecentPurchasesCard.vue";
import PurchaseDetailCard from "@/components/dashboard/PurchaseDetailCard.vue";
import DueAlertsCard from "@/components/dashboard/DueAlertsCard.vue";
import ImpayesPanelCard from "@/components/dashboard/ImpayesPanelCard.vue";
import PaymentModal from "@/components/PaymentModal.vue";
import LoadError from "@/components/ui/LoadError.vue";
import { useFormat } from "@/composables/useFormat";
import { useLoader } from "@/composables/useLoader";
import { api } from "@/api";
import type { Dashboard, Installment } from "@/types/models";

const { t } = useI18n();
const fmt = useFormat();

const data = ref<Dashboard | null>(null);
const payTarget = ref<Installment | null>(null);

const { error: loadError, run: load } = useLoader(async () => {
  data.value = await api.getDashboard();
});
onMounted(load);

const kpis = computed(() => {
  const s = data.value?.stats;
  if (!s) return [];
  return [
    {
      icon: "cart",
      tone: "blue" as const,
      label: t("dashboard.kpi.totalPurchases"),
      value: fmt.number(s.totalPurchases),
      sub: t("dashboard.thisMonth"),
    },
    {
      icon: "banknote",
      tone: "green" as const,
      label: t("dashboard.kpi.totalSales"),
      value: fmt.money(s.totalSales),
      sub: t("dashboard.thisMonth"),
    },
    {
      icon: "card",
      tone: "purple" as const,
      label: t("dashboard.kpi.collected"),
      value: fmt.money(s.totalCollected),
      sub: t("dashboard.thisMonth"),
    },
    {
      icon: "alert",
      tone: "orange" as const,
      label: t("dashboard.kpi.outstanding"),
      value: fmt.money(s.totalOutstanding),
      sub: t("dashboard.thisMonth"),
    },
    {
      icon: "calendar",
      tone: "red" as const,
      label: t("dashboard.kpi.latePayments"),
      value: fmt.number(s.overdueCount),
      sub: t("dashboard.clientsConcerned"),
    },
  ];
});

function onSaved(detail: Dashboard["featuredPurchase"]) {
  payTarget.value = null;
  if (detail && data.value) data.value.featuredPurchase = detail;
  load();
}
</script>

<template>
  <LoadError v-if="loadError" :message="loadError" @retry="load" />
  <div v-else-if="data" class="dashboard">
    <div class="kpi-row">
      <KpiCard
        v-for="(k, i) in kpis"
        :key="i"
        :icon="k.icon"
        :tone="k.tone"
        :label="k.label"
        :value="k.value"
        :sub="k.sub"
      />
    </div>

    <div class="dash-grid">
      <div class="dash-col dash-col--main">
        <RecentPurchasesCard :purchases="data.recentPurchases" />
        <PurchaseDetailCard
          v-if="data.featuredPurchase"
          :detail="data.featuredPurchase"
          @pay="payTarget = $event"
        />
      </div>
      <div class="dash-col dash-col--side">
        <DueAlertsCard :alerts="data.dueAlerts" />
        <ImpayesPanelCard :impayes="data.impayes" />
      </div>
    </div>

    <PaymentModal
      v-if="payTarget && data.featuredPurchase"
      :installment="payTarget"
      :installment-count="data.featuredPurchase.purchase.installmentCount"
      :purchase-reference="data.featuredPurchase.purchase.reference"
      @close="payTarget = null"
      @saved="onSaved"
    />
  </div>
  <div v-else class="loading">{{ t("common.loading") }}</div>
</template>

<style scoped>
.dashboard {
  display: flex;
  flex-direction: column;
  gap: 20px;
}
.kpi-row {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 16px;
}
.dash-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.8fr) minmax(0, 1fr);
  gap: 20px;
  align-items: start;
}
.dash-col {
  display: flex;
  flex-direction: column;
  gap: 20px;
  min-width: 0;
}
.loading {
  padding: 40px;
  color: var(--text-muted);
}

@media (max-width: 1200px) {
  .kpi-row {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .dash-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
