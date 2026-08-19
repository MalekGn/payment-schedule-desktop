// Unit tests for the report aggregation in the mock backend.
//
// These matter more than a mock's tests usually would: `src/api/mock.ts` is what
// the integration and E2E suites actually run against, so a divergence from the
// Rust `get_report_impl` would make both suites green while the desktop app
// behaved differently. Each case below mirrors one of the Rust tests by name, so
// the two can be read side by side.

import { describe, it, expect } from "vitest";

import { mockDb } from "@/api/mock";
import { todayIso } from "@/lib/finance";
import { AGING_BUCKETS } from "@/types/models";

/** An ISO date `n` days before today. */
function daysAgo(n: number): string {
  return new Date(Date.parse(`${todayIso()}T00:00:00Z`) - n * 86_400_000)
    .toISOString()
    .slice(0, 10);
}

const WIDE = { dateFrom: "2000-01-01", dateTo: "2049-12-31" };

describe("mockDb.getReport — range handling", () => {
  it("treats both ends of the range as inclusive", () => {
    // The seed's own data is irrelevant here; what matters is that a one-day
    // range is a real range rather than an empty one.
    const r = mockDb.getReport({ dateFrom: "2024-06-01", dateTo: "2024-06-01" });
    expect(r.range.from).toBe("2024-06-01");
    expect(r.range.to).toBe("2024-06-01");
    expect(r.collections).toHaveLength(1);
  });

  it("refuses a range it cannot serve, with an actionable code", () => {
    expect(() => mockDb.getReport({ dateFrom: "2024-06-30", dateTo: "2024-06-01" })).toThrow(
      "INVALID_DATE",
    );
    expect(() => mockDb.getReport({ dateFrom: "nope", dateTo: "2024-06-01" })).toThrow(
      "INVALID_DATE",
    );
    expect(() =>
      // @ts-expect-error deliberately outside the union, as a hostile caller would send
      mockDb.getReport({ dateFrom: "2024-01-01", dateTo: "2024-01-31", granularity: "fortnight" }),
    ).toThrow("INVALID_GRANULARITY");
    expect(() => mockDb.getReport({ dateFrom: "1900-01-01", dateTo: "2200-01-01" })).toThrow(
      /^REPORT_RANGE_TOO_LONG:/,
    );
  });

  it("refuses a granularity that would return thousands of buckets", () => {
    // Legal as a range, but daily buckets across a decade would serialize ~3650
    // points and ask the chart to draw as many bars. The auto-selected
    // granularity never gets near the cap, so only an explicit choice trips it.
    expect(() =>
      mockDb.getReport({ dateFrom: "2000-01-01", dateTo: "2019-12-31", granularity: "day" }),
    ).toThrow(/^REPORT_RANGE_TOO_LONG:/);
    // The same span with the granularity the UI actually sends is fine.
    expect(
      mockDb.getReport({ dateFrom: "2000-01-01", dateTo: "2019-12-31" }).collections.length,
    ).toBe(20);
  });

  it("picks a granularity from the span, and lets an explicit one win", () => {
    // Thresholds mirror REPORT_DAY_MAX_SPAN (62) and REPORT_MONTH_MAX_SPAN (730).
    const at = (spanDays: number) => {
      const from = "2024-01-01";
      const to = new Date(Date.parse(`${from}T00:00:00Z`) + (spanDays - 1) * 86_400_000)
        .toISOString()
        .slice(0, 10);
      return mockDb.getReport({ dateFrom: from, dateTo: to }).range.granularity;
    };
    expect(at(62)).toBe("day");
    expect(at(63)).toBe("month");
    expect(at(730)).toBe("month");
    expect(at(731)).toBe("year");

    const explicit = mockDb.getReport({
      dateFrom: "2024-01-01",
      dateTo: "2024-01-31",
      granularity: "year",
    });
    expect(explicit.range.granularity).toBe("year");
    expect(explicit.collections).toHaveLength(1);
  });
});

describe("mockDb.getReport — series and buckets", () => {
  it("carries every period including the empty ones", () => {
    const r = mockDb.getReport({ dateFrom: "2024-01-01", dateTo: "2024-03-31" });
    expect(r.range.granularity).toBe("month");
    expect(r.collections.map((p) => p.period)).toEqual(["2024-01", "2024-02", "2024-03"]);
  });

  it("emits all five aging buckets in a fixed order, always", () => {
    const r = mockDb.getReport(WIDE);
    expect(r.aging.map((b) => b.bucket)).toEqual([...AGING_BUCKETS]);
  });

  it("partitions what is owed across the buckets, losing nothing", () => {
    const r = mockDb.getReport(WIDE);
    const summed = r.aging.reduce((s, b) => s + b.amount, 0);
    expect(summed).toBe(r.totals.outstandingNow);

    // Overdue is everything except the not-yet-due bucket.
    const current = r.aging.find((b) => b.bucket === "current")!;
    expect(r.totals.overdueNow).toBe(summed - current.amount);
  });

  it("puts an installment due today in `current`, not in `1-30`", () => {
    // `daysLate` is today - dueDate, so due-today is 0 days late. Asserted
    // through the boundary helper rather than the seed, which has no
    // installment reliably due exactly today.
    const before = mockDb.getReport(WIDE);
    const currentBefore = before.aging.find((b) => b.bucket === "current")!.amount;

    const client = mockDb.createClient({
      firstName: "Aging",
      lastName: "Edge",
      phone: "",
      address: "",
      email: null,
    });
    mockDb.createPurchase({
      clientId: client.id,
      productLabel: "Test",
      totalPrice: 500,
      installmentCount: 1,
      intervalKind: "monthly",
      intervalDays: null,
      purchaseDate: todayIso(),
      installments: [{ index: 1, amount: 500, dueDate: todayIso() }],
    });

    const after = mockDb.getReport(WIDE);
    expect(after.aging.find((b) => b.bucket === "current")!.amount).toBe(currentBefore + 500);
  });

  it("keeps an overdue installment out of `current`", () => {
    const client = mockDb.createClient({
      firstName: "Late",
      lastName: "Payer",
      phone: "",
      address: "",
      email: null,
    });
    const due = daysAgo(45);
    mockDb.createPurchase({
      clientId: client.id,
      productLabel: "Congélateur",
      totalPrice: 700,
      installmentCount: 1,
      intervalKind: "monthly",
      intervalDays: null,
      purchaseDate: due,
      installments: [{ index: 1, amount: 700, dueDate: due }],
    });

    const r = mockDb.getReport(WIDE);
    const risk = r.topClients.find((c) => c.clientId === client.id);
    expect(risk?.overdue).toBeGreaterThanOrEqual(700);
    expect(risk?.overdueCount).toBeGreaterThanOrEqual(1);
  });
});

describe("mockDb.getReport — totals", () => {
  it("agrees with the payment ledger over the same window", () => {
    const r = mockDb.getReport(WIDE);
    // `listAllPayments` caps its own result, so the comparison is drawn against
    // a generous limit — the point is that `collected` is not itself capped.
    const ledger = mockDb
      .listAllPayments(5000)
      .filter((p) => p.paymentDate >= WIDE.dateFrom && p.paymentDate <= WIDE.dateTo)
      .reduce((s, p) => s + p.amount, 0);
    expect(r.totals.collected).toBe(ledger);
  });

  it("reports whole currency units only — no floats reach the money figures", () => {
    const r = mockDb.getReport(WIDE);
    for (const value of Object.values(r.totals)) {
      expect(Number.isInteger(value)).toBe(true);
    }
    for (const b of r.aging) expect(Number.isInteger(b.amount)).toBe(true);
    for (const p of r.collections) {
      expect(Number.isInteger(p.collected)).toBe(true);
      expect(Number.isInteger(p.due)).toBe(true);
    }
  });

  it("excludes an archived purchase from sales, outstanding and aging", () => {
    const client = mockDb.createClient({
      firstName: "Archive",
      lastName: "Case",
      phone: "",
      address: "",
      email: null,
    });
    const purchase = mockDb.createPurchase({
      clientId: client.id,
      productLabel: "Téléviseur",
      totalPrice: 900,
      installmentCount: 1,
      intervalKind: "monthly",
      intervalDays: null,
      purchaseDate: daysAgo(10),
      installments: [{ index: 1, amount: 900, dueDate: daysAgo(10) }],
    });

    const before = mockDb.getReport(WIDE);
    mockDb.archivePurchase(purchase.purchase.id);
    const after = mockDb.getReport(WIDE);

    expect(before.totals.salesAmount - after.totals.salesAmount).toBe(900);
    expect(before.totals.outstandingNow - after.totals.outstandingNow).toBe(900);
    expect(after.topClients.some((c) => c.clientId === client.id)).toBe(false);
  });
});
