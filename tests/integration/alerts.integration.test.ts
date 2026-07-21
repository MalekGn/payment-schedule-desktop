// Integration suite — the Alertes read model over the real `api` facade.
//
// The Alertes page (src/views/AlertesView.vue) is built entirely from
// `api.listSchedule()` piped through the pure `buildAlerts` classifier. These
// tests cross-check that this derived model stays consistent with the other
// backend read models the app already trusts (the dashboard counters and the
// impayés list), and that a mutation (settling an overdue tranche) propagates
// into it — exactly what the UI relies on. As with the other integration
// suites, each test runs against a freshly re-seeded in-memory backend reached
// through the real `api` facade.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { buildAlerts, DEFAULT_SOON_DAYS } from "@/lib/alerts";
import { dayDiff, todayIso } from "@/lib/finance";

let api: typeof import("@/api").api;

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
});

describe("the alerts model reconciles with the schedule, dashboard, and impayés", () => {
  it("derives one actionable row per unpaid, in-window installment", async () => {
    const today = todayIso();
    const schedule = await api.listSchedule();
    const alerts = buildAlerts(schedule, today, DEFAULT_SOON_DAYS);

    // The seed anchors purchases months back, so overdue alerts must exist.
    expect(alerts.length).toBeGreaterThan(0);

    // Nothing fully paid, and nothing outside the overdue…soon window, leaks in.
    for (const a of alerts) {
      expect(a.remaining).toBeGreaterThan(0);
      expect(a.status).not.toBe("paid");
      const diff = dayDiff(a.dueDate, today);
      expect(diff).toBeLessThanOrEqual(DEFAULT_SOON_DAYS);
      if (a.kind === "overdue") expect(diff).toBeLessThan(0);
      if (a.kind === "dueToday") expect(diff).toBe(0);
      if (a.kind === "dueSoon") expect(diff).toBeGreaterThan(0);
      // The attached `days` figure is the absolute distance, always positive
      // for overdue/soon and zero exactly on the due day.
      expect(a.days).toBe(a.kind === "overdue" ? -diff : diff);
    }
  });

  it("agrees with the dashboard on the overdue installment count", async () => {
    const [schedule, dash] = await Promise.all([api.listSchedule(), api.getDashboard()]);
    const overdue = buildAlerts(schedule, todayIso()).filter((a) => a.kind === "overdue");
    expect(overdue.length).toBe(dash.stats.overdueCount);
  });

  it("matches impayés on the exact overdue installments and their remaining totals", async () => {
    const [schedule, impayes] = await Promise.all([api.listSchedule(), api.listImpayes()]);
    const overdue = buildAlerts(schedule, todayIso()).filter((a) => a.kind === "overdue");

    // Same set of installment ids as the impayés (past-due, unpaid) view.
    const fromAlerts = new Set(overdue.map((a) => a.installmentId));
    const fromImpayes = new Set(impayes.flatMap((c) => c.installments.map((i) => i.installmentId)));
    expect(fromAlerts).toEqual(fromImpayes);

    // And the same money: overdue remaining sums to the impayés grand total.
    const alertTotal = overdue.reduce((s, a) => s + a.remaining, 0);
    const impayeTotal = impayes.reduce((s, c) => s + c.totalOverdue, 0);
    expect(alertTotal).toBe(impayeTotal);
  });
});

describe("the alert window is a persisted setting that widens the due-soon set", () => {
  it("round-trips alertSoonDays through the api and grows dueSoon as it increases", async () => {
    const today = todayIso();
    const schedule = await api.listSchedule();

    // Default 7-day window on a fresh backend.
    const initial = await api.getSettings();
    expect(initial.alertSoonDays).toBe(7);

    const narrow = buildAlerts(schedule, today, initial.alertSoonDays).filter(
      (a) => a.kind === "dueSoon",
    );

    // Widen the window; the setting persists and re-reads.
    const updated = await api.updateSettings({ alertSoonDays: 30 });
    expect(updated.alertSoonDays).toBe(30);
    expect((await api.getSettings()).alertSoonDays).toBe(30);

    const wide = buildAlerts(schedule, today, 30).filter((a) => a.kind === "dueSoon");
    // A wider horizon can only add upcoming rows, never drop any.
    expect(wide.length).toBeGreaterThanOrEqual(narrow.length);
    const narrowIds = new Set(narrow.map((a) => a.installmentId));
    for (const id of narrowIds) {
      expect(wide.some((a) => a.installmentId === id)).toBe(true);
    }
  });

  it("clamps out-of-range values to the 1..90 bounds", async () => {
    expect((await api.updateSettings({ alertSoonDays: 0 })).alertSoonDays).toBe(1);
    expect((await api.updateSettings({ alertSoonDays: 999 })).alertSoonDays).toBe(90);
  });
});

describe("settling an overdue installment removes it from the alerts model", () => {
  it("drops the paid tranche and shrinks the overdue set by exactly one", async () => {
    const today = todayIso();
    const before = buildAlerts(await api.listSchedule(), today);
    const overdueBefore = before.filter((a) => a.kind === "overdue");
    expect(overdueBefore.length).toBeGreaterThan(0);

    const target = overdueBefore[0];
    await api.recordPayment({
      installmentId: target.installmentId,
      amount: target.remaining,
      paymentDate: today,
      note: null,
    });

    const after = buildAlerts(await api.listSchedule(), today);
    const overdueAfter = after.filter((a) => a.kind === "overdue");

    expect(overdueAfter.length).toBe(overdueBefore.length - 1);
    expect(after.some((a) => a.installmentId === target.installmentId)).toBe(false);
  });
});
