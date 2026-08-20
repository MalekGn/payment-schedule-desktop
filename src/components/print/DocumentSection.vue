<script setup lang="ts">
/**
 * One titled block inside a printed document, and the key/value grid the
 * client and purchase blocks are built from.
 *
 * Exists so `break-inside: avoid` is declared once. A client block split across
 * a page boundary is the most common way a printed document looks amateurish,
 * and it is invisible until someone actually prints a long schedule.
 */
defineProps<{ title?: string }>();
</script>

<template>
  <section class="doc-section">
    <h2 v-if="title" class="doc-section-title">{{ title }}</h2>
    <slot />
  </section>
</template>

<style scoped>
/* `.doc-section-title` is deliberately *not* here: the schedule and statement
   documents also use it on bare <h2>s outside this component, so it lives in
   the global print block in style.css. A scoped copy would drift from it. */
.doc-section {
  break-inside: avoid;
}
</style>
