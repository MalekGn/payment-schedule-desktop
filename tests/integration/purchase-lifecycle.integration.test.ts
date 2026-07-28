// Integration suite — purchase → schedule → payment lifecycle.
//
// Unlike the unit tests (which poke individual pure functions), these exercise
// the real `api` facade end-to-end: `@/api` decides mock-vs-Tauri, delegates to
// `mockDb`, which in turn leans on the `@/lib/finance` engine to split amounts,
// roll installment statuses up to a purchase status, and keep the dashboard
// aggregates in sync. We assert that those layers agree with one another across
// a full create → pay-off flow.
//
// Outside a Tauri runtime `isTauri()` is false, so every call resolves against
// the in-memory backend. That backend is a module-level singleton seeded on
// construction, so we reset the module registry before each test to hand every
// case an identical, isolated dataset (6 clients / 8 purchases).
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { splitAmounts, todayIso } from "@/lib/finance";
import type { PurchaseInput } from "@/types/models";

let api: typeof import("@/api").api;

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
});

const CLIENT_ID = 1; // Mohamed Trabelsi — always present in the seed.

function newPurchase(over: Partial<PurchaseInput> = {}): PurchaseInput {
  return {
    clientId: CLIENT_ID,
    productLabel: "Aspirateur Dyson",
    totalPrice: 1000,
    installmentCount: 3,
    intervalKind: "monthly",
    intervalDays: null,
    purchaseDate: todayIso(),
    installments: null,
    ...over,
  };
}

describe("creating a purchase wires the finance split into the backend", () => {
  it("auto-splits the total across installments and starts fully pending", async () => {
    const detail = await api.createPurchase(newPurchase({ totalPrice: 1000, installmentCount: 3 }));

    // splitAmounts drops the rounding remainder on the last tranche: 1000/3.
    expect(detail.installments.map((i) => i.amount)).toEqual(splitAmounts(1000, 3));
    expect(detail.installments.map((i) => i.amount)).toEqual([333, 333, 334]);
    expect(detail.installments.reduce((s, i) => s + i.amount, 0)).toBe(1000);

    // A brand-new purchase dated today has nothing paid and nothing overdue.
    expect(detail.installments.every((i) => i.status === "pending")).toBe(true);
    expect(detail.totalPaid).toBe(0);
    expect(detail.remaining).toBe(1000);
    expect(detail.status).toBe("pending");

    // It is the 9th purchase and is immediately listable + searchable.
    expect(detail.purchase.reference).toBe("A-000009");
    // `listPurchases` takes the scope first now, then the search.
    const list = await api.listPurchases("active", "A-000009");
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe(detail.purchase.id);
  });

  it("honours a caller-supplied uneven split when the sum matches the total", async () => {
    const detail = await api.createPurchase(
      newPurchase({
        totalPrice: 1000,
        installmentCount: 2,
        installments: [
          { index: 1, amount: 600, dueDate: "2026-08-01" },
          { index: 2, amount: 400, dueDate: "2026-09-01" },
        ],
      }),
    );
    expect(detail.installments.map((i) => i.amount)).toEqual([600, 400]);
    expect(detail.installments.map((i) => i.dueDate)).toEqual(["2026-08-01", "2026-09-01"]);
  });

  it("rejects an explicit split whose amounts do not add up to the total", async () => {
    let error: unknown;
    try {
      await api.createPurchase(
        newPurchase({
          totalPrice: 1000,
          installmentCount: 2,
          installments: [
            { index: 1, amount: 600, dueDate: "2026-08-01" },
            { index: 2, amount: 300, dueDate: "2026-09-01" }, // 900 ≠ 1000
          ],
        }),
      );
    } catch (e) {
      error = e;
    }
    expect(String(error)).toMatch(/SUM_MISMATCH:900:1000/);
  });
});

describe("recording payments drives installment and purchase status transitions", () => {
  it("moves an installment pending → partial → paid and the purchase to in_progress", async () => {
    const created = await api.createPurchase(
      newPurchase({ totalPrice: 1000, installmentCount: 3 }),
    );
    const first = created.installments[0]; // 333 due today, pending
    const today = todayIso();

    // A partial payment leaves a remaining balance -> "partial", purchase moves.
    let detail = await api.recordPayment({
      installmentId: first.id,
      amount: 100,
      paymentDate: today,
      note: null,
    });
    expect(detail.installments[0].status).toBe("partial");
    expect(detail.totalPaid).toBe(100);
    expect(detail.remaining).toBe(900);
    expect(detail.status).toBe("in_progress");

    // Paying off the rest of that tranche flips it to "paid"; others still pending.
    detail = await api.recordPayment({
      installmentId: first.id,
      amount: 233,
      paymentDate: today,
      note: null,
    });
    expect(detail.installments[0].status).toBe("paid");
    expect(detail.installments[1].status).toBe("pending");
    expect(detail.status).toBe("in_progress");
  });

  it("marks the purchase paid once every installment is settled", async () => {
    const created = await api.createPurchase(
      newPurchase({ totalPrice: 1000, installmentCount: 3 }),
    );
    const today = todayIso();
    for (const inst of created.installments) {
      await api.recordPayment({
        installmentId: inst.id,
        amount: inst.amount, // pay each tranche in full
        paymentDate: today,
        note: null,
      });
    }
    const detail = await api.getPurchaseDetail(created.purchase.id);
    expect(detail.installments.every((i) => i.status === "paid")).toBe(true);
    expect(detail.remaining).toBe(0);
    expect(detail.totalPaid).toBe(1000);
    expect(detail.status).toBe("paid");

    // The payment ledger records one row per recordPayment call, newest-first.
    const payments = await api.listPaymentsForPurchase(created.purchase.id);
    expect(payments).toHaveLength(3);
    expect(payments.reduce((s, p) => s + p.amount, 0)).toBe(1000);
  });

  it("rejects a non-positive payment amount", async () => {
    const created = await api.createPurchase(newPurchase());
    let error: unknown;
    try {
      await api.recordPayment({
        installmentId: created.installments[0].id,
        amount: 0,
        paymentDate: todayIso(),
        note: null,
      });
    } catch (e) {
      error = e;
    }
    expect(String(error)).toMatch(/INVALID_AMOUNT/);
  });
});

describe("a new purchase and its payments propagate into the dashboard aggregates", () => {
  it("bumps purchase/sales counts on create and collected total on each payment", async () => {
    const before = await api.getDashboard();
    expect(before.stats.totalPurchases).toBe(8); // seed baseline

    const created = await api.createPurchase(
      newPurchase({ totalPrice: 1000, installmentCount: 3 }),
    );
    const afterCreate = await api.getDashboard();
    expect(afterCreate.stats.totalPurchases).toBe(before.stats.totalPurchases + 1);
    expect(afterCreate.stats.totalSales).toBe(before.stats.totalSales + 1000);
    // Nothing paid yet: collected is unchanged, outstanding grew by the full price.
    expect(afterCreate.stats.totalCollected).toBe(before.stats.totalCollected);
    expect(afterCreate.stats.totalOutstanding).toBe(before.stats.totalOutstanding + 1000);

    await api.recordPayment({
      installmentId: created.installments[0].id,
      amount: 333,
      paymentDate: todayIso(),
      note: null,
    });
    const afterPay = await api.getDashboard();
    expect(afterPay.stats.totalCollected).toBe(before.stats.totalCollected + 333);
    expect(afterPay.stats.totalOutstanding).toBe(before.stats.totalOutstanding + 1000 - 333);
  });
});
