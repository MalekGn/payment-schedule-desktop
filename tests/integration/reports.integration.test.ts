// Integration suite — the Rapports read model over the real `api` facade.
//
// The property these tests exist to protect is the one that motivated putting
// the aggregation in the Rust core: a report must agree with the read models the
// app already trusts. `api.listAllPayments` caps its result, so a report built
// in the renderer would drift from the ledger silently, and every screen would
// still look right. These tests cross-check the report against the dashboard,
// the schedule and the payment ledger, and pin the two-population split
// (historical period figures vs. an as-of-today snapshot) that the screen
// labels but nothing else enforces.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";

import { todayIso } from "@/lib/finance";
import { AGING_BUCKETS } from "@/types/models";

let api: typeof import("@/api").api;

/** Wide enough to cover the whole seed, narrow enough to stay under the span cap. */
const WIDE = { dateFrom: "2000-01-01", dateTo: "2049-12-31" };

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
});

/**
 * Run a call that must fail, and return the message it failed with.
 *
 * Not `expect(...).rejects`: the gateway builds the browser path as
 * `Promise.resolve(mockDb.x())`, so the mock runs before the promise exists and
 * a rejection surfaces as a synchronous throw. Awaiting inside a `try` reads
 * both backends identically — the same helper the archive suites use.
 */
async function failureOf(call: () => Promise<unknown>): Promise<string> {
  try {
    await call();
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
  throw new Error("expected the call to fail, but it resolved");
}

describe("the report reconciles with the read models the app already trusts", () => {
  it("reports the same outstanding and overdue totals as the dashboard", async () => {
    const [report, dashboard] = await Promise.all([api.getReport(WIDE), api.getDashboard()]);

    // Both are as-of-today snapshots over live purchases, so they must agree
    // exactly — a divergence means one of them forgot the archived filter.
    expect(report.totals.outstandingNow).toBe(dashboard.stats.totalOutstanding);
    expect(report.totals.collected).toBe(dashboard.stats.totalCollected);
    expect(report.totals.salesAmount).toBe(dashboard.stats.totalSales);
    expect(report.totals.salesCount).toBe(dashboard.stats.totalPurchases);
  });

  it("agrees with the payment ledger rather than with a capped page of it", async () => {
    const [report, payments] = await Promise.all([api.getReport(WIDE), api.listAllPayments(5000)]);
    const ledger = payments
      .filter((p) => p.paymentDate >= WIDE.dateFrom && p.paymentDate <= WIDE.dateTo)
      .reduce((sum, p) => sum + p.amount, 0);

    expect(report.totals.collected).toBe(ledger);
    expect(report.totals.paymentCount).toBe(payments.length);
  });

  it("ages exactly the money the schedule still shows as owed", async () => {
    const [report, schedule] = await Promise.all([api.getReport(WIDE), api.listSchedule()]);
    const owed = schedule.reduce((sum, row) => sum + row.remaining, 0);

    const aged = report.aging.reduce((sum, b) => sum + b.amount, 0);
    expect(aged).toBe(owed);
    expect(aged).toBe(report.totals.outstandingNow);

    // Every overdue row in the schedule is money in a late bucket.
    const today = todayIso();
    const late = schedule
      .filter((row) => row.dueDate < today && row.remaining > 0)
      .reduce((sum, row) => sum + row.remaining, 0);
    const notYetDue = report.aging.find((b) => b.bucket === "current")!.amount;
    expect(aged - notYetDue).toBe(late);
    expect(report.totals.overdueNow).toBe(late);
  });

  it("never omits a bucket, however empty the range", async () => {
    // A range with no activity at all still has to render a full table.
    const quiet = await api.getReport({ dateFrom: "2001-01-01", dateTo: "2001-01-31" });
    expect(quiet.aging.map((b) => b.bucket)).toEqual([...AGING_BUCKETS]);
    expect(quiet.collections).toHaveLength(31);
    expect(quiet.totals.salesCount).toBe(0);
    expect(quiet.totals.collected).toBe(0);
  });
});

describe("period figures move with the range; balance figures do not", () => {
  it("holds the as-of snapshot steady while the period window changes", async () => {
    const wide = await api.getReport(WIDE);
    const narrow = await api.getReport({ dateFrom: "2001-01-01", dateTo: "2001-01-31" });

    // The whole reason `asOf` is echoed back: these are not period figures, and
    // the screen labels them separately.
    expect(narrow.range.asOf).toBe(todayIso());
    expect(narrow.totals.outstandingNow).toBe(wide.totals.outstandingNow);
    expect(narrow.totals.overdueNow).toBe(wide.totals.overdueNow);
    expect(narrow.aging).toEqual(wide.aging);

    // ...whereas these are.
    expect(narrow.totals.collected).toBeLessThanOrEqual(wide.totals.collected);
    expect(narrow.totals.salesCount).toBeLessThanOrEqual(wide.totals.salesCount);
  });

  it("moves collected into the report as soon as a payment is recorded", async () => {
    const purchases = await api.listPurchases();
    const target = purchases.find((p) => p.remaining > 0);
    expect(target, "the seed must contain an unsettled purchase").toBeDefined();

    const detail = await api.getPurchaseDetail(target!.id);
    const next = detail.installments.find((i) => i.amount > i.paidAmount)!;
    const today = todayIso();
    const range = { dateFrom: today, dateTo: today };

    const before = await api.getReport(range);
    await api.recordPayment({
      installmentId: next.id,
      amount: 1,
      paymentDate: today,
      note: "integration",
    });
    const after = await api.getReport(range);

    expect(after.totals.collected).toBe(before.totals.collected + 1);
    expect(after.totals.paymentCount).toBe(before.totals.paymentCount + 1);
    // A payment reduces what is owed, so the snapshot moves the other way.
    expect(after.totals.outstandingNow).toBe(before.totals.outstandingNow - 1);
  });
});

describe("rejections stay actionable across the IPC boundary", () => {
  it("refuses an inverted range with a code, not a raw message", async () => {
    const message = await failureOf(() =>
      api.getReport({ dateFrom: "2026-06-30", dateTo: "2026-06-01" }),
    );
    expect(message).toBe("INVALID_DATE");
  });

  it("refuses a range too wide to bucket", async () => {
    const message = await failureOf(() =>
      api.getReport({ dateFrom: "2000-01-01", dateTo: "2019-12-31", granularity: "day" }),
    );
    expect(message).toMatch(/^REPORT_RANGE_TOO_LONG:\d+$/);
  });
});

describe("the CSV export goes through the gateway, not the DOM", () => {
  it("hands the rendered file to the backend rather than an <a download>", async () => {
    // The bug this covers: both export buttons used to build a Blob and click a
    // detached anchor, which is inert inside the Tauri WebView — no file, no
    // error. The gateway now owns the split, so the export is observable here.
    const { mockDb } = await import("@/api/mock");
    expect(mockDb.lastCsvExport).toBeNull();

    const report = await api.getReport(WIDE);
    const written = await api.saveCsv("rapport-test.csv", `sales,${report.totals.salesAmount}`);

    expect(written).toBe(true);
    expect(mockDb.lastCsvExport?.name).toBe("rapport-test.csv");
    expect(mockDb.lastCsvExport?.contents).toContain(String(report.totals.salesAmount));
  });
});
