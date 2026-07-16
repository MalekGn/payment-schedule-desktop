<script setup lang="ts">
import AppIcon from "@/components/ui/AppIcon.vue";

defineProps<{ icon?: string; title: string; hint?: string }>();
</script>

<template>
  <div class="empty">
    <div v-if="icon" class="empty-icon"><AppIcon :name="icon" :size="26" /></div>
    <p class="empty-title">{{ title }}</p>
    <p v-if="hint" class="empty-hint">{{ hint }}</p>
    <slot />
  </div>
</template>

<style scoped>
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 40px 24px;
  gap: 8px;
}
.empty-icon {
  width: 56px;
  height: 56px;
  border-radius: 16px;
  background: var(--bg);
  color: var(--text-muted);
  display: flex;
  align-items: center;
  justify-content: center;
  margin-bottom: 4px;
}
.empty-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
}
.empty-hint {
  font-size: 13px;
  color: var(--text-muted);
}
</style>
