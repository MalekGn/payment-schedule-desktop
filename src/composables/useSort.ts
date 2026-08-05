// Reusable client-side sorting for list/table views. A view declares a set of
// named accessors (column key -> value extractor); clicking a `SortHeader`
// toggles the active column and direction. The initial state is "unsorted"
// (`key: null`), so a list keeps the backend's own ordering until the user
// picks a column — this keeps default screens (and their tests) unchanged.

import { computed, reactive, toValue, type MaybeRefOrGetter } from "vue";

export type SortDir = "asc" | "desc";
export type SortValue = string | number | null | undefined;
export type Accessor<T> = (row: T) => SortValue;
export type Accessors<T> = Record<string, Accessor<T>>;

export interface SortState {
  key: string | null;
  dir: SortDir;
  /** Activate a column, or flip direction when it is already active. */
  toggle: (key: string) => void;
}

/**
 * Locale-aware comparison of two present values; numbers compare numerically.
 *
 * Blanks are deliberately *not* handled here — see `sortRows`, which keeps them
 * last in both directions and so must decide them outside the direction factor.
 */
function compare(a: NonNullable<SortValue>, b: NonNullable<SortValue>): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return String(a).localeCompare(String(b), undefined, { numeric: true, sensitivity: "base" });
}

export function useSortState(initial: { key?: string | null; dir?: SortDir } = {}): SortState {
  const state = reactive<SortState>({
    key: initial.key ?? null,
    dir: initial.dir ?? "asc",
    toggle(key: string) {
      if (state.key === key) {
        state.dir = state.dir === "asc" ? "desc" : "asc";
      } else {
        state.key = key;
        state.dir = "asc";
      }
    },
  });
  return state;
}

/** Pure, stable sort of `rows` according to the given state (no mutation). */
export function sortRows<T>(rows: T[], accessors: Accessors<T>, state: SortState): T[] {
  const key = state.key;
  if (!key) return rows;
  const accessor = accessors[key];
  if (!accessor) return rows;
  const factor = state.dir === "asc" ? 1 : -1;
  return rows.slice().sort((a, b) => {
    const va = accessor(a);
    const vb = accessor(b);
    // Blanks settle before the direction factor is applied, so they stay at the
    // bottom whichever way the column is sorted. Folding them into `compare`
    // and multiplying meant descending pulled every empty row to the top —
    // sorting the Notes column that way buried the payments that had one.
    if (va == null && vb == null) return 0;
    if (va == null) return 1;
    if (vb == null) return -1;
    return compare(va, vb) * factor;
  });
}

/**
 * Bind a reactive source list to a sort state and return the sorted view.
 * `source` may be a ref, a getter, or a plain array.
 */
export function useSort<T>(
  source: MaybeRefOrGetter<T[]>,
  accessors: Accessors<T>,
  initial: { key?: string | null; dir?: SortDir } = {},
) {
  const sort = useSortState(initial);
  const sorted = computed(() => sortRows(toValue(source), accessors, sort));
  return { sort, sorted };
}
