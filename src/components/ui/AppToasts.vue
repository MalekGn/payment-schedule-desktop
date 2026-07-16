<script setup lang="ts">
import AppIcon from "@/components/ui/AppIcon.vue";
import { useUiStore } from "@/stores/ui";

const ui = useUiStore();
const ICON: Record<string, string> = { success: "check", error: "alert", info: "bell" };
</script>

<template>
  <div class="toast-stack" role="status" aria-live="polite">
    <TransitionGroup name="toast">
      <div v-for="toast in ui.toasts" :key="toast.id" class="toast" :class="`toast--${toast.kind}`">
        <AppIcon :name="ICON[toast.kind]" :size="18" />
        <span>{{ toast.message }}</span>
        <button class="toast-close" type="button" @click="ui.dismiss(toast.id)">
          <AppIcon name="x" :size="15" />
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toast-stack {
  position: fixed;
  bottom: 24px;
  inset-inline-end: 24px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  z-index: 1000;
}
.toast {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 14px;
  border-radius: 10px;
  background: var(--surface);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
  font-size: 14px;
  font-weight: 500;
  min-width: 240px;
  max-width: 380px;
}
.toast--success {
  border-inline-start: 3px solid var(--success);
  color: var(--text);
}
.toast--success :deep(.app-icon) {
  color: var(--success);
}
.toast--error {
  border-inline-start: 3px solid var(--danger);
}
.toast--error :deep(.app-icon) {
  color: var(--danger);
}
.toast--info {
  border-inline-start: 3px solid var(--primary);
}
.toast-close {
  margin-inline-start: auto;
  border: none;
  background: transparent;
  color: var(--text-muted);
  display: inline-flex;
  padding: 2px;
}
.toast-enter-active,
.toast-leave-active {
  transition: all 0.25s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
</style>
