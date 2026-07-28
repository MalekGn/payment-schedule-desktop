// Integration suite — updating a single installment.
//
// `updatePurchase` is refused the moment a payment exists: saving there deletes
// and reinserts the installment rows, and those rows own the payments through
// an `ON DELETE CASCADE`. `updateInstallment` is the path that stays open, and
// it is now the *only* editor of an installment — it absorbed the old payment
// modal, so it moves both what is owed and what has been collected.
//
// What these tests pin down is that the extra reach costs none of the money
// invariants the rest of the app leans on:
//
//   * `SUM(amount) === purchase.totalPrice` — never written, always rebalanced;
//   * `paidAmount <= amount` on every row, which keeps the outstanding and
//     overdue aggregates from going negative;
//   * `SUM(payments) === SUM(paidAmount)` — the dashboard's "Amount collected"
//     is the only money figure read from the ledger, so a paid-amount edit that
//     skipped the ledger would make that tile contradict every other total.
//
// The four product rules are asserted alongside: the schedule (amount, due
// date) is editable until the installment settles and ignores its neighbours;
// the money (paid amount, payment date) is gated on the previous installment
// and ignores this one's own status.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PurchaseDetail } from "@/types/models";

let api: typeof import("@/api").api;

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
});

/**
 * Run a call that must fail, and return the message it failed with.
 *
 * Not `expect(...).rejects`: the gateway builds the browser path as
 * `Promise.resolve(mockDb.x())`, so the mock runs before the promise exists and
 * a rejection surfaces as a synchronous throw.
 */
async function failureOf(call: () => Promise<unknown>): Promise<string> {
  try {
    await call();
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
  throw new Error("expected the call to fail, but it resolved");
}

/** A fresh 1000-over-4 purchase, monthly from 2024-01-15, nothing paid. */
async function freshPurchase(): Promise<PurchaseDetail> {
  const client = (await api.listClients())[0];
  return api.createPurchase({
    clientId: client.id,
    productLabel: "Réfrigérateur",
    totalPrice: 1000,
    installmentCount: 4,
    intervalKind: "monthly",
    intervalDays: null,
    purchaseDate: "2024-01-15",
  });
}

const amountsOf = (d: PurchaseDetail): number[] => d.installments.map((i) => i.amount);

/** Assert every invariant that must survive an edit, ledger included. */
async function expectConsistent(detail: PurchaseDetail): Promise<void> {
  expect(amountsOf(detail).reduce((s, v) => s + v, 0)).toBe(detail.purchase.totalPrice);
  for (const inst of detail.installments) {
    expect(inst.paidAmount).toBeLessThanOrEqual(inst.amount);
    expect(Number.isInteger(inst.amount)).toBe(true);
  }
  // The ledger has to agree with the cache, or the dashboard's "Amount
  // collected" silently disagrees with every purchase and client total.
  const payments = await api.listPaymentsForPurchase(detail.purchase.id);
  expect(payments.reduce((s, p) => s + p.amount, 0)).toBe(
    detail.installments.reduce((s, i) => s + i.paidAmount, 0),
  );
}

/** Settle installment at `pos` (0-based) through the ordinary payment path. */
async function settle(detail: PurchaseDetail, pos: number, amount?: number): Promise<void> {
  const inst = detail.installments[pos];
  await api.recordPayment({
    installmentId: inst.id,
    amount: amount ?? inst.amount,
    paymentDate: "2024-02-01",
    note: null,
  });
}

describe("the schedule half — editable until the installment settles", () => {
  it("absorbs a changed amount into the later tranches, holding the total", async () => {
    const detail = await freshPurchase();
    expect(amountsOf(detail)).toEqual([250, 250, 250, 250]);

    const updated = await api.updateInstallment(detail.installments[0].id, { amount: 400 });

    expect(amountsOf(updated)).toEqual([400, 200, 200, 200]);
    expect(updated.purchase.totalPrice).toBe(1000);
    await expectConsistent(updated);
  });

  it("ignores the previous tranche entirely", async () => {
    // Requirement 4: the sequential gate is on the money, not the schedule.
    const detail = await freshPurchase();

    const updated = await api.updateInstallment(detail.installments[2].id, {
      amount: 400,
      dueDate: "2024-04-01",
    });

    expect(amountsOf(updated)).toEqual([250, 250, 400, 100]);
    expect(updated.installments[2].dueDate).toBe("2024-04-01");
    await expectConsistent(updated);
  });

  it("settles a tranche that is zeroed before anything is collected on it", async () => {
    const detail = await freshPurchase();

    const updated = await api.updateInstallment(detail.installments[0].id, { amount: 0 });

    expect(amountsOf(updated)).toEqual([0, 333, 333, 334]);
    expect(updated.installments[0].status).toBe("paid");
    expect(updated.installments[0].paidDate).toBeNull();
    await expectConsistent(updated);
  });

  it("locks both schedule fields once the tranche is paid", async () => {
    // Requirement 3.
    const detail = await freshPurchase();
    await settle(detail, 0);
    const id = detail.installments[0].id;

    expect(await failureOf(() => api.updateInstallment(id, { amount: 400 }))).toBe("AMOUNT_LOCKED");
    expect(await failureOf(() => api.updateInstallment(id, { dueDate: "2024-01-20" }))).toBe(
      "DUE_DATE_LOCKED",
    );

    // Resending the values it already has is not a change, so not a refusal.
    await api.updateInstallment(id, { amount: 250, dueDate: "2024-01-15" });
    expect(amountsOf(await api.getPurchaseDetail(detail.purchase.id))).toEqual([
      250, 250, 250, 250,
    ]);
  });

  it("keeps a due date between its neighbours", async () => {
    // This clamp is what makes position order and chronological order the same
    // thing, so "the previous installment" is unambiguous.
    const detail = await freshPurchase();
    const id = detail.installments[2].id; // between 2024-02-15 and 2024-04-15

    for (const outside of ["2024-02-01", "2024-05-01"]) {
      expect(await failureOf(() => api.updateInstallment(id, { dueDate: outside }))).toBe(
        "DUE_DATE_OUT_OF_ORDER",
      );
    }
    // The neighbours' own dates are inclusive bounds.
    for (const edge of ["2024-02-15", "2024-04-15"]) {
      await api.updateInstallment(id, { dueDate: edge });
    }
    // The outer tranches are unbounded on their missing side.
    await api.updateInstallment(detail.installments[0].id, { dueDate: "2020-01-01" });
    await api.updateInstallment(detail.installments[3].id, { dueDate: "2030-12-31" });
  });

  it("refuses when no other tranche can absorb the change", async () => {
    const detail = await freshPurchase();
    for (const pos of [0, 1, 2]) await settle(detail, pos);

    expect(
      await failureOf(() => api.updateInstallment(detail.installments[3].id, { amount: 100 })),
    ).toBe("NO_REBALANCE_ROOM");
    await expectConsistent(await api.getPurchaseDetail(detail.purchase.id));
  });
});

describe("the money half — editable only in payment order", () => {
  it("writes a correction entry when the collected figure goes up", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0, 100);

    const updated = await api.updateInstallment(detail.installments[0].id, {
      paidAmount: 250,
      paymentDate: "2024-03-05",
      note: "solde",
    });

    expect(updated.installments[0].paidAmount).toBe(250);
    expect(updated.installments[0].status).toBe("paid");
    expect(updated.installments[0].paidDate).toBe("2024-03-05");

    const payments = await api.listPaymentsForPurchase(detail.purchase.id);
    expect(payments).toHaveLength(2);
    // The entry carries the difference, not the new total.
    expect(payments.map((p) => p.amount).sort((a, b) => a - b)).toEqual([100, 150]);
    expect(payments.find((p) => p.amount === 150)?.note).toBe("solde");
    await expectConsistent(updated);
  });

  it("writes a negative correction when it goes down", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    const updated = await api.updateInstallment(detail.installments[0].id, { paidAmount: 80 });

    expect(updated.installments[0].paidAmount).toBe(80);
    expect(updated.installments[0].status).not.toBe("paid");
    // A row that owes again must not still display a settlement date.
    expect(updated.installments[0].paidDate).toBeNull();
    const payments = await api.listPaymentsForPurchase(detail.purchase.id);
    expect(payments.map((p) => p.amount).sort((a, b) => a - b)).toEqual([-170, 250]);
    await expectConsistent(updated);
  });

  it("reverses the whole ledger for a tranche set back to zero", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    const updated = await api.updateInstallment(detail.installments[0].id, { paidAmount: 0 });

    expect(updated.installments[0].paidAmount).toBe(0);
    expect(updated.totalPaid).toBe(0);
    await expectConsistent(updated);
  });

  it("is gated on the previous tranche", async () => {
    // Requirement 4: cash cannot be recorded out of order.
    const detail = await freshPurchase();

    expect(
      await failureOf(() => api.updateInstallment(detail.installments[1].id, { paidAmount: 100 })),
    ).toBe("PREVIOUS_UNPAID:1");
    expect(await api.listPaymentsForPurchase(detail.purchase.id)).toHaveLength(0);

    await settle(detail, 0);
    const updated = await api.updateInstallment(detail.installments[1].id, { paidAmount: 100 });
    expect(updated.installments[1].paidAmount).toBe(100);
    await expectConsistent(updated);
  });

  it("never lets a tranche collect more than it is worth", async () => {
    const detail = await freshPurchase();

    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { paidAmount: 400 })),
    ).toBe("PAID_ABOVE_AMOUNT:250");
    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { paidAmount: -1 })),
    ).toBe("INVALID_AMOUNT");
  });

  it("reports the same constraint against whichever field moved", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0, 100);

    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { amount: 50 })),
    ).toBe("BELOW_PAID:100");

    // Lowering both together resolves the conflict, and must be accepted.
    const updated = await api.updateInstallment(detail.installments[0].id, {
      amount: 50,
      paidAmount: 50,
    });
    expect(updated.installments[0].amount).toBe(50);
    expect(updated.installments[0].paidAmount).toBe(50);
    await expectConsistent(updated);
  });
});

describe("the payment date", () => {
  it("re-dates the latest ledger entry when nothing else moved", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    const updated = await api.updateInstallment(detail.installments[0].id, {
      paymentDate: "2024-03-05",
    });

    expect(updated.installments[0].paidDate).toBe("2024-03-05");
    const payments = await api.listPaymentsForPurchase(detail.purchase.id);
    expect(payments).toHaveLength(1);
    expect(payments[0].paymentDate).toBe("2024-03-05");
    await expectConsistent(updated);
  });

  it("lets a note alone amend the entry already there", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    await api.updateInstallment(detail.installments[0].id, { note: "chèque" });

    const payments = await api.listPaymentsForPurchase(detail.purchase.id);
    expect(payments).toHaveLength(1);
    expect(payments[0].note).toBe("chèque");
  });

  it("refuses a note with no payment behind it rather than dropping it", async () => {
    const detail = await freshPurchase();
    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { note: "x" })),
    ).toBe("NO_PAYMENT_TO_DATE");
  });

  it("needs something to date, and cannot be in the future", async () => {
    const detail = await freshPurchase();
    const id = detail.installments[0].id;

    expect(await failureOf(() => api.updateInstallment(id, { paymentDate: "2024-03-05" }))).toBe(
      "NO_PAYMENT_TO_DATE",
    );

    await settle(detail, 0);
    const tomorrow = new Date(Date.now() + 86_400_000).toISOString().slice(0, 10);
    expect(await failureOf(() => api.updateInstallment(id, { paymentDate: tomorrow }))).toBe(
      "FUTURE_PAID_DATE",
    );
  });
});

describe("guards shared with the rest of the purchase surface", () => {
  it("refuses an edit on an archived purchase", async () => {
    const detail = await freshPurchase();
    await api.archivePurchase(detail.purchase.id);

    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { amount: 300 })),
    ).toBe("PURCHASE_ARCHIVED");
  });

  it("refuses an unknown installment and a malformed date without writing", async () => {
    const detail = await freshPurchase();

    expect(await failureOf(() => api.updateInstallment(999_999, { amount: 300 }))).toBe(
      "INSTALLMENT_NOT_FOUND",
    );
    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { dueDate: "31/12" })),
    ).toBe("INVALID_DATE");
    expect(amountsOf(await api.getPurchaseDetail(detail.purchase.id))).toEqual([
      250, 250, 250, 250,
    ]);
  });

  it("leaves everything alone when one half of a combined edit is refused", async () => {
    const detail = await freshPurchase();

    // The amount alone would be fine; the money half is gated on tranche 1.
    expect(
      await failureOf(() =>
        api.updateInstallment(detail.installments[1].id, { amount: 150, paidAmount: 50 }),
      ),
    ).toBe("PREVIOUS_UNPAID:1");

    const after = await api.getPurchaseDetail(detail.purchase.id);
    expect(amountsOf(after)).toEqual([250, 250, 250, 250]);
    await expectConsistent(after);
  });
});

describe("the rest of the app follows an edit", () => {
  it("keeps the outstanding total unchanged by a pure rebalance", async () => {
    const detail = await freshPurchase();
    const before = (await api.getDashboard()).stats.totalOutstanding;

    await api.updateInstallment(detail.installments[0].id, { amount: 400 });

    // Money moved between tranches, not into or out of the books.
    expect((await api.getDashboard()).stats.totalOutstanding).toBe(before);
  });

  it("moves the dashboard's collected total with a paid-amount edit", async () => {
    const detail = await freshPurchase();
    const before = (await api.getDashboard()).stats.totalCollected;

    await api.updateInstallment(detail.installments[0].id, { paidAmount: 150 });
    expect((await api.getDashboard()).stats.totalCollected).toBe(before + 150);

    await api.updateInstallment(detail.installments[0].id, { paidAmount: 40 });
    expect((await api.getDashboard()).stats.totalCollected).toBe(before + 40);
  });

  it("shows the edited amount and remaining on the schedule", async () => {
    const detail = await freshPurchase();
    await api.updateInstallment(detail.installments[0].id, {
      amount: 400,
      dueDate: "2020-06-30",
    });

    const row = (await api.listSchedule()).find(
      (r) => r.installmentId === detail.installments[0].id,
    );
    expect(row?.amount).toBe(400);
    expect(row?.dueDate).toBe("2020-06-30");
    expect(row?.remaining).toBe(400);
  });
});
