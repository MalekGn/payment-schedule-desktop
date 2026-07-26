<script setup lang="ts">
import { onMounted, onBeforeUnmount } from "vue";
import AppIcon from "@/components/ui/AppIcon.vue";

const props = defineProps<{ title: string; wide?: boolean }>();
const emit = defineEmits<{ close: [] }>();

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") emit("close");
}
onMounted(() => {
  document.addEventListener("keydown", onKey);
  document.body.style.overflow = "hidden";
});
onBeforeUnmount(() => {
  document.removeEventListener("keydown", onKey);
  document.body.style.overflow = "";
});
// touch props to satisfy the linter when wide is unused in script
void props;
</script>

<template>
  <div class="overlay" @mousedown.self="emit('close')">
    <div class="modal" :class="{ 'modal--wide': wide }" role="dialog" aria-modal="true">
      <div class="modal-head">
        <h2>{{ title }}</h2>
        <button class="icon-btn" type="button" aria-label="close" @click="emit('close')">
          <AppIcon name="x" :size="18" />
        </button>
      </div>
      <div class="modal-body">
        <slot />
      </div>
      <div v-if="$slots.footer" class="modal-foot">
        <slot name="footer" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(15, 23, 42, 0.45);
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 60px 20px;
  z-index: 900;
  overflow-y: auto;
}
.modal {
  background: var(--surface);
  border-radius: 16px;
  width: 100%;
  max-width: 520px;
  box-shadow: var(--shadow-pop);
  animation: pop 0.16s ease;
}
.modal--wide {
  max-width: 760px;
}
@keyframes pop {
  from {
    opacity: 0;
    transform: translateY(-8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
.modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 22px;
  border-bottom: 1px solid var(--border);
}
.modal-head h2 {
  font-size: 17px;
  font-weight: 700;
}
.icon-btn {
  border: none;
  background: transparent;
  color: var(--text-muted);
  display: inline-flex;
  padding: 6px;
  border-radius: 8px;
}
.icon-btn:hover {
  background: var(--bg);
  color: var(--text);
}
.modal-body {
  padding: 22px;
}
.modal-foot {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 16px 22px;
  border-top: 1px solid var(--border);
}
</style>
