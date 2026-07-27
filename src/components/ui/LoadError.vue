<script setup lang="ts">
// Inline failure state for a view whose data could not be loaded.
//
// The message is already localized by `toUserMessage` (see `useLoader`); this
// component only presents it and offers the retry the user otherwise has no way
// to trigger short of navigating away and back.
import AppIcon from "@/components/ui/AppIcon.vue";

defineProps<{ message: string }>();
defineEmits<{ retry: [] }>();
</script>

<template>
  <div class="load-error card" role="alert">
    <div class="load-error-icon"><AppIcon name="alert" :size="24" /></div>
    <p class="load-error-message">{{ message }}</p>
    <button type="button" class="btn btn-secondary" @click="$emit('retry')">
      {{ $t("common.retry") }}
    </button>
  </div>
</template>

<style scoped>
.load-error {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 40px 24px;
  gap: 12px;
}
.load-error-icon {
  width: 56px;
  height: 56px;
  border-radius: 16px;
  background: var(--bg);
  color: var(--danger);
  display: flex;
  align-items: center;
  justify-content: center;
}
.load-error-message {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
  max-width: 46ch;
}
</style>
