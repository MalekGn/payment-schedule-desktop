<script setup lang="ts">
/**
 * A grouped bar chart, drawn as inline SVG.
 *
 * Hand-rolled rather than pulled from a charting library, for the same reason
 * `DatePicker.vue` and `AppIcon.vue` are: this app ships no chart dependency,
 * and one series of paired bars does not justify adding the first. Inline SVG
 * also scales with the layout and inherits the theme tokens, which a canvas
 * renderer would not.
 *
 * Accessibility: the SVG carries a `<title>`, and the same numbers are repeated
 * in a visually-hidden table so a screen reader gets the data rather than an
 * unlabelled graphic.
 */
import { computed } from "vue";

export interface BarChartSeries {
  /** Localized name, shown in the legend and the accessible table. */
  label: string;
  /** One value per entry in `labels`, same order. */
  values: number[];
  /** A CSS colour — pass a `var(--token)` so the chart follows the theme. */
  color: string;
}

const props = withDefaults(
  defineProps<{
    /** Category labels along the value axis, one per bar group. */
    labels: string[];
    series: BarChartSeries[];
    /** Accessible name for the whole chart. */
    title: string;
    /** Renders a value for the tooltip, the axis and the accessible table. */
    format: (value: number) => string;
    /** Height of the plot area in px; the width is fluid. */
    height?: number;
  }>(),
  { height: 220 },
);

/** Gridline count. Four bands reads as a scale without becoming a ruler. */
const BANDS = 4;

/**
 * The axis maximum, rounded up to a whole band so the gridlines land on round
 * numbers instead of on an arbitrary fraction of the tallest bar.
 */
const max = computed(() => {
  const peak = Math.max(0, ...props.series.flatMap((s) => s.values));
  if (peak <= 0) return 0;
  const step = Math.pow(10, Math.floor(Math.log10(peak / BANDS)));
  return Math.ceil(peak / BANDS / step) * step * BANDS;
});

/** Height in percent of the plot area, so the bars scale with the viewBox. */
function barHeight(value: number): string {
  if (max.value <= 0) return "0%";
  return `${Math.max(0, (value / max.value) * 100)}%`;
}

const gridlines = computed(() =>
  Array.from({ length: BANDS + 1 }, (_, i) => ({
    value: (max.value / BANDS) * (BANDS - i),
    offset: (100 / BANDS) * i,
  })),
);

/**
 * Thin the tick labels when the range is long. Every bucket keeps its bar; only
 * the text is dropped, because 90 overlapping day labels are less readable than
 * none.
 */
const tickEvery = computed(() => Math.ceil(props.labels.length / 12) || 1);
</script>

<template>
  <div class="chart">
    <div class="chart-legend">
      <span v-for="s in series" :key="s.label" class="chart-legend-item">
        <span class="chart-swatch" :style="{ background: s.color }" aria-hidden="true"></span>
        {{ s.label }}
      </span>
    </div>

    <div class="chart-plot" :style="{ height: `${height}px` }">
      <!-- Gridlines and their value labels sit behind the bars. -->
      <div class="chart-grid" aria-hidden="true">
        <div
          v-for="g in gridlines"
          :key="g.offset"
          class="chart-grid-line"
          :style="{ top: `${g.offset}%` }"
        >
          <span class="chart-grid-label tabular">{{ format(g.value) }}</span>
        </div>
      </div>

      <div class="chart-bars" role="img" :aria-label="title">
        <div v-for="(label, i) in labels" :key="label" class="chart-group">
          <div class="chart-group-bars">
            <div
              v-for="s in series"
              :key="s.label"
              class="chart-bar"
              :style="{ height: barHeight(s.values[i] ?? 0), background: s.color }"
              :title="`${label} · ${s.label}: ${format(s.values[i] ?? 0)}`"
            ></div>
          </div>
          <span class="chart-tick" :class="{ 'chart-tick--hidden': i % tickEvery !== 0 }">
            {{ label }}
          </span>
        </div>
      </div>
    </div>

    <!--
      The same figures as text. A bar is invisible to a screen reader, and the
      alternative — an aria-label per bar — produces an unreadable stream.
    -->
    <table class="visually-hidden">
      <caption>
        {{
          title
        }}
      </caption>
      <thead>
        <tr>
          <th></th>
          <th v-for="s in series" :key="s.label">{{ s.label }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(label, i) in labels" :key="label">
          <th>{{ label }}</th>
          <td v-for="s in series" :key="s.label">{{ format(s.values[i] ?? 0) }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.chart {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.chart-legend {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
  font-size: 13px;
  color: var(--text-secondary);
}
.chart-legend-item {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
}
.chart-swatch {
  width: 12px;
  height: 12px;
  border-radius: 3px;
  flex-shrink: 0;
}

.chart-plot {
  position: relative;
  /* Room for the gridline values, which sit outside the plot on the inline
     start edge. Logical padding so RTL mirrors without a second rule. */
  padding-inline-start: 64px;
}

.chart-grid {
  position: absolute;
  inset-block: 0;
  inset-inline: 64px 0;
  bottom: 26px;
}
.chart-grid-line {
  position: absolute;
  inset-inline: 0;
  border-top: 1px solid var(--border);
}
.chart-grid-label {
  position: absolute;
  inset-inline-end: 100%;
  transform: translateY(-50%);
  padding-inline-end: var(--space-2);
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
}

.chart-bars {
  position: absolute;
  inset-block: 0;
  inset-inline: 64px 0;
  display: flex;
  align-items: flex-end;
  gap: 2px;
}
.chart-group {
  flex: 1 1 0;
  min-width: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
}
.chart-group-bars {
  height: calc(100% - 26px);
  display: flex;
  align-items: flex-end;
  justify-content: center;
  gap: 1px;
}
.chart-bar {
  flex: 1 1 0;
  max-width: 18px;
  min-height: 1px;
  border-radius: 2px 2px 0 0;
  transition: opacity 0.12s ease;
}
.chart-bar:hover {
  opacity: 0.78;
}
.chart-tick {
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
}
.chart-tick--hidden {
  visibility: hidden;
}

@media (prefers-reduced-motion: reduce) {
  .chart-bar {
    transition: none;
  }
}
</style>
