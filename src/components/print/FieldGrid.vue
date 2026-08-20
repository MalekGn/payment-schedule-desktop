<script setup lang="ts">
/** A label/value grid for the identity blocks on a printed document. */
defineProps<{ fields: { label: string; value: string }[] }>();
</script>

<template>
  <dl class="fields">
    <div v-for="f in fields" :key="f.label" class="field">
      <dt>{{ f.label }}</dt>
      <dd>{{ f.value }}</dd>
    </div>
  </dl>
</template>

<style scoped>
.fields {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
  gap: var(--space-2) var(--space-6);
}
.field {
  display: flex;
  gap: var(--space-2);
  font-size: 13px;
  line-height: 1.5;
}
dt {
  color: #4b5563;
  flex-shrink: 0;
}
dt::after {
  content: " :";
}
dd {
  font-weight: 600;
  min-width: 0;
}
</style>
