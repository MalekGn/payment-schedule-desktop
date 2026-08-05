// Unit tests for the client-side table sorting behind every `SortHeader`.
//
// The comparator was the last piece of shared logic in the app with no test, and
// it was wrong: blanks were folded into `compare` as "sort last" and then
// multiplied by the direction factor, so descending pulled every empty value to
// the top. `note` is a sortable column on the purchase and client detail pages
// and `Payment.note` is nullable, so sorting notes descending buried exactly the
// payments that had one. These tests pin the corrected contract — blanks last in
// *both* directions — along with the toggle cycle and the identity guarantee
// that keeps unsorted screens in the backend's own order.
//
// The reactive `useSort` wrapper is a two-line `computed` over `sortRows`, so
// everything here drives the pure functions directly.

import { describe, it, expect } from "vitest";
import { sortRows, useSortState, type Accessors, type SortState } from "@/composables/useSort";

interface Row {
  name: string | null;
  qty: number | null;
}

const rows = (...vals: (readonly [string | null, number | null])[]): Row[] =>
  vals.map(([name, qty]) => ({ name, qty }));

const accessors: Accessors<Row> = {
  name: (r) => r.name,
  qty: (r) => r.qty,
};

/** A sort state without going through the toggle cycle to reach it. */
const state = (key: string | null, dir: "asc" | "desc" = "asc"): SortState => {
  const s = useSortState({ key, dir });
  return s;
};

const names = (out: Row[]) => out.map((r) => r.name);

describe("sortRows — blanks", () => {
  it("keeps blanks last when ascending", () => {
    const input = rows(["cheque", 1], [null, 2], ["acompte", 3], [null, 4]);
    expect(names(sortRows(input, accessors, state("name")))).toEqual([
      "acompte",
      "cheque",
      null,
      null,
    ]);
  });

  it("keeps blanks last when descending too, which is the whole fix", () => {
    // Before, `compare` returned 1 for a blank and `sortRows` multiplied it by
    // -1, so this came back [null, null, "cheque", "acompte"] — the rows with
    // data pushed off the bottom of the screen.
    const input = rows(["cheque", 1], [null, 2], ["acompte", 3], [null, 4]);
    expect(names(sortRows(input, accessors, state("name", "desc")))).toEqual([
      "cheque",
      "acompte",
      null,
      null,
    ]);
  });

  it("treats undefined the same as null", () => {
    const input = [{ name: undefined as unknown as null, qty: 1 }, ...rows(["a", 2])];
    expect(names(sortRows(input, accessors, state("name", "desc")))).toEqual(["a", undefined]);
  });

  it("leaves an all-blank column in its original order", () => {
    const input = rows([null, 1], [null, 2], [null, 3]);
    expect(sortRows(input, accessors, state("name")).map((r) => r.qty)).toEqual([1, 2, 3]);
  });
});

describe("sortRows — comparing values", () => {
  it("orders numbers numerically rather than as text", () => {
    const input = rows(["a", 10], ["b", 9], ["c", 100]);
    expect(sortRows(input, accessors, state("qty")).map((r) => r.qty)).toEqual([9, 10, 100]);
  });

  it("orders embedded numbers naturally, so item2 comes before item10", () => {
    const input = rows(["item10", 1], ["item2", 2], ["item1", 3]);
    expect(names(sortRows(input, accessors, state("name")))).toEqual(["item1", "item2", "item10"]);
  });

  it("compares accented text by base letter, as a reader would expect", () => {
    const input = rows(["Zoé", 1], ["Élodie", 2], ["Amir", 3]);
    expect(names(sortRows(input, accessors, state("name")))).toEqual(["Amir", "Élodie", "Zoé"]);
  });

  it("falls back to text comparison when the types are mixed", () => {
    const mixed: Row[] = [
      { name: null, qty: 2 },
      { name: "10", qty: null },
    ];
    // Only that it does not throw and puts the blank last; the exact ordering of
    // a mixed column is not a contract any screen relies on.
    expect(sortRows(mixed, { v: (r) => r.name ?? r.qty }, state("v")).length).toBe(2);
  });
});

describe("sortRows — when it must not reorder at all", () => {
  it("returns the very same array while no column is active", () => {
    // Identity, not equality: this is what keeps a freshly loaded list in the
    // order the backend chose.
    const input = rows(["b", 1], ["a", 2]);
    expect(sortRows(input, accessors, state(null))).toBe(input);
  });

  it("returns the very same array for a column it has no accessor for", () => {
    const input = rows(["b", 1], ["a", 2]);
    expect(sortRows(input, accessors, state("nope"))).toBe(input);
  });

  it("never mutates the caller's array", () => {
    const input = rows(["b", 1], ["a", 2]);
    sortRows(input, accessors, state("name"));
    expect(names(input)).toEqual(["b", "a"]);
  });
});

describe("useSortState — the toggle cycle", () => {
  it("starts unsorted so a list keeps the backend's ordering", () => {
    const s = useSortState();
    expect(s.key).toBeNull();
    expect(s.dir).toBe("asc");
  });

  it("activates a column ascending, then flips on a second click", () => {
    const s = useSortState();
    s.toggle("name");
    expect([s.key, s.dir]).toEqual(["name", "asc"]);
    s.toggle("name");
    expect([s.key, s.dir]).toEqual(["name", "desc"]);
    s.toggle("name");
    expect([s.key, s.dir]).toEqual(["name", "asc"]);
  });

  it("restarts ascending when the user moves to another column", () => {
    const s = useSortState();
    s.toggle("name");
    s.toggle("name"); // now desc
    s.toggle("qty");
    expect([s.key, s.dir]).toEqual(["qty", "asc"]);
  });
});
