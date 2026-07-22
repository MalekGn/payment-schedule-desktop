// Unit tests for the alert-derivation logic behind the Alertes page. These pin
// the classification boundaries (overdue / due-today / due-soon / ignored) with
// a fixed `today`, so they stay stable regardless of the real calendar.

import { describe, it, expect } from "vitest";
import { buildAlerts, classifyAlert, DEFAULT_SOON_DAYS } from "@/lib/alerts";
import type { InstallmentStatus, ScheduleRow } from "@/types/models";

const TODAY = "2026-07-22";

function row(o: Partial<ScheduleRow> & { dueDate: string }): ScheduleRow {
  const amount = o.amount ?? 400;
  const paidAmount = o.paidAmount ?? 0;
  return {
    installmentId: o.installmentId ?? 1,
    purchaseId: o.purchaseId ?? 1,
    reference: o.reference ?? "A-000001",
    clientId: o.clientId ?? 1,
    clientName: o.clientName ?? "Sample Client",
    index: o.index ?? 1,
    installmentCount: o.installmentCount ?? 6,
    dueDate: o.dueDate,
    amount,
    paidAmount,
    remaining: o.remaining ?? amount - paidAmount,
    status: (o.status ?? "pending") as InstallmentStatus,
  };
}

describe("classifyAlert", () => {
  it("flags a past-due unpaid row as overdue with positive days late", () => {
    const a = classifyAlert(row({ dueDate: "2026-07-19", status: "late" }), TODAY);
    expect(a).toMatchObject({ kind: "overdue", days: 3 });
  });

  it("flags a row due exactly today as dueToday with zero days", () => {
    const a = classifyAlert(row({ dueDate: TODAY }), TODAY);
    expect(a).toMatchObject({ kind: "dueToday", days: 0 });
  });

  it("flags a row inside the horizon as dueSoon with days remaining", () => {
    const a = classifyAlert(row({ dueDate: "2026-07-27" }), TODAY);
    expect(a).toMatchObject({ kind: "dueSoon", days: 5 });
  });

  it("includes the last day of the horizon but excludes the day after", () => {
    expect(classifyAlert(row({ dueDate: "2026-07-29" }), TODAY, 7)?.kind).toBe("dueSoon");
    expect(classifyAlert(row({ dueDate: "2026-07-30" }), TODAY, 7)).toBeNull();
  });

  it("ignores fully paid rows even when past due", () => {
    expect(
      classifyAlert(row({ dueDate: "2026-07-01", remaining: 0, status: "paid" }), TODAY),
    ).toBeNull();
  });

  it("still alerts on a partially paid overdue row (remaining > 0)", () => {
    const a = classifyAlert(
      row({ dueDate: "2026-07-15", amount: 400, paidAmount: 100, status: "late" }),
      TODAY,
    );
    expect(a).toMatchObject({ kind: "overdue" });
    expect(a?.remaining).toBe(300);
  });

  it("respects a custom horizon", () => {
    expect(classifyAlert(row({ dueDate: "2026-07-25" }), TODAY, 2)).toBeNull();
    expect(classifyAlert(row({ dueDate: "2026-07-25" }), TODAY, 3)?.kind).toBe("dueSoon");
  });
});

describe("buildAlerts", () => {
  const rows = [
    row({ installmentId: 1, dueDate: "2026-07-10", status: "late" }), // overdue
    row({ installmentId: 2, dueDate: TODAY }), // due today
    row({ installmentId: 3, dueDate: "2026-07-26" }), // due soon
    row({ installmentId: 4, dueDate: "2026-12-01" }), // far future → dropped
    row({ installmentId: 5, dueDate: "2026-06-01", remaining: 0, status: "paid" }), // paid → dropped
  ];

  it("keeps only actionable rows, preserving input order", () => {
    const out = buildAlerts(rows, TODAY, DEFAULT_SOON_DAYS);
    expect(out.map((a) => a.installmentId)).toEqual([1, 2, 3]);
    expect(out.map((a) => a.kind)).toEqual(["overdue", "dueToday", "dueSoon"]);
  });

  it("returns an empty array when nothing is actionable", () => {
    expect(buildAlerts([rows[3], rows[4]], TODAY)).toEqual([]);
  });
});
