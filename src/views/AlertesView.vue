<script setup lang="ts">
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
const { t } = useI18n();
</script>

<template>
  <div class="card stub">
    <div class="stub-icon"><AppIcon name="bell" :size="30" /></div>
    <h2>{{ t("alertes.comingSoon") }}</h2>
    <p>{{ t("alertes.description") }}</p>
    <RouterLink class="btn btn--primary" to="/">{{ t("nav.dashboard") }}</RouterLink>
  </div>
</template>

<style scoped>
.stub {
  max-width: 560px;
  margin: 40px auto;
  padding: 40px;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 12px;
}
.stub-icon {
  width: 64px;
  height: 64px;
  border-radius: 18px;
  background: var(--warning-bg);
  color: var(--warning);
  display: flex;
  align-items: center;
  justify-content: center;
}
.stub h2 {
  font-size: 18px;
  font-weight: 700;
}
.stub p {
  color: var(--text-secondary);
  line-height: 1.5;
}
.stub .btn {
  margin-top: 8px;
}
</style>
