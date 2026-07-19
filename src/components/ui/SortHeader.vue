<script setup lang="ts">
// A clickable table header cell that drives a shared `SortState`. Shows a caret
// on the active column and toggles direction on repeat clicks.
import AppIcon from "@/components/ui/AppIcon.vue";
import type { SortState } from "@/composables/useSort";

const props = defineProps<{
  sort: SortState;
  field: string;
  label: string;
  align?: "start" | "end";
}>();

const isActive = () => props.sort.key === props.field;
</script>

<template>
  <th
    class="sortable"
    :class="{ 'is-active': isActive(), 'align-end': align === 'end' }"
    :aria-sort="isActive() ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none'"
    role="button"
    tabindex="0"
    @click="sort.toggle(field)"
    @keydown.enter.prevent="sort.toggle(field)"
    @keydown.space.prevent="sort.toggle(field)"
  >
    <span class="sort-label">
      {{ label }}
      <AppIcon
        v-if="isActive()"
        class="sort-caret"
        :name="sort.dir === 'asc' ? 'chevron-up' : 'chevron-down'"
        :size="13"
      />
    </span>
  </th>
</template>

<style scoped>
.sortable {
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}
.sortable:hover {
  color: var(--text);
}
.sortable.is-active {
  color: var(--primary);
}
.sortable.align-end {
  text-align: end;
}
.sort-label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.align-end .sort-label {
  flex-direction: row-reverse;
}
.sort-caret {
  color: var(--primary);
}
</style>
