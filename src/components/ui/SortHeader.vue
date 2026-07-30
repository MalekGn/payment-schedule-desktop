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
  /**
   * Present the column as unsortable. Sorting is a licensed feature, and it runs
   * entirely in the browser (`useSort.ts` reorders rows already fetched), so
   * this is the only place it can be withheld — there is no server call to
   * refuse. It communicates the boundary; it does not enforce it.
   */
  disabled?: boolean;
}>();

const isActive = () => props.sort.key === props.field;

function toggle() {
  if (props.disabled) return;
  props.sort.toggle(props.field);
}
</script>

<template>
  <th
    class="sortable"
    :class="{
      'is-active': isActive(),
      'align-end': align === 'end',
      'is-disabled': disabled,
    }"
    :aria-sort="isActive() ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none'"
    :aria-disabled="disabled ? 'true' : undefined"
    role="button"
    :tabindex="disabled ? -1 : 0"
    @click="toggle"
    @keydown.enter.prevent="toggle"
    @keydown.space.prevent="toggle"
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
.sortable.is-disabled {
  cursor: default;
}
.sortable.is-disabled:hover {
  color: inherit;
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
