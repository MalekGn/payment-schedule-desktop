<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";

// Renders a localized status pill for installment or purchase statuses.
const props = defineProps<{
  status: "pending" | "partial" | "paid" | "late" | "in_progress";
  // Feminine "Payée" (tranche) vs masculine "Payé" (achat) in French.
  feminine?: boolean;
}>();

const { t } = useI18n();

const CLASS: Record<string, string> = {
  pending: "badge--pending",
  partial: "badge--partial",
  paid: "badge--success",
  in_progress: "badge--progress",
  late: "badge--late",
};

const ICON: Record<string, string | null> = {
  paid: "check",
  late: "x",
  pending: null,
  partial: null,
  in_progress: null,
};

const label = computed(() => {
  if (props.status === "paid") return t(props.feminine ? "status.paid" : "status.paidM");
  return t(`status.${props.status}`);
});
</script>

<template>
  <span class="badge" :class="CLASS[status]">
    <AppIcon v-if="ICON[status]" :name="ICON[status] as string" :size="14" :stroke-width="2.5" />
    {{ label }}
  </span>
</template>
