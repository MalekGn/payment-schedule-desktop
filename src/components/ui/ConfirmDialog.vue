<script setup lang="ts">
import { useI18n } from "vue-i18n";
import BaseModal from "@/components/ui/BaseModal.vue";

defineProps<{ title: string; message: string; confirmLabel?: string; danger?: boolean }>();
const emit = defineEmits<{ close: []; confirm: [] }>();
const { t } = useI18n();
</script>

<template>
  <BaseModal :title="title" @close="emit('close')">
    <p class="confirm-msg">{{ message }}</p>
    <template #footer>
      <button class="btn btn--ghost" type="button" @click="emit('close')">{{ t("common.cancel") }}</button>
      <button
        class="btn"
        :class="danger ? 'btn--danger' : 'btn--primary'"
        type="button"
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
</style>
