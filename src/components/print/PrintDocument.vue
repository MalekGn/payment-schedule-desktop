<script setup lang="ts">
/**
 * The sheet every printed document sits on: shop letterhead, title, and the
 * screen-only action bar.
 *
 * On screen this renders as a page preview — a centred white sheet on a grey
 * ground — so what the user sees is what the printer gets. The action bar and
 * the ground are `.no-print`; everything inside the sheet is the document.
 */
import { useI18n } from "vue-i18n";

import AppIcon from "@/components/ui/AppIcon.vue";
import { useBack } from "@/composables/useBack";
import { useFormat } from "@/composables/useFormat";
import { usePrint } from "@/composables/usePrint";
import { useShopIdentity } from "@/composables/useShopIdentity";
import { todayIso } from "@/lib/finance";

const props = defineProps<{
  /** Document title, e.g. "Échéancier". */
  title: string;
  /** The document's own reference — a purchase reference, a receipt number. */
  reference?: string;
  /**
   * Where Back goes when there is no in-app history to return to — which is the
   * normal case here, since a print route is reachable directly by URL.
   */
  backTo: string;
}>();

const { t } = useI18n();
const fmt = useFormat();
const { shopName, shopInfo, logoSrc } = useShopIdentity();
const { print } = usePrint();
const goBack = useBack(props.backTo);
</script>

<template>
  <div class="print-page">
    <div class="print-actions no-print">
      <button class="btn btn--ghost" type="button" @click="goBack">
        <AppIcon name="arrow-left" :size="16" class="icon-flip" />
        {{ t("common.back") }}
      </button>
      <button class="btn btn--primary" type="button" @click="print">
        <AppIcon name="download" :size="16" />
        {{ t("print.action") }}
      </button>
    </div>

    <article class="sheet">
      <header class="letterhead">
        <div class="letterhead-shop">
          <img v-if="logoSrc" :src="logoSrc" alt="" class="letterhead-logo" />
          <div class="letterhead-text">
            <!-- Falls back to the app title so the letterhead is never nameless,
                 which is what an install with no licence and no shop name would
                 otherwise produce. -->
            <p class="shop-name">{{ shopName || t("app.title") }}</p>
            <p v-if="shopInfo" class="shop-info">{{ shopInfo }}</p>
          </div>
        </div>
        <div class="letterhead-doc">
          <h1>{{ title }}</h1>
          <p v-if="reference" class="doc-ref">{{ reference }}</p>
          <p class="doc-date">{{ t("print.issuedOn", { date: fmt.date(todayIso()) }) }}</p>
        </div>
      </header>

      <div class="doc-body">
        <slot />
      </div>

      <footer class="doc-footer">
        <span>{{ shopName || t("app.title") }}</span>
        <span>{{ title }}</span>
      </footer>
    </article>
  </div>
</template>

<style scoped>
.print-page {
  min-height: 100vh;
  background: var(--bg);
  padding: var(--space-6);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--space-4);
}
.print-actions {
  width: 100%;
  max-width: 210mm;
  display: flex;
  justify-content: space-between;
  gap: var(--space-3);
}

/* A4 width, so the on-screen preview and the printed page agree. */
.sheet {
  width: 100%;
  max-width: 210mm;
  background: #fff;
  color: #111827;
  padding: 16mm 14mm;
  box-shadow: var(--shadow-card);
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.letterhead {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: var(--space-6);
  padding-bottom: var(--space-4);
  border-bottom: 2px solid #111827;
}
.letterhead-shop {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
}
.letterhead-logo {
  width: 54px;
  height: 54px;
  object-fit: contain;
  flex-shrink: 0;
}
.shop-name {
  font-size: 17px;
  font-weight: 700;
  line-height: 1.2;
}
.shop-info {
  font-size: 12px;
  color: #4b5563;
  white-space: pre-line;
  line-height: 1.45;
  margin-top: 2px;
}
/* Logical end, so the block sits on the correct side in Arabic. */
.letterhead-doc {
  text-align: end;
  flex-shrink: 0;
}
.letterhead-doc h1 {
  font-size: 20px;
  font-weight: 700;
  letter-spacing: -0.01em;
}
.doc-ref {
  font-size: 14px;
  font-weight: 600;
  margin-top: 2px;
}
.doc-date {
  font-size: 12px;
  color: #4b5563;
  margin-top: 2px;
}

.doc-body {
  display: flex;
  flex-direction: column;
  gap: var(--space-6);
}

.doc-footer {
  display: flex;
  justify-content: space-between;
  gap: var(--space-4);
  padding-top: var(--space-3);
  border-top: 1px solid #d1d5db;
  font-size: 11px;
  color: #6b7280;
}

@media print {
  .print-page {
    min-height: 0;
    background: none;
    padding: 0;
    display: block;
  }
  .sheet {
    max-width: none;
    box-shadow: none;
    padding: 0;
  }
}
</style>
