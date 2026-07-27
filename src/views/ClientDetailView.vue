<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import { useFormat } from "@/composables/useFormat";
import { useSort } from "@/composables/useSort";
import { useBack } from "@/composables/useBack";
import { useUiStore } from "@/stores/ui";
import { api } from "@/api";
import type { ClientDetail, Payment } from "@/types/models";

const props = defineProps<{ id: string }>();
const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();
const ui = useUiStore();
const goBack = useBack("/clients");

const detail = ref<ClientDetail | null>(null);
const payments = ref<Payment[]>([]);
const notFound = ref(false);

const { sort: purchaseSort, sorted: sortedPurchases } = useSort(
  computed(() => detail.value?.purchases ?? []),
  {
    reference: (p) => p.reference,
    product: (p) => p.productLabel,
    date: (p) => p.purchaseDate,
    total: (p) => p.totalPrice,
    remaining: (p) => p.remaining,
    status: (p) => p.status,
  },
);

const { sort: paymentSort, sorted: sortedPayments } = useSort(payments, {
  date: (p) => p.paymentDate,
  reference: (p) => p.purchaseReference,
  tranche: (p) => p.installmentIndex,
  amount: (p) => p.amount,
  note: (p) => p.note,
});

async function load() {
  const clientId = Number(props.id);
  try {
    detail.value = await api.getClientDetail(clientId);
    payments.value = await api.listPaymentsForClient(clientId);
    ui.pageTitle = `${detail.value.client.firstName} ${detail.value.client.lastName}`;
  } catch {
    notFound.value = true;
  }
}
onMounted(load);
</script>

<template>
  <div v-if="notFound" class="page">
    <button class="back-link" type="button" @click="goBack">
      <AppIcon name="arrow-left" :size="16" class="icon-flip" /> {{ t("common.back") }}
    </button>
    <div class="card">
      <EmptyState icon="users" :title="t('notFound.clientMissing')" />
    </div>
  </div>

  <div v-else-if="detail" class="page">
    <button class="back-link" type="button" @click="goBack">
      <AppIcon name="arrow-left" :size="16" class="icon-flip" /> {{ t("common.back") }}
    </button>

    <div class="top-grid">
      <section class="card contact-card">
        <div class="card-header">
          <h2>{{ t("clients.detail.contact") }}</h2>
        </div>
        <div class="contact-body">
          <div class="contact-avatar">
            {{ detail.client.firstName.charAt(0) }}{{ detail.client.lastName.charAt(0) }}
          </div>
          <div class="contact-info">
            <span class="contact-name"
              >{{ detail.client.firstName }} {{ detail.client.lastName }}</span
            >
            <!-- Reachable by deep link or from a purchase, so an archived
                 client must not read as active here. Archive/restore stay on
                 the list view; this page has no mutating actions. -->
            <span
              v-if="detail.client.archivedAt"
              class="badge badge--pending archived-badge"
              :title="t('clients.archivedOn', { date: fmt.date(detail.client.archivedAt) })"
            >
              {{ t("clients.archivedBadge") }}
            </span>
            <span v-if="detail.client.phone" class="contact-line">
              <AppIcon name="phone" :size="15" /> {{ detail.client.phone }}
            </span>
            <span v-if="detail.client.address" class="contact-line">
              <AppIcon name="map-pin" :size="15" /> {{ detail.client.address }}
            </span>
            <span v-if="detail.client.email" class="contact-line">
              <AppIcon name="mail" :size="15" /> {{ detail.client.email }}
            </span>
          </div>
        </div>
      </section>

      <div class="figures">
        <div class="fig-card card">
          <span class="fig-label">{{ t("clients.detail.totalPurchased") }}</span>
          <span class="fig-value tabular">{{ fmt.money(detail.totalPurchased) }}</span>
        </div>
        <div class="fig-card card">
          <span class="fig-label">{{ t("clients.detail.totalPaid") }}</span>
          <span class="fig-value tabular" style="color: var(--success)">{{
            fmt.money(detail.totalPaid)
          }}</span>
        </div>
        <div class="fig-card card">
          <span class="fig-label">{{ t("clients.detail.outstanding") }}</span>
          <span class="fig-value tabular" style="color: var(--warning-text)">{{
            fmt.money(detail.totalOutstanding)
          }}</span>
        </div>
      </div>
    </div>

    <section class="card">
      <div class="card-header">
        <h2>{{ t("clients.detail.purchases") }}</h2>
      </div>
      <EmptyState
        v-if="detail.purchases.length === 0"
        icon="cart"
        :title="t('clients.detail.noPurchases')"
      />
      <table v-else class="table">
        <thead>
          <tr>
            <SortHeader
              :sort="purchaseSort"
              field="reference"
              :label="t('dashboard.table.reference')"
            />
            <SortHeader :sort="purchaseSort" field="product" :label="t('common.product')" />
            <SortHeader :sort="purchaseSort" field="date" :label="t('common.date')" />
            <SortHeader :sort="purchaseSort" field="total" :label="t('common.total')" />
            <SortHeader
              :sort="purchaseSort"
              field="remaining"
              :label="t('achats.columns.remaining')"
            />
            <SortHeader :sort="purchaseSort" field="status" :label="t('common.status')" />
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="p in sortedPurchases"
            :key="p.id"
            class="clickable"
            @click="router.push({ name: 'achat-detail', params: { id: p.id } })"
          >
            <td>
              <span class="row-link">{{ p.reference }}</span>
            </td>
            <td class="ellipsis">{{ p.productLabel }}</td>
            <td class="tabular">{{ fmt.date(p.purchaseDate) }}</td>
            <td class="tabular">{{ fmt.money(p.totalPrice) }}</td>
            <td class="tabular strong">{{ fmt.money(p.remaining) }}</td>
            <td><StatusBadge :status="p.status" /></td>
          </tr>
        </tbody>
      </table>
    </section>

    <section class="card">
      <div class="card-header">
        <h2>{{ t("clients.detail.paymentHistory") }}</h2>
      </div>
      <EmptyState
        v-if="payments.length === 0"
        icon="card"
        :title="t('clients.detail.noPayments')"
      />
      <table v-else class="table">
        <thead>
          <tr>
            <SortHeader :sort="paymentSort" field="date" :label="t('paiements.columns.date')" />
            <SortHeader
              :sort="paymentSort"
              field="reference"
              :label="t('paiements.columns.reference')"
            />
            <SortHeader
              :sort="paymentSort"
              field="tranche"
              :label="t('paiements.columns.tranche')"
            />
            <SortHeader :sort="paymentSort" field="amount" :label="t('paiements.columns.amount')" />
            <SortHeader :sort="paymentSort" field="note" :label="t('paiements.columns.note')" />
          </tr>
        </thead>
        <tbody>
          <tr v-for="pay in sortedPayments" :key="pay.id">
            <td class="tabular">{{ fmt.date(pay.paymentDate) }}</td>
            <td>
              <span class="row-link">{{ pay.purchaseReference }}</span>
            </td>
            <td class="tabular">{{ pay.installmentIndex }}</td>
            <td class="tabular strong">{{ fmt.money(pay.amount) }}</td>
            <td class="muted">{{ pay.note || "—" }}</td>
          </tr>
        </tbody>
      </table>
    </section>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.back-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: transparent;
  border: none;
  color: var(--text-secondary);
  font-weight: 600;
  font-size: 13.5px;
  align-self: flex-start;
}
.back-link:hover {
  color: var(--primary);
}
.top-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
  gap: 18px;
  align-items: stretch;
}
.contact-body {
  display: flex;
  gap: 16px;
  padding: 4px 22px 22px;
}
.contact-avatar {
  width: 60px;
  height: 60px;
  border-radius: 16px;
  background: linear-gradient(135deg, #6366f1, #2563eb);
  color: #fff;
  font-weight: 700;
  font-size: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.contact-info {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.contact-name {
  font-size: 16px;
  font-weight: 700;
}
.archived-badge {
  /* The contact info is a column, so keep the badge at its own width. */
  align-self: flex-start;
  font-size: 11.5px;
  padding: 2px 8px;
}
.contact-line {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13.5px;
  color: var(--text-secondary);
}
.contact-line :deep(.app-icon) {
  color: var(--text-muted);
}
.figures {
  display: grid;
  grid-template-rows: repeat(3, 1fr);
  gap: 18px;
}
.fig-card {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 4px;
  padding: 16px 22px;
}
.fig-label {
  font-size: 13px;
  color: var(--text-secondary);
}
.fig-value {
  font-size: 20px;
  font-weight: 700;
}
.clickable {
  cursor: pointer;
}
.clickable:hover {
  background: var(--bg);
}
.ellipsis {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
@media (max-width: 1000px) {
  .top-grid {
    grid-template-columns: 1fr;
  }
  .figures {
    grid-template-rows: none;
    grid-template-columns: repeat(3, 1fr);
  }
}
</style>
