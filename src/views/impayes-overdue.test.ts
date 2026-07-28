// Unit tests for the data + sorting logic behind the Overdue (Impayés) page.
//
// The page (src/views/ImpayesView.vue) calls `api.listImpayes()` with no
// backend filter and does all search/amount/date filtering and sorting on the
// client. These tests pin down the two pure foundations it stands on:
//   1. the overdue dataset itself — `mockDb.listImpayes()`, which mirrors the
//      Rust `build_impayes` command (past-due + unpaid, grouped per client,
//      most-owed first);
//   2. the shared `sortRows` engine driven by the exact column accessors the
//      view declares for each client's installment table.
// Assertions are written to be independent of the current date wherever the
// seed's relative dates would otherwise make them brittle.

import { describe, it, expect } from "vitest";
import { mockDb } from "@/api/mock";
import { sortRows, useSortState } from "@/composables/useSort";
import { todayIso } from "@/lib/finance";
import type { OverdueInstallment } from "@/types/models";

describe("mockDb.listImpayes — overdue dataset invariants", () => {
  const today = todayIso();

  it("returns only past-due installments with a remaining balance", () => {
    const clients = mockDb.listImpayes();
    // The seed anchors purchases up to 6 months back, so there is overdue data.
    expect(clients.length).toBeGreaterThan(0);
    for (const c of clients) {
      expect(c.installments.length).toBeGreaterThan(0);
      for (const i of c.installments) {
        expect(i.dueDate < today).toBe(true); // strictly before today
        expect(i.remaining).toBeGreaterThan(0);
        expect(i.remaining).toBeLessThanOrEqual(i.amount);
        expect(i.daysLate).toBeGreaterThan(0);
      }
    }
  });

  it("keeps per-client totals consistent with their installments", () => {
    for (const c of mockDb.listImpayes()) {
      const sum = c.installments.reduce((s, i) => s + i.remaining, 0);
      expect(c.totalOverdue).toBe(sum);
      expect(c.overdueCount).toBe(c.installments.length);
    }
  });

  it("orders clients most-owed first and lists each client once", () => {
    const clients = mockDb.listImpayes();
    const totals = clients.map((c) => c.totalOverdue);
    const descending = [...totals].sort((a, b) => b - a);
    expect(totals).toEqual(descending);

    const ids = clients.map((c) => c.clientId);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("applies the backend clientId filter to a single client", () => {
    const all = mockDb.listImpayes();
    const target = all[0].clientId;
    const one = mockDb.listImpayes({ clientId: target });
    expect(one).toHaveLength(1);
    expect(one[0].clientId).toBe(target);
  });

  it("returns nothing when the date window excludes every due date", () => {
    expect(mockDb.listImpayes({ dateFrom: "2999-01-01" })).toEqual([]);
    expect(mockDb.listImpayes({ dateTo: "1900-01-01" })).toEqual([]);
  });
});

describe("sortRows with the Overdue page's installment accessors", () => {
  // Mirrors `instAccessors` in ImpayesView.vue. Kept in sync by hand: if the
  // view's columns change, update these too.
  const instAccessors = {
    reference: (i: OverdueInstallment) => i.purchaseReference,
    tranche: (i: OverdueInstallment) => i.index,
    dueDate: (i: OverdueInstallment) => i.dueDate,
    amount: (i: OverdueInstallment) => i.remaining,
    since: (i: OverdueInstallment) => i.daysLate,
  };

  const make = (o: Partial<OverdueInstallment>): OverdueInstallment => ({
    installmentId: 0,
    purchaseId: 0,
    purchaseReference: "A-000001",
    index: 1,
    installmentCount: 6,
    dueDate: "2026-01-01",
    amount: 400,
    remaining: 400,
    daysLate: 1,
    ...o,
  });

  const rows: OverdueInstallment[] = [
    make({
      purchaseReference: "A-000003",
      index: 2,
      dueDate: "2026-03-10",
      remaining: 150,
      daysLate: 30,
    }),
    make({
      purchaseReference: "A-000001",
      index: 5,
      dueDate: "2026-01-05",
      remaining: 900,
      daysLate: 5,
    }),
    make({
      purchaseReference: "A-000002",
      index: 1,
      dueDate: "2026-02-20",
      remaining: 400,
      daysLate: 120,
    }),
  ];

  function sortedBy(field: string, dir: "asc" | "desc") {
    const sort = useSortState();
    sort.key = field;
    sort.dir = dir;
    return sortRows(rows, instAccessors, sort);
  }

  it("leaves order untouched until a column is chosen", () => {
    const sort = useSortState(); // key === null
    expect(sortRows(rows, instAccessors, sort)).toEqual(rows);
  });

  it("sorts the amount column numerically (remaining), both directions", () => {
    expect(sortedBy("amount", "asc").map((i) => i.remaining)).toEqual([150, 400, 900]);
    expect(sortedBy("amount", "desc").map((i) => i.remaining)).toEqual([900, 400, 150]);
  });

  it("sorts days-late numerically, not lexically (120 > 30 > 5)", () => {
    expect(sortedBy("since", "desc").map((i) => i.daysLate)).toEqual([120, 30, 5]);
  });

  it("sorts the due date column chronologically", () => {
    expect(sortedBy("dueDate", "asc").map((i) => i.dueDate)).toEqual([
      "2026-01-05",
      "2026-02-20",
      "2026-03-10",
    ]);
  });

  it("sorts the reference column as text", () => {
    expect(sortedBy("reference", "asc").map((i) => i.purchaseReference)).toEqual([
      "A-000001",
      "A-000002",
      "A-000003",
    ]);
  });

  it("does not mutate the source array", () => {
    const before = rows.map((i) => i.purchaseReference);
    sortedBy("amount", "desc");
    expect(rows.map((i) => i.purchaseReference)).toEqual(before);
  });
});
