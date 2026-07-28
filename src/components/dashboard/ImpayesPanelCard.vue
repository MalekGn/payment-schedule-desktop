<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppIcon from "@/components/ui/AppIcon.vue";
import EmptyState from "@/components/ui/EmptyState.vue";
import { useFormat } from "@/composables/useFormat";
import { useContactActions } from "@/composables/useContactActions";
import { todayIso } from "@/lib/finance";
import type { ImpayeClient } from "@/types/models";

const props = defineProps<{ impayes: ImpayeClient[] }>();
const { t } = useI18n();
const fmt = useFormat();
const router = useRouter();

// Display range: earliest overdue due date → today.
const range = computed(() => {
  const dates = props.impayes.flatMap((c) => c.installments.map((i) => i.dueDate)).sort();
  const from = dates[0] ?? todayIso();
  return `${fmt.date(from)} – ${fmt.date(todayIso())}`;
});

function goToImpayes(clientId?: number) {
  router.push({ name: "impayes", query: clientId ? { client: String(clientId) } : {} });
}
function openClient(id: number) {
  router.push({ name: "client-detail", params: { id } });
}

const contact = useContactActions();
</script>

<template>
  <section class="card">
    <div class="card-header">
      <h2>{{ t("dashboard.impayes") }}</h2>
      <RouterLink class="card-link" to="/impayes">{{ t("common.viewAll") }}</RouterLink>
    </div>

    <div class="filter-row">
      <button class="range-control" type="button" @click="goToImpayes()">
        <span class="tabular">{{ range }}</span>
        <AppIcon name="chevron-down" :size="15" class="muted" />
      </button>
      <button class="client-control" type="button" @click="goToImpayes()">
        <span>{{ t("impaye.allClients") }}</span>
        <AppIcon name="chevron-down" :size="15" class="muted" />
      </button>
      <button class="btn btn--primary btn--sm" type="button" @click="goToImpayes()">
        {{ t("common.filter") }}
      </button>
    </div>

    <EmptyState
      v-if="impayes.length === 0"
      :title="t('impayes.empty')"
      :hint="t('impayes.emptyHint')"
    />

    <ul v-else class="impaye-list">
      <li v-for="c in impayes" :key="c.clientId" class="impaye-row">
        <div class="impaye-ident">
          <span class="impaye-name">{{ c.clientName }}</span>
          <a class="row-link" href="#" @click.prevent="openClient(c.clientId)">{{ c.reference }}</a>
        </div>
        <div class="impaye-amount">
          <span class="impaye-total tabular">{{ fmt.money(c.totalOverdue) }}</span>
          <span class="impaye-count">{{ t("impaye.trancheLate", c.overdueCount) }}</span>
        </div>
        <div class="impaye-actions">
          <button
            class="contact-btn contact-btn--call"
            type="button"
            :title="t('impaye.call')"
            @click="contact.call(c.phone)"
          >
            <AppIcon name="phone" :size="17" />
          </button>
          <button
            class="contact-btn contact-btn--msg"
            type="button"
            :title="t('impaye.message')"
            @click="contact.message(c.phone)"
          >
            <AppIcon name="message" :size="17" />
          </button>
        </div>
      </li>
    </ul>

    <div class="impaye-foot">
      <button class="btn btn--ghost btn--sm" type="button" @click="goToImpayes()">
        <AppIcon name="download" :size="16" />
        {{ t("common.export") }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.filter-row {
  display: flex;
  gap: 10px;
  padding: 0 22px 14px;
  align-items: center;
}
.range-control,
.client-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px;
  border: 1px solid var(--border-strong);
  border-radius: 9px;
  background: var(--surface);
  font-size: 13px;
  color: var(--text-secondary);
  font-weight: 500;
}
.range-control {
  flex: 1.3;
}
.client-control {
  flex: 1;
}
.range-control:hover,
.client-control:hover {
  border-color: var(--primary);
}
.impaye-list {
  list-style: none;
  margin: 0;
  padding: 0 10px;
}
.impaye-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 13px 12px;
  border-bottom: 1px solid var(--border);
}
.impaye-row:last-child {
  border-bottom: none;
}
.impaye-ident {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  min-width: 0;
}
.impaye-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}
.impaye-amount {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  text-align: end;
}
.impaye-total {
  font-size: 14px;
  font-weight: 700;
  color: var(--danger-strong);
  white-space: nowrap;
}
.impaye-count {
  font-size: 12px;
  color: var(--danger);
  white-space: nowrap;
}
.impaye-actions {
  display: flex;
  gap: 8px;
}
.contact-btn {
  width: 38px;
  height: 34px;
  border-radius: 9px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
}
.contact-btn--call {
  background: #e7f8ef;
  color: var(--success);
  border-color: #c7efd9;
}
.contact-btn--call:hover {
  background: #d6f2e3;
}
.contact-btn--msg {
  background: var(--primary-soft);
  color: var(--primary);
  border-color: #d3e2fd;
}
.contact-btn--msg:hover {
  background: #dce9fe;
}
.impaye-foot {
  display: flex;
  justify-content: center;
  padding: 14px 22px 18px;
}
</style>
