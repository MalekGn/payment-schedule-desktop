<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import ClientForm from "@/components/ClientForm.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { useFormat } from "@/composables/useFormat";
import { useUiStore } from "@/stores/ui";
import { useStatsStore } from "@/stores/stats";
import { api } from "@/api";
import type { Client, ClientSummary } from "@/types/models";

const { t } = useI18n();
const router = useRouter();
const fmt = useFormat();
const ui = useUiStore();
const stats = useStatsStore();

const clients = ref<ClientSummary[]>([]);
const search = ref("");
const loading = ref(true);
const showForm = ref(false);
const editing = ref<Client | null>(null);
const deleteTarget = ref<ClientSummary | null>(null);
const deleteMessage = ref("");

const filtered = computed(() => {
  const n = search.value.trim().toLowerCase();
  if (!n) return clients.value;
  return clients.value.filter((c) =>
    `${c.firstName} ${c.lastName} ${c.phone} ${c.address}`.toLowerCase().includes(n),
  );
});

async function load() {
  loading.value = true;
  clients.value = await api.listClients();
  loading.value = false;
}
onMounted(load);

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

function askDelete(c: ClientSummary) {
  deleteTarget.value = c;
  deleteMessage.value =
    c.purchaseCount > 0
      ? t("clients.delete.hasPurchases", { n: c.purchaseCount })
      : t("clients.delete.confirmText");
}
async function confirmDelete() {
  if (!deleteTarget.value) return;
  try {
    await api.deleteClient(deleteTarget.value.id, true);
    ui.notify(t("common.delete"));
    deleteTarget.value = null;
    await load();
    await stats.refresh();
  } catch (e) {
    ui.notify(String(e), "error");
  }
}

function openDetail(id: number) {
  router.push({ name: "client-detail", params: { id } });
}
</script>

<template>
  <div class="page">
    <div class="toolbar">
      <div class="search-box">
        <AppIcon name="search" :size="18" class="muted" />
        <input v-model="search" class="search-input" :placeholder="t('clients.searchPlaceholder')" />
      </div>
      <button class="btn btn--primary" type="button" @click="openNew">
        <AppIcon name="plus" :size="18" /> {{ t("clients.new") }}
      </button>
    </div>

    <div class="card">
      <EmptyState
        v-if="!loading && clients.length === 0"
        icon="users"
        :title="t('clients.empty')"
      />
      <table v-else class="table">
        <thead>
          <tr>
            <th>{{ t("clients.columns.name") }}</th>
            <th>{{ t("clients.columns.phone") }}</th>
            <th>{{ t("clients.columns.address") }}</th>
            <th>{{ t("clients.columns.purchases") }}</th>
            <th>{{ t("clients.columns.outstanding") }}</th>
            <th class="col-action">{{ t("common.actions") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="c in filtered" :key="c.id" class="clickable" @click="openDetail(c.id)">
            <td>
              <div class="client-name">
                <span class="strong">{{ c.firstName }} {{ c.lastName }}</span>
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
              <button class="icon-action" type="button" :title="t('common.edit')" @click="openEdit(c)">
                <AppIcon name="edit" :size="17" />
              </button>
              <button class="icon-action icon-action--danger" type="button" :title="t('common.delete')" @click="askDelete(c)">
                <AppIcon name="trash" :size="17" />
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <ClientForm v-if="showForm" :client="editing" @close="showForm = false" @saved="onSaved" />
    <ConfirmDialog
      v-if="deleteTarget"
      :title="t('clients.delete.confirmTitle')"
      :message="deleteMessage"
      :confirm-label="t('common.delete')"
      danger
      @close="deleteTarget = null"
      @confirm="confirmDelete"
    />
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
  transition: background 0.13s, color 0.13s;
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
