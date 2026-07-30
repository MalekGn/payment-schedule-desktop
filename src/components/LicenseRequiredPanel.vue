<script setup lang="ts">
// Shown in place of a licensed route's content when the install has no valid
// licence. Rendered from `App.vue` off `route.meta.licensed`, so there is one
// gate site rather than a check inside each of the seven licensed views.
//
// This is presentation only. The commands behind these screens refuse on their
// own in Rust — see the licence gate in `src-tauri/src/commands.rs`.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import { useLicenseStore } from "@/stores/license";
import { useFormat } from "@/composables/useFormat";

const license = useLicenseStore();
const fmt = useFormat();
const { t } = useI18n();

/**
 * Say why the app is locked, not just that it is. "Your licence expired on 3
 * March" tells the user what to do next; "licence required" does not.
 */
const explanation = computed(() => {
  switch (license.status) {
    case "expired":
      return t("license.expiredBody", { date: fmt.date(license.info.expiredOn) });
    case "machineMismatch":
      return t("license.machineMismatchBody");
    case "clockTampered":
      return t("license.clockTamperedBody");
    default:
      return t("license.requiredBody");
  }
});
</script>

<template>
  <div class="card gate">
    <div class="gate-icon"><AppIcon name="lock" :size="30" /></div>
    <h2>{{ t("license.requiredTitle") }}</h2>
    <p>{{ explanation }}</p>
    <RouterLink class="btn btn--primary" to="/parametres">
      {{ t("license.goToSettings") }}
    </RouterLink>
  </div>
</template>

<style scoped>
.gate {
  max-width: 560px;
  margin: 40px auto;
  padding: 40px;
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 12px;
}
.gate-icon {
  width: 64px;
  height: 64px;
  border-radius: 18px;
  background: var(--primary-soft);
  color: var(--primary);
  display: flex;
  align-items: center;
  justify-content: center;
}
.gate h2 {
  font-size: 18px;
  font-weight: 700;
}
.gate p {
  color: var(--text-secondary);
  line-height: 1.5;
}
.gate .btn {
  margin-top: 8px;
}
</style>
