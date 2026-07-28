<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import LoadError from "@/components/ui/LoadError.vue";
import SortHeader from "@/components/ui/SortHeader.vue";
import ClientForm from "@/components/ClientForm.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { parseErrorCode, toUserMessage } from "@/lib/errors";
import { useFormat } from "@/composables/useFormat";
import { useLoader } from "@/composables/useLoader";
import { useSort } from "@/composables/useSort";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { api } from "@/api";
import type { Client, ClientScope, ClientSummary } from "@/types/models";

const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();
const ui = useUiStore();
const stats = useStatsStore();

const clients = ref<ClientSummary[]>([]);
const search = ref("");
const showForm = ref(false);
const editing = ref<Client | null>(null);

/** Which slice the list is showing. Same shape as the tabs on Échéances/Alertes. */
const SCOPES = [
  { key: "active", label: "clients.scope.active" },
  { key: "archived", label: "clients.scope.archived" },
  { key: "all", label: "clients.scope.all" },
] as const;
const scope = ref<ClientScope>("active");

/**
 * The one open confirmation, whatever it is about.
 *
 * Delete, archive and restore all render through the single `ConfirmDialog`
 * below rather than three siblings: three would each need their own `v-if` and
 * would give the E2E suite three different `.confirm-msg` nodes to disambiguate.
 */
type PendingKind = "delete" | "archive" | "restore";
const pending = ref<{ kind: PendingKind; client: ClientSummary } | null>(null);

/**
 * Balance the backend reported when it refused an archive, if it ever does.
 *
 * The row's own `totalOutstanding` normally catches this first, so the dialog
 * opens already blocked and the confirm button is disabled. This covers the
 * stale-list race — the client took on a purchase in another window since the
 * list loaded — where the prediction said 0 and the database disagrees. Cleared
 * whenever a new dialog opens.
 */
const serverOutstanding = ref<number | null>(null);
watch(pending, () => {
  serverOutstanding.value = null;
});

/**
 * What is blocking the pending archive, or `null` when it can go ahead.
 * The backend's figure wins over the row's when we have it.
 */
const archiveBlockedBy = computed<number | null>(() => {
  const p = pending.value;
  if (!p || p.kind !== "archive") return null;
  return serverOutstanding.value ?? (p.client.totalOutstanding || null);
});

const filtered = computed(() => {
  const n = search.value.trim().toLowerCase();
  if (!n) return clients.value;
  return clients.value.filter((c) =>
    `${c.firstName} ${c.lastName} ${c.phone} ${c.address}`.toLowerCase().includes(n),
  );
});

const { sort, sorted } = useSort(filtered, {
  name: (c) => `${c.lastName} ${c.firstName}`,
  phone: (c) => c.phone,
  address: (c) => c.address,
  purchases: (c) => c.purchaseCount,
  outstanding: (c) => c.totalOutstanding,
});

const {
  loading,
  error: loadError,
  run: load,
} = useLoader(async () => {
  clients.value = await api.listClients(scope.value);
});
onMounted(load);
// The scope is a server-side filter (the search below stays client-side), so
// switching tabs has to refetch.
watch(scope, load);

function openNew() {
  editing.value = null;
  showForm.value = true;
}
function openEdit(c: ClientSummary) {
  editing.value = c;
  showForm.value = true;
}
async function onSaved() {
  showForm.value = false;
  await load();
  await stats.refresh();
}

/** Title, body, confirm label and danger styling for the open dialog. */
const dialog = computed(() => {
  const p = pending.value;
  if (!p) return null;
  if (p.kind === "archive") {
    const blocked = archiveBlockedBy.value;
    return {
      title: t("clients.archive.confirmTitle"),
      message: t("clients.archive.confirmText"),
      confirmLabel: t("clients.archive.action"),
      danger: false,
      // Explain the refusal up front rather than after a doomed confirm. The
      // amount is formatted here because only the view knows the configured
      // currency and grouping.
      warning: blocked ? t("clients.archive.hasOutstanding", { amount: fmt.money(blocked) }) : "",
      confirmDisabled: Boolean(blocked),
      secondaryLabel: blocked ? t("clients.archive.viewInstallments") : "",
    };
  }
  if (p.kind === "restore") {
    return {
      title: t("clients.restore.confirmTitle"),
      message: t("clients.restore.confirmText"),
      confirmLabel: t("clients.restore.action"),
      danger: false,
      warning: "",
      confirmDisabled: false,
      secondaryLabel: "",
    };
  }
  return {
    title: t("clients.delete.confirmTitle"),
    message: t("clients.delete.confirmText"),
    confirmLabel: t("common.delete"),
    danger: true,
    warning: "",
    confirmDisabled: false,
    secondaryLabel: "",
  };
});

/**
 * Delete a client outright.
 *
 * Only reachable for a client with no purchases — the button is not rendered
 * otherwise. The `CLIENT_HAS_PURCHASES` branch below is therefore the stale-list
 * case: the client gained a purchase in another window since this list loaded.
 * The backend stays the authority, and unlike before there is no second
 * confirmation that could push the delete through anyway, so this surfaces as a
 * plain error telling the user to archive instead.
 */
async function confirmDelete(target: ClientSummary) {
  try {
    await api.deleteClient(target.id);
    ui.notify(t("common.delete"));
  } catch (e) {
    const parsed = parseErrorCode(e);
    if (parsed?.code === "CLIENT_HAS_PURCHASES") {
      ui.notify(t("clients.delete.hasPurchases", { n: Number(parsed.params[0] ?? 0) }), "error");
      return;
    }
    throw e;
  }
}

/**
 * Archive the client, or record why the backend says we cannot.
 *
 * Returns `false` when the archive was refused for an outstanding balance, so
 * the caller leaves the dialog open — it re-renders blocked, with the figure
 * the database reports *now* rather than the one this list loaded with.
 */
async function confirmArchive(target: ClientSummary): Promise<boolean> {
  try {
    await api.archiveClient(target.id);
    ui.notify(t("clients.archive.action"));
    return true;
  } catch (e) {
    const parsed = parseErrorCode(e);
    if (parsed?.code === "ARCHIVE_HAS_OUTSTANDING") {
      serverOutstanding.value = Number(parsed.params[0] ?? 0);
      return false;
    }
    throw e;
  }
}

/** Run the pending action, then refresh the list and the sidebar counters. */
async function confirmPending() {
  const p = pending.value;
  // The confirm button is disabled while an archive is blocked; this is the
  // guard behind it, for a keyboard or programmatic activation.
  if (!p || archiveBlockedBy.value) return;

  // Only a refused archive keeps the dialog open — it turns into the blocked
  // state and explains itself. Every other outcome here is terminal, so leaving
  // the dialog up would just invite a retry that fails identically.
  let keepOpen = false;
  try {
    if (p.kind === "delete") await confirmDelete(p.client);
    else if (p.kind === "archive") keepOpen = !(await confirmArchive(p.client));
    else {
      await api.restoreClient(p.client.id);
      ui.notify(t("clients.restore.action"));
    }
  } catch (e) {
    ui.notify(toUserMessage(e, t), "error");
  }
  if (!keepOpen) pending.value = null;
  await load();
  await stats.refresh();
}

/** Blocked-dialog escape hatch: go where the unpaid installments are listed. */
function onDialogSecondary() {
  const p = pending.value;
  if (!p) return;
  pending.value = null;
  openDetail(p.client.id);
}

function openDetail(id: number) {
  router.push({ name: "client-detail", params: { id } });
}
</script>

<template>
  <div class="page">
    <LoadError v-if="loadError" :message="loadError" @retry="load" />

    <template v-else>
      <div class="toolbar">
        <div class="search-box">
          <AppIcon name="search" :size="18" class="muted" />
          <input
            v-model="search"
            class="search-input"
            :placeholder="t('clients.searchPlaceholder')"
          />
        </div>
        <!-- Same segmented-tabs shape as EcheancesView/AlertesView. Kept
             direction-agnostic (flex + gap, symmetric padding, no physical
             margins) so it mirrors correctly under dir="rtl". -->
        <div class="tabs">
          <button
            v-for="s in SCOPES"
            :key="s.key"
            class="tab"
            :class="{ 'tab--active': scope === s.key }"
            type="button"
            @click="scope = s.key"
          >
            {{ t(s.label) }}
          </button>
        </div>
        <button class="btn btn--primary" type="button" @click="openNew">
          <AppIcon name="plus" :size="18" /> {{ t("clients.new") }}
        </button>
      </div>

      <div class="card">
        <EmptyState
          v-if="!loading && clients.length === 0"
          icon="users"
          :title="scope === 'archived' ? t('clients.emptyArchived') : t('clients.empty')"
        />
        <div v-else class="table-scroll">
          <table class="table">
            <thead>
              <tr>
                <SortHeader :sort="sort" field="name" :label="t('clients.columns.name')" />
                <SortHeader :sort="sort" field="phone" :label="t('clients.columns.phone')" />
                <SortHeader :sort="sort" field="address" :label="t('clients.columns.address')" />
                <SortHeader
                  :sort="sort"
                  field="purchases"
                  :label="t('clients.columns.purchases')"
                />
                <SortHeader
                  :sort="sort"
                  field="outstanding"
                  :label="t('clients.columns.outstanding')"
                />
                <th class="col-action">{{ t("common.actions") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="c in sorted" :key="c.id" class="clickable" @click="openDetail(c.id)">
                <td>
                  <div class="client-name">
                    <span class="strong">{{ c.firstName }} {{ c.lastName }}</span>
                    <span
                      v-if="c.archivedAt"
                      class="badge badge--pending overdue-pill"
                      :title="t('clients.archivedOn', { date: fmt.date(c.archivedAt) })"
                    >
                      {{ t("clients.archivedBadge") }}
                    </span>
                    <span v-if="c.overdueCount > 0" class="badge badge--late overdue-pill">
                      {{ t("impaye.trancheLate", c.overdueCount) }}
                    </span>
                  </div>
                </td>
                <td class="tabular">{{ c.phone || "—" }}</td>
                <td class="ellipsis">{{ c.address || "—" }}</td>
                <td class="tabular">{{ c.purchaseCount }}</td>
                <td class="tabular strong">{{ fmt.money(c.totalOutstanding) }}</td>
                <td class="col-action" @click.stop>
                  <template v-if="c.archivedAt">
                    <button
                      class="icon-action"
                      type="button"
                      :title="t('clients.restore.action')"
                      @click="pending = { kind: 'restore', client: c }"
                    >
                      <AppIcon name="rotate-ccw" :size="17" />
                    </button>
                  </template>
                  <template v-else>
                    <button
                      class="icon-action"
                      type="button"
                      :title="t('common.edit')"
                      @click="openEdit(c)"
                    >
                      <AppIcon name="edit" :size="17" />
                    </button>
                    <button
                      class="icon-action"
                      type="button"
                      :title="t('clients.archive.action')"
                      @click="pending = { kind: 'archive', client: c }"
                    >
                      <AppIcon name="archive" :size="17" />
                    </button>
                  </template>
                  <!-- The only hard delete left. Hidden once the client has any
                       history, so the policy is visible rather than something the
                       user discovers by being refused. -->
                  <button
                    v-if="c.purchaseCount === 0"
                    class="icon-action icon-action--danger"
                    type="button"
                    :title="t('common.delete')"
                    @click="pending = { kind: 'delete', client: c }"
                  >
                    <AppIcon name="trash" :size="17" />
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <ClientForm v-if="showForm" :client="editing" @close="showForm = false" @saved="onSaved" />
      <ConfirmDialog
        v-if="dialog"
        :title="dialog.title"
        :message="dialog.message"
        :confirm-label="dialog.confirmLabel"
        :danger="dialog.danger"
        :warning="dialog.warning"
        :confirm-disabled="dialog.confirmDisabled"
        :secondary-label="dialog.secondaryLabel"
        @close="pending = null"
        @confirm="confirmPending"
        @secondary="onDialogSecondary"
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
.toolbar {
  display: flex;
  align-items: center;
  gap: 14px;
  /* Three children now (search, scope tabs, New client) against a 1024px
     minimum window. */
  flex-wrap: wrap;
}
/* Segmented scope filter. Same tokens and metrics as the tabs on
   EcheancesView/AlertesView — deliberately duplicated rather than extracted, so
   this change doesn't refactor two working views that both have tab-driven E2E
   scenarios. Extract all three together if a fourth appears. */
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
.search-box {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  max-width: 420px;
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
.client-name {
  display: flex;
  align-items: center;
  gap: 10px;
}
.overdue-pill {
  font-size: 11.5px;
  padding: 2px 8px;
}
.ellipsis {
  max-width: 240px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
</style>
