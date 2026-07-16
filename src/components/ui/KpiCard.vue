<script setup lang="ts">
import AppIcon from "@/components/ui/AppIcon.vue";

defineProps<{
  icon: string;
  label: string;
  value: string;
  sub: string;
  tone: "blue" | "green" | "purple" | "orange" | "red";
}>();
</script>

<template>
  <div class="kpi card">
    <div class="kpi-icon" :class="`kpi-icon--${tone}`">
      <AppIcon :name="icon" :size="24" :stroke-width="2" />
    </div>
    <div class="kpi-body">
      <span class="kpi-label">{{ label }}</span>
      <span class="kpi-value tabular">{{ value }}</span>
      <span class="kpi-sub">{{ sub }}</span>
    </div>
  </div>
</template>

<style scoped>
.kpi {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 18px 20px;
}
.kpi-icon {
  width: 52px;
  height: 52px;
  border-radius: 13px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  flex-shrink: 0;
}
.kpi-icon--blue {
  background: #2563eb;
}
.kpi-icon--green {
  background: #16a34a;
}
.kpi-icon--purple {
  background: #7c3aed;
}
.kpi-icon--orange {
  background: #f59e0b;
}
.kpi-icon--red {
  background: #ef4444;
}
.kpi-body {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.kpi-label {
  font-size: 13px;
  color: var(--text-secondary);
  font-weight: 500;
  line-height: 1.3;
}
.kpi-value {
  font-size: 22px;
  font-weight: 700;
  color: var(--text);
  letter-spacing: -0.02em;
  margin-top: 2px;
  white-space: nowrap;
}
.kpi-sub {
  font-size: 12px;
  color: var(--text-muted);
  margin-top: 2px;
}
</style>
