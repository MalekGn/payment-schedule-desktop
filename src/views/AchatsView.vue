<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import StatusBadge from "@/components/ui/StatusBadge.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import ListFilterBar from "@/components/ui/ListFilterBar.vue";
import NewPurchaseModal from "@/components/NewPurchaseModal.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import LoadError from "@/components/ui/LoadError.vue";
import { useFormat } from "@/composables/useFormat";
import { useLoader } from "@/composables/useLoader";
import { useSort } from "@/composables/useSort";
import { parseErrorCode, toUserMessage } from "@/lib/errors";
import { useUiStore } from "@/stores/ui";
import { useLicenseStore } from "@/stores/license";
import { useStatsStore } from "@/stores/stats";
import { api } from "@/api";
import type { PurchaseDetail, PurchaseScope, PurchaseSummary } from "@/types/models";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const fmt = useFormat();
const ui = useUiStore();
const license = useLicenseStore();
const stats = useStatsStore();

const purchases = ref<PurchaseSummary[]>([]);
const search = ref("");
const dateFrom = ref("");
const dateTo = ref("");
const showModal = ref(false);
/** The purchase being edited, or `null` when the modal is creating. */
const editing = ref<PurchaseDetail | null>(null);

/** Same segmented-tabs shape as the Clients page. */
const SCOPES = [
  { key: "active", label: "achats.scope.active" },
  { key: "archived", label: "achats.scope.archived" },
  { key: "all", label: "achats.scope.all" },
] as const;
const scope = ref<PurchaseScope>("active");

type PendingKind = "archive" | "restore" | "delete";
const pending = ref<{ kind: PendingKind; purchase: PurchaseSummary } | null>(null);

/**
 * Payment count reported by the backend when it refused an archive.
 *
 * The row's `paidAmount` normally predicts this, so the dialog opens already
 * blocked. This covers the stale-list race where it did not. Cleared whenever a
 * new dialog opens.
 */
const serverPayments = ref<number | null>(null);
watch(pending, () => {
  serverPayments.value = null;
});

/** How many payments block archiving the pending purchase, or `null`. */
const archiveBlockedBy = computed<number | null>(() => {
  const p = pending.value;
  if (!p || p.kind !== "archive") return null;
  if (serverPayments.value !== null) return serverPayments.value;
  // The row carries money, not a count; any paid amount means at least one.
  return p.purchase.paidAmount > 0 ? 1 : null;
});

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

const {
  loading,
  error: loadError,
  run: load,
} = useLoader(async () => {
  purchases.value = await api.listPurchases(scope.value);
});
// The scope is a server-side filter; the search below stays client-side.
watch(scope, load);

onMounted(() => {
  load();
  if (route.query.new === "1") {
    showModal.value = true;
    router.replace({ query: {} });
  }
});

function openNew() {
  editing.value = null;
  showModal.value = true;
}

async function openEdit(p: PurchaseSummary) {
  try {
    // The modal needs the full schedule, which the list row does not carry.
    editing.value = await api.getPurchaseDetail(p.id);
    showModal.value = true;
  } catch (e) {
    ui.notify(toUserMessage(e, t), "error");
  }
}

function closeModal() {
  showModal.value = false;
  editing.value = null;
}

async function onSaved(detail: PurchaseDetail) {
  const wasEditing = editing.value !== null;
  closeModal();
  if (wasEditing) {
    // Stay on the list: the user was working through it, not drilling in.
    await load();
    return;
  }
  router.push({ name: "achat-detail", params: { id: detail.purchase.id } });
}

/** Title, body, labels and blocked state for the open dialog. */
const dialog = computed(() => {
  const p = pending.value;
  if (!p) return null;
  if (p.kind === "archive") {
    const blocked = archiveBlockedBy.value;
    return {
      title: t("achats.archive.confirmTitle"),
      message: t("achats.archive.confirmText"),
      confirmLabel: t("achats.archive.action"),
      danger: false,
      // Explain up front rather than after a doomed confirm.
      warning: blocked ? t("achats.archive.hasPayments", { count: blocked }) : "",
      confirmDisabled: Boolean(blocked),
    };
  }
  if (p.kind === "restore") {
    return {
      title: t("achats.restore.confirmTitle"),
      message: t("achats.restore.confirmText"),
      confirmLabel: t("achats.restore.action"),
      danger: false,
      warning: "",
      confirmDisabled: false,
    };
  }
  return {
    title: t("achats.delete.confirmTitle"),
    message: t("achats.delete.confirmText"),
    confirmLabel: t("achats.delete.action"),
    danger: true,
    warning: "",
    confirmDisabled: false,
  };
});

/**
 * Archive the purchase, or record why the backend says we cannot.
 *
 * Returns `false` on a payments refusal so the caller keeps the dialog open;
 * it re-renders blocked with the count the database reports now.
 */
async function confirmArchive(target: PurchaseSummary): Promise<boolean> {
  try {
    await api.archivePurchase(target.id);
    ui.notify(t("achats.archive.action"));
    return true;
  } catch (e) {
    const parsed = parseErrorCode(e);
    if (parsed?.code === "PURCHASE_HAS_PAYMENTS") {
      serverPayments.value = Number(parsed.params[0] ?? 0);
      return false;
    }
    throw e;
  }
}

async function confirmPending() {
  const p = pending.value;
  if (!p || archiveBlockedBy.value) return;

  let keepOpen = false;
  try {
    if (p.kind === "archive") {
      keepOpen = !(await confirmArchive(p.purchase));
    } else if (p.kind === "restore") {
      await api.restorePurchase(p.purchase.id);
      ui.notify(t("achats.restore.action"));
    } else {
      await api.deletePurchase(p.purchase.id);
      ui.notify(t("common.delete"));
    }
  } catch (e) {
    ui.notify(toUserMessage(e, t), "error");
  }
  if (!keepOpen) pending.value = null;
  await load();
  await stats.refresh();
}
</script>

<template>
  <div class="page">
    <LoadError v-if="loadError" :message="loadError" @retry="load" />

    <template v-else>
      <div class="card">
        <div class="card-header">
          <h2>{{ t("achats.title") }}</h2>
          <div class="header-actions">
            <!-- Same segmented-tabs block as ClientsView/EcheancesView/AlertesView;
                 direction-agnostic (flex + gap, symmetric padding) so it mirrors
                 correctly under dir="rtl". -->
            <div class="tabs">
              <button
                v-for="sc in SCOPES"
                :key="sc.key"
                class="tab"
                :class="{ 'tab--active': scope === sc.key }"
                type="button"
                :disabled="sc.key !== 'active' && !license.isLicensed"
                :title="
                  sc.key !== 'active' && !license.isLicensed
                    ? t('license.requiredTitle')
                    : undefined
                "
                @click="scope = sc.key"
              >
                {{ t(sc.label) }}
              </button>
            </div>
            <button class="btn btn--primary" type="button" @click="openNew">
              <AppIcon name="plus" :size="18" /> {{ t("achats.new") }}
            </button>
          </div>
        </div>

        <ListFilterBar
          v-model:search="search"
          v-model:date-from="dateFrom"
          v-model:date-to="dateTo"
          :search-placeholder="t('achats.searchPlaceholder')"
        />

        <EmptyState
          v-if="!loading && purchases.length === 0"
          icon="cart"
          :title="scope === 'archived' ? t('achats.emptyArchived') : t('achats.empty')"
        />
        <div v-else class="table-scroll">
          <table class="table">
            <thead>
              <tr>
                <SortHeader :sort="sort" field="reference" :label="t('achats.columns.reference')" />
                <SortHeader :sort="sort" field="client" :label="t('achats.columns.client')" />
                <SortHeader :sort="sort" field="product" :label="t('achats.columns.product')" />
                <SortHeader :sort="sort" field="date" :label="t('achats.columns.date')" />
                <SortHeader :sort="sort" field="total" :label="t('achats.columns.total')" />
                <SortHeader :sort="sort" field="remaining" :label="t('achats.columns.remaining')" />
                <SortHeader :sort="sort" field="status" :label="t('achats.columns.status')" />
                <th class="col-action">{{ t("common.actions") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="p in sorted"
                :key="p.id"
                class="clickable"
                @click="router.push({ name: 'achat-detail', params: { id: p.id } })"
              >
                <td>
                  <div class="ref-cell">
                    <span class="row-link">{{ p.reference }}</span>
                    <span
                      v-if="p.archivedAt"
                      class="badge badge--pending archived-pill"
                      :title="t('achats.archivedOn', { date: fmt.date(p.archivedAt) })"
                    >
                      {{ t("achats.archivedBadge") }}
                    </span>
                  </div>
                </td>
                <td>{{ p.clientName }}</td>
                <td class="ellipsis">{{ p.productLabel }}</td>
                <td class="tabular">{{ fmt.date(p.purchaseDate) }}</td>
                <td class="tabular">{{ fmt.money(p.totalPrice) }}</td>
                <td class="tabular strong">{{ fmt.money(p.remaining) }}</td>
                <td><StatusBadge :status="p.status" /></td>
                <td class="col-action" @click.stop>
                  <template v-if="p.archivedAt">
                    <button
                      class="icon-action"
                      type="button"
                      :title="t('achats.restore.action')"
                      @click="pending = { kind: 'restore', purchase: p }"
                    >
                      <AppIcon name="rotate-ccw" :size="17" />
                    </button>
                    <!-- Permanent delete lives only here, behind the archive. -->
                    <button
                      class="icon-action icon-action--danger"
                      type="button"
                      :title="t('achats.delete.action')"
                      @click="pending = { kind: 'delete', purchase: p }"
                    >
                      <AppIcon name="trash" :size="17" />
                    </button>
                  </template>
                  <template v-else>
                    <button
                      class="icon-action"
                      type="button"
                      :title="t('common.edit')"
                      @click="openEdit(p)"
                    >
                      <AppIcon name="edit" :size="17" />
                    </button>
                    <button
                      class="icon-action"
                      type="button"
                      :title="t('achats.archive.action')"
                      @click="pending = { kind: 'archive', purchase: p }"
                    >
                      <AppIcon name="archive" :size="17" />
                    </button>
                  </template>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <NewPurchaseModal v-if="showModal" :purchase="editing" @close="closeModal" @saved="onSaved" />
      <ConfirmDialog
        v-if="dialog"
        :title="dialog.title"
        :message="dialog.message"
        :confirm-label="dialog.confirmLabel"
        :danger="dialog.danger"
        :warning="dialog.warning"
        :confirm-disabled="dialog.confirmDisabled"
        @close="pending = null"
        @confirm="confirmPending"
      />
    </template>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 18px;
}
.header-actions {
  display: flex;
  align-items: center;
  gap: 14px;
  flex-wrap: wrap;
}
/* Third copy of the shared tabs block — see EcheancesView/AlertesView. */
.tabs {
  display: flex;
  gap: 4px;
  padding: 4px;
  background: var(--bg);
  border-radius: 9px;
}
.tab {
  padding: 7px 14px;
  border: none;
  background: transparent;
  border-radius: 7px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-secondary);
}
.tab--active {
  background: var(--surface);
  color: var(--primary);
  box-shadow: var(--shadow-card);
}
.ref-cell {
  display: flex;
  align-items: center;
  gap: 10px;
}
.archived-pill {
  font-size: 11.5px;
  padding: 2px 8px;
}
.col-action {
  text-align: end;
  white-space: nowrap;
}
.icon-action {
  border: none;
  background: transparent;
  color: var(--text-muted);
  padding: 6px;
  border-radius: 8px;
  transition:
    background 0.13s,
    color 0.13s;
}
.icon-action:hover {
  background: var(--bg);
  color: var(--text);
}
.icon-action--danger:hover {
  background: var(--danger-bg);
  color: var(--danger-strong);
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
