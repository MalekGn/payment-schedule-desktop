<script setup lang="ts">
// Self-contained calendar date picker (no external deps). The trigger shows the
// selected date in the *configured* settings format (via useFormat); the popup
// is a styled month grid with localized month/weekday names, a Today button and
// a Clear button. Closes on outside click, Esc, or day selection.
// Model value is an ISO YYYY-MM-DD string ("" = unset).
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import AppIcon from "@/components/ui/AppIcon.vue";
import { useFormat, LOCALE_TAG } from "@/composables/useFormat";
import { useSettingsStore } from "@/stores/settings";

const model = defineModel<string>({ default: "" });
const props = defineProps<{ min?: string; max?: string; placeholder?: string }>();

const { t } = useI18n();
const fmt = useFormat();
const settings = useSettingsStore();

const open = ref(false);
const root = ref<HTMLElement | null>(null);
const popup = ref<HTMLElement | null>(null);

// The popup is teleported to <body> so it is never clipped by a scroll
// container (e.g. the installment rows) or stacked under a modal overlay.
const POP_WIDTH = 264;
const POP_HEIGHT = 350;
const popStyle = ref<Record<string, string>>({});

function reposition() {
  const r = root.value?.getBoundingClientRect();
  if (!r) return;
  const gap = 6;
  const left = Math.max(8, Math.min(r.left, window.innerWidth - POP_WIDTH - 8));
  const spaceBelow = window.innerHeight - r.bottom;
  const openUp = spaceBelow < POP_HEIGHT + gap && r.top > spaceBelow;
  const top = openUp ? Math.max(8, r.top - POP_HEIGHT - gap) : r.bottom + gap;
  popStyle.value = { top: `${top}px`, left: `${left}px` };
}

function onDocClick(e: MouseEvent) {
  if (!open.value) return;
  const target = e.target as Node;
  if (root.value?.contains(target) || popup.value?.contains(target)) return;
  open.value = false;
}
// Capture-phase Esc so it dismisses the picker without also closing a host modal.
function onKeydown(e: KeyboardEvent) {
  if (open.value && e.key === "Escape") {
    open.value = false;
    e.stopImmediatePropagation();
    e.preventDefault();
  }
}

watch(open, async (isOpen) => {
  if (isOpen) {
    view.value = parse(model.value);
    await nextTick();
    reposition();
    window.addEventListener("scroll", reposition, true);
    window.addEventListener("resize", reposition);
  } else {
    window.removeEventListener("scroll", reposition, true);
    window.removeEventListener("resize", reposition);
  }
});

onMounted(() => {
  document.addEventListener("click", onDocClick);
  document.addEventListener("keydown", onKeydown, true);
});
onBeforeUnmount(() => {
  document.removeEventListener("click", onDocClick);
  document.removeEventListener("keydown", onKeydown, true);
  window.removeEventListener("scroll", reposition, true);
  window.removeEventListener("resize", reposition);
});

const locale = computed(() => LOCALE_TAG[settings.language] ?? "fr-FR");

function pad(n: number): string {
  return String(n).padStart(2, "0");
}
function iso(y: number, m: number, d: number): string {
  return `${y}-${pad(m)}-${pad(d)}`;
}
function todayIso(): string {
  const n = new Date();
  return iso(n.getFullYear(), n.getMonth() + 1, n.getDate());
}
function parse(value: string): { y: number; m: number } {
  const [y, m] = value.split("-").map(Number);
  if (y && m) return { y, m };
  const [ty, tm] = todayIso().split("-").map(Number);
  return { y: ty, m: tm };
}
const today = todayIso();

// The month currently shown in the grid (m is 1-based). Reset when the popup
// opens (see the watch above).
const view = ref(parse(model.value));

const monthLabel = computed(() =>
  new Intl.DateTimeFormat(locale.value, { month: "long", year: "numeric" }).format(
    new Date(view.value.y, view.value.m - 1, 1),
  ),
);

const weekdays = computed(() => {
  const f = new Intl.DateTimeFormat(locale.value, { weekday: "short" });
  // 2023-01-01 is a Sunday; build a Sunday-first header row.
  return Array.from({ length: 7 }, (_, i) => f.format(new Date(2023, 0, 1 + i)));
});

interface Cell {
  iso: string;
  day: number;
  inMonth: boolean;
  disabled: boolean;
}
function makeCell(y: number, m: number, day: number, inMonth: boolean): Cell {
  const value = iso(y, m, day);
  const disabled = Boolean((props.min && value < props.min) || (props.max && value > props.max));
  return { iso: value, day, inMonth, disabled };
}

const cells = computed<Cell[]>(() => {
  const { y, m } = view.value;
  const startDow = new Date(y, m - 1, 1).getDay(); // 0 = Sunday
  const daysInMonth = new Date(y, m, 0).getDate();
  const prevMonthDays = new Date(y, m - 1, 0).getDate();
  const prevY = m === 1 ? y - 1 : y;
  const prevM = m === 1 ? 12 : m - 1;
  const nextY = m === 12 ? y + 1 : y;
  const nextM = m === 12 ? 1 : m + 1;

  const out: Cell[] = [];
  for (let i = startDow - 1; i >= 0; i--)
    out.push(makeCell(prevY, prevM, prevMonthDays - i, false));
  for (let d = 1; d <= daysInMonth; d++) out.push(makeCell(y, m, d, true));
  for (let d = 1; out.length < 42; d++) out.push(makeCell(nextY, nextM, d, false));
  return out;
});

const display = computed(() => (model.value ? fmt.date(model.value) : (props.placeholder ?? "")));

function shiftMonth(delta: number) {
  const total = view.value.y * 12 + (view.value.m - 1) + delta;
  view.value = { y: Math.floor(total / 12), m: (total % 12) + 1 };
}
function pick(cell: Cell) {
  if (cell.disabled) return;
  model.value = cell.iso;
  open.value = false;
}
function pickToday() {
  const [y, m] = today.split("-").map(Number);
  if ((props.min && today < props.min) || (props.max && today > props.max)) {
    view.value = { y, m }; // out of range: just jump the view to today's month
    return;
  }
  model.value = today;
  open.value = false;
}
function clear() {
  model.value = "";
  open.value = false;
}
</script>

<template>
  <div ref="root" class="datepicker">
    <button
      type="button"
      class="dp-trigger"
      :class="{ 'is-empty': !model, 'is-open': open }"
      :aria-expanded="open"
      @click="open = !open"
    >
      <AppIcon name="calendar" :size="15" class="dp-cal" />
      <span class="dp-value">{{ display }}</span>
      <AppIcon
        v-if="model"
        name="x"
        :size="14"
        class="dp-clear"
        role="button"
        :aria-label="t('filters.clear')"
        @click.stop="clear"
      />
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        ref="popup"
        class="dp-pop"
        role="dialog"
        data-datepicker-pop
        :style="popStyle"
      >
        <div class="dp-head">
          <button type="button" class="dp-nav" aria-label="previous month" @click="shiftMonth(-1)">
            <AppIcon name="chevron-left" :size="18" />
          </button>
          <span class="dp-month">{{ monthLabel }}</span>
          <button type="button" class="dp-nav" aria-label="next month" @click="shiftMonth(1)">
            <AppIcon name="chevron-right" :size="18" />
          </button>
        </div>

        <div class="dp-grid dp-weekdays">
          <span v-for="(w, i) in weekdays" :key="i" class="dp-weekday">{{ w }}</span>
        </div>

        <div class="dp-grid">
          <button
            v-for="c in cells"
            :key="c.iso"
            type="button"
            class="dp-day"
            :class="{
              'is-outside': !c.inMonth,
              'is-today': c.iso === today,
              'is-selected': c.iso === model,
              'is-disabled': c.disabled,
            }"
            :disabled="c.disabled"
            @click="pick(c)"
          >
            {{ c.day }}
          </button>
        </div>

        <div class="dp-foot">
          <button type="button" class="btn btn--ghost btn--sm" @click="clear">
            {{ t("filters.clear") }}
          </button>
          <button type="button" class="btn btn--primary btn--sm" @click="pickToday">
            {{ t("filters.today") }}
          </button>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.datepicker {
  position: relative;
}
.dp-trigger {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-width: 150px;
  padding: 8px 11px;
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-radius: 10px;
  color: var(--text);
  font-size: 13.5px;
  font-weight: 500;
  text-align: start;
  transition: border-color 0.13s ease;
}
.dp-trigger:hover {
  border-color: var(--primary);
}
.dp-trigger.is-open {
  border-color: var(--primary);
  box-shadow: 0 0 0 3px var(--primary-soft);
}
.dp-cal {
  color: var(--text-muted);
  flex-shrink: 0;
}
.dp-value {
  flex: 1;
}
.dp-trigger.is-empty .dp-value {
  color: var(--text-muted);
}
.dp-clear {
  color: var(--text-muted);
  border-radius: 4px;
  flex-shrink: 0;
}
.dp-clear:hover {
  color: var(--danger-strong);
}
.dp-pop {
  position: fixed;
  width: 264px;
  padding: 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 14px;
  box-shadow: var(--shadow-pop);
  /* Above the modal overlay (z-index 900) so it works inside modals too. */
  z-index: 1000;
}
.dp-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}
.dp-month {
  font-size: 14px;
  font-weight: 700;
  color: var(--text);
  text-transform: capitalize;
}
.dp-nav {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-secondary);
}
.dp-nav:hover {
  background: var(--bg);
  color: var(--text);
}
.dp-grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 2px;
}
.dp-weekdays {
  margin-bottom: 4px;
}
.dp-weekday {
  text-align: center;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-muted);
  text-transform: uppercase;
  padding: 4px 0;
}
.dp-day {
  aspect-ratio: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  font-weight: 500;
  transition:
    background 0.1s ease,
    color 0.1s ease;
}
.dp-day:hover:not(.is-disabled) {
  background: var(--primary-soft);
  color: var(--primary);
}
.dp-day.is-outside {
  color: var(--text-muted);
  opacity: 0.55;
}
.dp-day.is-today {
  box-shadow: inset 0 0 0 1.5px var(--primary);
  color: var(--primary);
  font-weight: 700;
}
.dp-day.is-selected {
  background: var(--primary);
  color: #fff;
  font-weight: 700;
}
.dp-day.is-selected.is-today {
  box-shadow: none;
}
.dp-day.is-disabled {
  color: var(--text-muted);
  opacity: 0.35;
  cursor: not-allowed;
}
.dp-foot {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  margin-top: 10px;
  padding-top: 10px;
  border-top: 1px solid var(--border);
}
</style>
