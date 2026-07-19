<script setup lang="ts">
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import { useBack } from "@/composables/useBack";

const { t } = useI18n();
const goBack = useBack("/");
</script>

<template>
  <div class="card stub">
    <div class="stub-icon"><AppIcon name="alert" :size="30" /></div>
    <h2>{{ t("notFound.title") }}</h2>
    <p>{{ t("notFound.description") }}</p>
    <div class="stub-actions">
      <button class="btn btn--ghost" type="button" @click="goBack">
        <AppIcon name="arrow-left" :size="16" /> {{ t("common.back") }}
      </button>
      <RouterLink class="btn btn--primary" to="/">{{ t("notFound.home") }}</RouterLink>
    </div>
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
.stub-actions {
  display: flex;
  gap: 10px;
  margin-top: 8px;
}
</style>
