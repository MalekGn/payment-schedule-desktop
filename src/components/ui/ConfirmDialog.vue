<script setup lang="ts">
// Confirmation modal, with an optional *blocked* variant.
//
// The blocked variant exists because some actions can be known to be impossible
// before the user commits to them — archiving a client who still owes money is
// the case that prompted it. Rather than let the user confirm and then be
// refused by a toast in the far corner, the dialog opens explaining why, with
// the confirm button disabled and an optional way out. All three props are
// optional and inert when unset, so the plain confirmations are unchanged.
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import BaseModal from "@/components/ui/BaseModal.vue";

defineProps<{
  title: string;
  message: string;
  confirmLabel?: string;
  danger?: boolean;
  /** Why the action is blocked. Replaces `message` with a danger callout. */
  warning?: string;
  /** Renders the confirm button disabled. Pair with `warning` so the reason is visible. */
  confirmDisabled?: boolean;
  /** Optional extra button offering a way to resolve the block. */
  secondaryLabel?: string;
}>();
const emit = defineEmits<{ close: []; confirm: []; secondary: [] }>();
const { t } = useI18n();
</script>

<template>
  <BaseModal :title="title" @close="emit('close')">
    <div v-if="warning" class="confirm-warning" role="alert">
      <AppIcon name="alert" :size="20" />
      <p>{{ warning }}</p>
    </div>
    <p v-else class="confirm-msg">{{ message }}</p>

    <template #footer>
      <button class="btn btn--ghost" type="button" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
      <button v-if="secondaryLabel" class="btn btn--ghost" type="button" @click="emit('secondary')">
        {{ secondaryLabel }}
      </button>
      <button
        class="btn"
        :class="danger ? 'btn--danger' : 'btn--primary'"
        type="button"
        :disabled="confirmDisabled"
        @click="emit('confirm')"
      >
        {{ confirmLabel ?? t("common.confirm") }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.confirm-msg {
  color: var(--text-secondary);
  line-height: 1.5;
}
/* Same danger pairing as `.badge--late`, sized as a block callout. */
.confirm-warning {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 14px 16px;
  border-radius: 10px;
  background: var(--danger-bg);
  color: var(--danger-strong);
  line-height: 1.5;
  font-weight: 500;
}
.confirm-warning :deep(.app-icon) {
  margin-block-start: 1px;
}
</style>
