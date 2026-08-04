// Integration suite — the two installment editors, and the line between them.
//
// Editing is split by *which fields* it may touch, not by how much has been
// paid:
//
//   * `updatePurchase` is the only writer of `amount` and `dueDate`. It applies
//     a whole resolved schedule onto the stored rows position by position, so a
//     purchase carrying payments can still be rescheduled — the rows survive,
//     and the `payment` ledger hanging off them with it.
//   * `updateInstallment` deals only in money. An `amount` or `dueDate` sent
//     there is refused outright, which is what makes "the schedule is edited in
//     one place" a property of the backend rather than a habit of the UI.
//
// The three product rules asserted here:
//
//   1. A settled installment's amount and due date are immutable from anywhere.
//      Its collected figure stays editable.
//   2. A recorded payment date is immutable. Setting one the first time is not.
//   3. An unsettled installment's amount and due date move only through the
//      purchase editor.
//
// The money invariants the rest of the app leans on have to survive all of it:
//
//   * `SUM(amount) === purchase.totalPrice`;
//   * `paidAmount <= amount` on every row, which keeps the outstanding and
//     overdue aggregates from going negative;
//   * `SUM(payments) === SUM(paidAmount)` — the dashboard's "Amount collected"
//     is the only money figure read from the ledger, so a paid-amount edit that
//     skipped the ledger would make that tile contradict every other total.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstallmentInput, PurchaseDetail } from "@/types/models";

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
const datesOf = (d: PurchaseDetail): string[] => d.installments.map((i) => i.dueDate);

/**
 * Save `rows` as the purchase's schedule — the only route an amount or a due
 * date can travel. The total follows the rows, since the two have to agree.
 */
async function reschedule(
  detail: PurchaseDetail,
  rows: [amount: number, dueDate: string][],
): Promise<PurchaseDetail> {
  const installments: InstallmentInput[] = rows.map(([amount, dueDate], i) => ({
    index: i + 1,
    amount,
    dueDate,
  }));
  return api.updatePurchase(detail.purchase.id, {
    clientId: detail.purchase.clientId,
    productLabel: detail.purchase.productLabel,
    totalPrice: rows.reduce((s, [amount]) => s + amount, 0),
    installmentCount: rows.length,
    intervalKind: detail.purchase.intervalKind,
    intervalDays: detail.purchase.intervalDays,
    purchaseDate: detail.purchase.purchaseDate,
    installments,
  });
}

/** The stored schedule as `reschedule` wants it, for edits that keep most rows. */
const rowsOf = (d: PurchaseDetail): [number, string][] =>
  d.installments.map((i) => [i.amount, i.dueDate]);

/** Assert every invariant that must survive an edit, ledger included. */
async function expectConsistent(detail: PurchaseDetail): Promise<void> {
  expect(amountsOf(detail).reduce((s, v) => s + v, 0)).toBe(detail.purchase.totalPrice);
  for (const inst of detail.installments) {
    expect(inst.paidAmount).toBeLessThanOrEqual(inst.amount);
    expect(Number.isInteger(inst.amount)).toBe(true);
  }
  // Due dates run in position order — the sequential money rule is stated in
  // terms of "the previous installment" and means both readings at once.
  const dates = datesOf(detail);
  expect([...dates].sort()).toEqual(dates);
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

describe("rule 3 — the schedule is not the installment editor's to touch", () => {
  it("refuses an amount or a due date whatever the tranche's state", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    // Settled (tranche 1) and unsettled (tranche 3) alike.
    for (const pos of [0, 2]) {
      const id = detail.installments[pos].id;
      expect(await failureOf(() => api.updateInstallment(id, { amount: 400 }))).toBe(
        "SCHEDULE_VIA_PURCHASE",
      );
      expect(await failureOf(() => api.updateInstallment(id, { dueDate: "2024-06-01" }))).toBe(
        "SCHEDULE_VIA_PURCHASE",
      );
    }

    // Even a value identical to what is stored: sending the field at all is a
    // caller that still believes this command owns it.
    expect(
      await failureOf(() => api.updateInstallment(detail.installments[2].id, { amount: 250 })),
    ).toBe("SCHEDULE_VIA_PURCHASE");

    expect(amountsOf(await api.getPurchaseDetail(detail.purchase.id))).toEqual([
      250, 250, 250, 250,
    ]);
  });

  it("refuses before it has even looked the installment up", async () => {
    // The unknown id would otherwise be INSTALLMENT_NOT_FOUND, so the schedule
    // guard demonstrably runs first — and never reaches the store.
    expect(await failureOf(() => api.updateInstallment(999_999, { amount: 1 }))).toBe(
      "SCHEDULE_VIA_PURCHASE",
    );
  });

  it("moves an unsettled tranche's amount and due date through the purchase", async () => {
    const detail = await freshPurchase();

    const updated = await reschedule(detail, [
      [250, "2024-01-15"],
      [400, "2024-03-01"],
      [200, "2024-04-01"],
      [150, "2024-05-01"],
    ]);

    expect(amountsOf(updated)).toEqual([250, 400, 200, 150]);
    expect(datesOf(updated)).toEqual(["2024-01-15", "2024-03-01", "2024-04-01", "2024-05-01"]);
    expect(updated.purchase.totalPrice).toBe(1000);
    await expectConsistent(updated);

    // And it persists: re-read rather than trusting the returned detail.
    expect(amountsOf(await api.getPurchaseDetail(detail.purchase.id))).toEqual([
      250, 400, 200, 150,
    ]);
  });

  it("still moves the unpaid tranches once a payment exists", async () => {
    // The whole point of applying in place: the old editor refused outright
    // here, which left the unpaid tranches frozen the moment a sibling was paid.
    const detail = await freshPurchase();
    await settle(detail, 0);
    const ids = detail.installments.map((i) => i.id);

    const updated = await reschedule(detail, [
      [250, "2024-01-15"],
      [400, "2024-03-01"],
      [200, "2024-04-01"],
      [150, "2024-05-01"],
    ]);

    expect(amountsOf(updated)).toEqual([250, 400, 200, 150]);
    // In place, not regenerated — which is what kept the payment attached.
    expect(updated.installments.map((i) => i.id)).toEqual(ids);
    expect(updated.totalPaid).toBe(250);
    expect(await api.listPaymentsForPurchase(detail.purchase.id)).toHaveLength(1);
    await expectConsistent(updated);
  });
});

describe("rule 1 — a settled tranche's schedule is history", () => {
  it("refuses to move its amount or its due date from the purchase editor", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);
    const rows = rowsOf(detail);

    expect(await failureOf(() => reschedule(detail, [[400, "2024-01-15"], ...rows.slice(1)]))).toBe(
      "AMOUNT_LOCKED",
    );
    expect(await failureOf(() => reschedule(detail, [[250, "2024-01-20"], ...rows.slice(1)]))).toBe(
      "DUE_DATE_LOCKED",
    );

    const after = await api.getPurchaseDetail(detail.purchase.id);
    expect(amountsOf(after)).toEqual([250, 250, 250, 250]);
    expect(after.installments[0].dueDate).toBe("2024-01-15");
    await expectConsistent(after);
  });

  it("leaves the purchase row alone when the schedule is refused", async () => {
    // The purchase row is written before the schedule, so a late refusal has to
    // take it back with it.
    const detail = await freshPurchase();
    await settle(detail, 0);

    await failureOf(() =>
      api.updatePurchase(detail.purchase.id, {
        clientId: detail.purchase.clientId,
        productLabel: "Congélateur",
        totalPrice: 2000,
        installmentCount: 4,
        intervalKind: "monthly",
        intervalDays: null,
        purchaseDate: "2024-01-15",
      }),
    );

    const after = await api.getPurchaseDetail(detail.purchase.id);
    expect(after.purchase.totalPrice).toBe(1000);
    expect(after.purchase.productLabel).toBe("Réfrigérateur");
  });

  it("keeps its collected figure editable — only the schedule froze", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    const updated = await api.updateInstallment(detail.installments[0].id, { paidAmount: 180 });

    expect(updated.installments[0].paidAmount).toBe(180);
    expect(updated.installments[0].amount).toBe(250);
    // No longer settled, so the derived date goes with it.
    expect(updated.installments[0].paidDate).toBeNull();
    await expectConsistent(updated);
  });

  it("still allows a partially paid tranche to be rescheduled, down to what it collected", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0, 100);

    const updated = await reschedule(detail, [
      [150, "2024-01-15"],
      [300, "2024-02-15"],
      [300, "2024-03-15"],
      [250, "2024-04-15"],
    ]);
    expect(amountsOf(updated)).toEqual([150, 300, 300, 250]);
    await expectConsistent(updated);

    // But never below it: `amount - paidAmount` feeds every outstanding total.
    expect(
      await failureOf(() =>
        reschedule(updated, [
          [50, "2024-01-15"],
          [350, "2024-02-15"],
          [350, "2024-03-15"],
          [250, "2024-04-15"],
        ]),
      ),
    ).toBe("BELOW_PAID:100");
  });

  it("settles a tranche rescheduled onto its collected figure, date and all", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0, 100);

    const updated = await reschedule(detail, [
      [100, "2024-01-15"],
      [300, "2024-02-15"],
      [300, "2024-03-15"],
      [300, "2024-04-15"],
    ]);

    expect(updated.installments[0].status).toBe("paid");
    // Derived from the ledger, not invented.
    expect(updated.installments[0].paidDate).toBe("2024-02-01");
    await expectConsistent(updated);
  });
});

describe("rule 2 — a recorded payment date is history", () => {
  it("refuses to re-date an entry already on record", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    expect(
      await failureOf(() =>
        api.updateInstallment(detail.installments[0].id, { paymentDate: "2024-03-05" }),
      ),
    ).toBe("PAYMENT_DATE_LOCKED");

    const payments = await api.listPaymentsForPurchase(detail.purchase.id);
    expect(payments).toHaveLength(1);
    expect(payments[0].paymentDate).toBe("2024-02-01");
    const after = await api.getPurchaseDetail(detail.purchase.id);
    expect(after.installments[0].paidDate).toBe("2024-02-01");
  });

  it("still dates the entry it arrives with — setting one is not changing one", async () => {
    const detail = await freshPurchase();

    const updated = await api.updateInstallment(detail.installments[0].id, {
      paidAmount: 250,
      paymentDate: "2024-03-05",
    });
    expect(updated.installments[0].paidDate).toBe("2024-03-05");

    // A later correction dates its own entry without touching the first: the
    // ledger accumulates rather than being rewritten.
    const corrected = await api.updateInstallment(detail.installments[0].id, {
      paidAmount: 200,
      paymentDate: "2024-04-01",
    });
    expect(corrected.installments[0].paidAmount).toBe(200);
    const payments = await api.listPaymentsForPurchase(detail.purchase.id);
    expect(payments).toHaveLength(2);
    expect(payments.map((p) => p.paymentDate).sort()).toEqual(["2024-03-05", "2024-04-01"]);
    await expectConsistent(corrected);
  });

  it("refuses a date with no entry to carry it rather than dropping it", async () => {
    const detail = await freshPurchase();
    expect(
      await failureOf(() =>
        api.updateInstallment(detail.installments[0].id, { paymentDate: "2024-03-05" }),
      ),
    ).toBe("NO_PAYMENT_TO_DATE");
  });

  it("cannot be in the future", async () => {
    const detail = await freshPurchase();
    const tomorrow = new Date(Date.now() + 86_400_000).toISOString().slice(0, 10);
    expect(
      await failureOf(() =>
        api.updateInstallment(detail.installments[0].id, {
          paidAmount: 250,
          paymentDate: tomorrow,
        }),
      ),
    ).toBe("FUTURE_PAID_DATE");
  });

  it("lets a note alone amend the entry already there — only the date is frozen", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);

    await api.updateInstallment(detail.installments[0].id, { note: "chèque" });

    const payments = await api.listPaymentsForPurchase(detail.purchase.id);
    expect(payments).toHaveLength(1);
    expect(payments[0].note).toBe("chèque");
    expect(payments[0].paymentDate).toBe("2024-02-01");
  });

  it("refuses a note with no payment behind it rather than dropping it", async () => {
    const detail = await freshPurchase();
    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { note: "x" })),
    ).toBe("NO_PAYMENT_TO_DATE");
  });
});

describe("the money — editable only in payment order", () => {
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

  it("is gated on the previous tranche", async () => {
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

  it("leaves the ledger alone when a combined edit is refused", async () => {
    const detail = await freshPurchase();

    expect(
      await failureOf(() =>
        api.updateInstallment(detail.installments[1].id, { paidAmount: 50, note: "acompte" }),
      ),
    ).toBe("PREVIOUS_UNPAID:1");

    expect(await api.listPaymentsForPurchase(detail.purchase.id)).toHaveLength(0);
    await expectConsistent(await api.getPurchaseDetail(detail.purchase.id));
  });
});

describe("changing the length of the schedule", () => {
  it("drops tranches nobody has paid into", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);
    const firstId = detail.installments[0].id;

    const updated = await reschedule(detail, [
      [250, "2024-01-15"],
      [250, "2024-02-15"],
    ]);

    expect(amountsOf(updated)).toEqual([250, 250]);
    expect(updated.installments[0].id).toBe(firstId);
    expect(updated.purchase.totalPrice).toBe(500);
    expect(updated.totalPaid).toBe(250);
    await expectConsistent(updated);
  });

  it("refuses to drop one that carries cash", async () => {
    const detail = await freshPurchase();
    // Cash is recorded in order, so every earlier tranche settles first.
    for (const pos of [0, 1, 2, 3]) await settle(detail, pos);

    expect(
      await failureOf(() =>
        reschedule(detail, [
          [250, "2024-01-15"],
          [250, "2024-02-15"],
          [250, "2024-03-15"],
        ]),
      ),
    ).toBe("PURCHASE_HAS_PAYMENTS:1");
    expect((await api.getPurchaseDetail(detail.purchase.id)).installments).toHaveLength(4);
  });

  it("refuses to drop one corrected back down to zero — the entries are still there", async () => {
    const detail = await freshPurchase();
    for (const pos of [0, 1, 2, 3]) await settle(detail, pos);
    await api.updateInstallment(detail.installments[3].id, { paidAmount: 0 });

    const after = await api.getPurchaseDetail(detail.purchase.id);
    expect(after.installments[3].paidAmount).toBe(0);
    expect(await api.listPaymentsForPurchase(detail.purchase.id)).toHaveLength(5);

    // The collected figure is zero, but erasing the row would take the entry
    // that took the money and the one that gave it back with it.
    expect(
      await failureOf(() =>
        reschedule(detail, [
          [250, "2024-01-15"],
          [250, "2024-02-15"],
          [250, "2024-03-15"],
        ]),
      ),
    ).toBe("PURCHASE_HAS_PAYMENTS:1");
    expect(await api.listPaymentsForPurchase(detail.purchase.id)).toHaveLength(5);
  });

  it("appends new tranches past the ones already stored", async () => {
    const detail = await freshPurchase();
    await settle(detail, 0);
    const ids = detail.installments.map((i) => i.id);

    const updated = await reschedule(detail, [
      [250, "2024-01-15"],
      [200, "2024-02-15"],
      [200, "2024-03-15"],
      [200, "2024-04-15"],
      [150, "2024-05-15"],
    ]);

    expect(amountsOf(updated)).toEqual([250, 200, 200, 200, 150]);
    expect(updated.installments.slice(0, 4).map((i) => i.id)).toEqual(ids);
    expect(updated.installments[4].index).toBe(5);
    expect(updated.totalPaid).toBe(250);
    await expectConsistent(updated);
  });

  it("refuses a schedule whose dates run backwards, on create as on update", async () => {
    const detail = await freshPurchase();

    expect(
      await failureOf(() =>
        reschedule(detail, [
          [500, "2024-03-15"],
          [500, "2024-02-15"],
        ]),
      ),
    ).toBe("DUE_DATE_OUT_OF_ORDER");
    expect((await api.getPurchaseDetail(detail.purchase.id)).installments).toHaveLength(4);

    const client = (await api.listClients())[0];
    expect(
      await failureOf(() =>
        api.createPurchase({
          clientId: client.id,
          productLabel: "Four",
          totalPrice: 1000,
          installmentCount: 2,
          intervalKind: "monthly",
          intervalDays: null,
          purchaseDate: "2024-01-15",
          installments: [
            { index: 1, amount: 500, dueDate: "2024-03-15" },
            { index: 2, amount: 500, dueDate: "2024-02-15" },
          ],
        }),
      ),
    ).toBe("DUE_DATE_OUT_OF_ORDER");
  });
});

describe("guards shared with the rest of the purchase surface", () => {
  it("refuses an edit on an archived purchase", async () => {
    const detail = await freshPurchase();
    await api.archivePurchase(detail.purchase.id);

    expect(
      await failureOf(() => api.updateInstallment(detail.installments[0].id, { paidAmount: 250 })),
    ).toBe("PURCHASE_ARCHIVED");
  });

  it("refuses an unknown installment and a malformed date without writing", async () => {
    const detail = await freshPurchase();

    expect(await failureOf(() => api.updateInstallment(999_999, { paidAmount: 300 }))).toBe(
      "INSTALLMENT_NOT_FOUND",
    );
    expect(
      await failureOf(() =>
        api.updateInstallment(detail.installments[0].id, {
          paidAmount: 100,
          paymentDate: "31/12",
        }),
      ),
    ).toBe("INVALID_DATE");
    expect(await api.listPaymentsForPurchase(detail.purchase.id)).toHaveLength(0);
  });
});

describe("the rest of the app follows an edit", () => {
  it("keeps the outstanding total unchanged by a pure reshuffle of the tranches", async () => {
    const detail = await freshPurchase();
    const before = (await api.getDashboard()).stats.totalOutstanding;

    await reschedule(detail, [
      [400, "2024-01-15"],
      [200, "2024-02-15"],
      [200, "2024-03-15"],
      [200, "2024-04-15"],
    ]);

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

  it("shows the rescheduled amount and date on the échéances list", async () => {
    const detail = await freshPurchase();
    await reschedule(detail, [
      [400, "2020-06-30"],
      [200, "2024-02-15"],
      [200, "2024-03-15"],
      [200, "2024-04-15"],
    ]);

    const row = (await api.listSchedule()).find(
      (r) => r.installmentId === detail.installments[0].id,
    );
    expect(row?.amount).toBe(400);
    expect(row?.dueDate).toBe("2020-06-30");
    expect(row?.remaining).toBe(400);
  });
});
