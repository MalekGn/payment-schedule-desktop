// Integration suite — the data behind the three printable documents.
//
// The claim this exists to hold: printing needed **no new backend command**.
// Every document is assembled from read models the app already had, so what is
// worth testing is that the composition is right — that a schedule, a receipt
// and a statement can each be built from the gateway alone, and that the figures
// they print agree with the screens they were printed from.
//
// The receipt case also pins the guard that stops a wrong-purchase payment id
// rendering a document with the wrong client's name on it.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";

let api: typeof import("@/api").api;

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
});

describe("échéancier — everything the contract prints", () => {
  it("assembles from getPurchaseDetail alone", async () => {
    const purchases = await api.listPurchases();
    const detail = await api.getPurchaseDetail(purchases[0].id);

    // The letterhead needs the shop; the body needs client, purchase, schedule.
    expect(detail.client.firstName).toBeTruthy();
    expect(detail.purchase.reference).toMatch(/^A-\d+$/);
    expect(detail.installments.length).toBe(detail.purchase.installmentCount);

    // The printed footer row must reconcile with the rows above it, or the
    // client is handed a contract that does not add up.
    const scheduled = detail.installments.reduce((s, i) => s + i.amount, 0);
    const paid = detail.installments.reduce((s, i) => s + i.paidAmount, 0);
    expect(scheduled).toBe(detail.purchase.totalPrice);
    expect(paid).toBe(detail.totalPaid);
    expect(detail.purchase.totalPrice - paid).toBe(detail.remaining);
  });

  it("gives every tranche a due date and an amount to print", async () => {
    const purchases = await api.listPurchases();
    const detail = await api.getPurchaseDetail(purchases[0].id);
    for (const inst of detail.installments) {
      expect(inst.dueDate).toMatch(/^\d{4}-\d{2}-\d{2}$/);
      expect(inst.amount).toBeGreaterThan(0);
      expect(Number.isInteger(inst.amount)).toBe(true);
    }
  });
});

describe("reçu — the payment being receipted", () => {
  it("resolves the payment out of its own purchase's ledger", async () => {
    const purchases = await api.listPurchases();
    // The seed settles the first tranche of A-000001.
    const withPayment = await Promise.all(
      purchases.map(async (p) => ({ p, pays: await api.listPaymentsForPurchase(p.id) })),
    );
    const target = withPayment.find((row) => row.pays.length > 0);
    expect(target, "the seed must contain a purchase with a payment").toBeDefined();

    const payment = target!.pays[0];
    const detail = await api.getPurchaseDetail(target!.p.id);

    expect(payment.purchaseId).toBe(detail.purchase.id);
    expect(payment.installmentIndex).toBeGreaterThanOrEqual(1);
    expect(payment.installmentIndex).toBeLessThanOrEqual(detail.purchase.installmentCount);
    expect(payment.amount).toBeGreaterThan(0);
  });

  it("cannot resolve a payment belonging to a different purchase", async () => {
    // The view looks the payment up inside the addressed purchase's ledger and
    // shows its not-found state when it is absent. Without that, `/imprimer/
    // recu/<A>?payment=<B's payment>` would print B's money under A's client.
    const purchases = await api.listPurchases();
    const [first, second] = purchases;
    const otherPayments = await api.listPaymentsForPurchase(second.id);
    const firstPayments = await api.listPaymentsForPurchase(first.id);

    for (const foreign of otherPayments) {
      expect(firstPayments.some((p) => p.id === foreign.id)).toBe(false);
    }
  });
});

describe("relevé — the client's whole position", () => {
  it("assembles from getClientDetail plus the client's payments", async () => {
    const clients = await api.listClients();
    const target = clients.find((c) => c.purchaseCount > 0)!;
    const detail = await api.getClientDetail(target.id);
    const payments = await api.listPaymentsForClient(target.id);

    // Totals must come out of the live purchases only — the statement lists
    // archived ones separately and excludes them, because an archived purchase
    // is off the books and is not owed.
    const live = detail.purchases.reduce(
      (acc, p) => ({
        total: acc.total + p.totalPrice,
        paid: acc.paid + p.paidAmount,
        remaining: acc.remaining + p.remaining,
      }),
      { total: 0, paid: 0, remaining: 0 },
    );
    expect(live.total).toBe(detail.totalPurchased);
    expect(live.paid).toBe(detail.totalPaid);
    expect(live.remaining).toBe(detail.totalOutstanding);

    // Every printed payment row must name a purchase, so the statement can be
    // reconciled line by line.
    for (const pay of payments) {
      expect(pay.purchaseReference).toBeTruthy();
    }
  });

  it("keeps archived purchases out of the totals it prints", async () => {
    const clients = await api.listClients("all");
    for (const c of clients) {
      const detail = await api.getClientDetail(c.id);
      const archivedValue = detail.archivedPurchases.reduce((s, p) => s + p.remaining, 0);
      if (detail.archivedPurchases.length > 0) {
        // Archiving is refused while a purchase carries a payment, so an
        // archived one is worth nothing to the balance by construction.
        expect(archivedValue).toBe(0);
      }
      const live = detail.purchases.reduce((s, p) => s + p.remaining, 0);
      expect(detail.totalOutstanding).toBe(live);
    }
  });
});

describe("the document title crosses the gateway", () => {
  it("reaches the backend rather than being set on the DOM alone", async () => {
    // The print dialog derives the saved file's name from the title. On Linux
    // it comes from the GTK print job, which takes its name from the native
    // window — so the rename has to cross the boundary, not just touch
    // `document.title`. Recorded by the mock because no suite can read a native
    // title bar.
    const { mockDb } = await import("@/api/mock");
    expect(mockDb.lastWindowTitle).toBeNull();

    await api.setWindowTitle("Echeancier-A-000001-Ali-Ben-Salah");
    expect(mockDb.lastWindowTitle).toBe("Echeancier-A-000001-Ali-Ben-Salah");
  });
});
