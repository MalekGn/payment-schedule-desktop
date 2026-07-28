// Integration suite — editing and archiving purchases.
//
// The mirror image of `client-archive`, and the opposite assertion. An archived
// *client* is settled, so every money aggregate is unchanged by archiving them.
// An archived *purchase* must LEAVE the books: a removed purchase is no longer
// owed, sold, or due. These tests exist to pin that down across all of the
// dashboard, impayés, échéances and the client's own totals at once — the filter
// sweep touched nine queries and missing one leaves a headline number silently
// disagreeing with the list it links to.
//
// The second property under test is the zero-payments invariant that lets
// `totalCollected` skip the filter entirely: archiving is refused once a payment
// exists, and an archived purchase cannot take one.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";

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
 * a rejection surfaces as a synchronous throw. Awaiting inside a `try` reads
 * both backends identically.
 */
async function failureOf(call: () => Promise<unknown>): Promise<string> {
  try {
    await call();
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
  throw new Error("expected the call to fail, but it resolved");
}

/** Every headline figure the archive has to move, captured together. */
async function moneySnapshot() {
  const [dash, impayes, schedule, purchases] = await Promise.all([
    api.getDashboard(),
    api.listImpayes(),
    api.listSchedule(),
    api.listPurchases(),
  ]);
  return {
    totalPurchases: dash.stats.totalPurchases,
    totalSales: dash.stats.totalSales,
    totalCollected: dash.stats.totalCollected,
    totalOutstanding: dash.stats.totalOutstanding,
    overdueCount: dash.stats.overdueCount,
    overdueClients: dash.stats.overdueClients,
    upcomingCount: dash.stats.upcomingCount,
    impayeRows: impayes.reduce((s, c) => s + c.installments.length, 0),
    scheduleRows: schedule.length,
    listedPurchases: purchases.length,
  };
}

/** A seeded purchase with nothing paid on it — the archivable kind. */
async function unpaidPurchase() {
  const p = (await api.listPurchases()).find((x) => x.paidAmount === 0);
  expect(p, "expected a seeded purchase with no payments").toBeDefined();
  return p!;
}

/** A seeded purchase that has collected cash — the permanent kind. */
async function paidPurchase() {
  const p = (await api.listPurchases()).find((x) => x.paidAmount > 0);
  expect(p, "expected a seeded purchase with a payment").toBeDefined();
  return p!;
}

describe("archiving is gated on the purchase having collected nothing", () => {
  it("refuses to archive a purchase that has payments", async () => {
    const paid = await paidPurchase();

    expect(await failureOf(() => api.archivePurchase(paid.id))).toMatch(
      /^PURCHASE_HAS_PAYMENTS:\d+$/,
    );

    const after = (await api.listPurchases()).find((p) => p.id === paid.id);
    expect(after?.archivedAt).toBeNull();
  });

  it("makes a paid purchase permanent — neither archivable nor deletable", async () => {
    // The deliberate consequence: once real cash is recorded against a
    // purchase, it stays on the books. Asserted so a later change cannot
    // quietly open either door.
    const paid = await paidPurchase();

    expect(await failureOf(() => api.archivePurchase(paid.id))).toMatch(/^PURCHASE_HAS_PAYMENTS:/);
    expect(await failureOf(() => api.deletePurchase(paid.id))).toBe("PURCHASE_NOT_ARCHIVED");
  });

  it("refuses a payment on an archived purchase", async () => {
    // The other half of the invariant: an archived purchase can never start
    // collecting, so `totalCollected` never needs an archive filter.
    const target = await unpaidPurchase();
    const detail = await api.getPurchaseDetail(target.id);
    await api.archivePurchase(target.id);

    expect(
      await failureOf(() =>
        api.recordPayment({
          installmentId: detail.installments[0].id,
          amount: 100,
          paymentDate: detail.purchase.purchaseDate,
          note: null,
        }),
      ),
    ).toBe("PURCHASE_ARCHIVED");
  });
});

describe("an archived purchase leaves every money view", () => {
  it("removes exactly its own contribution, and restore puts it back", async () => {
    const target = await unpaidPurchase();
    const before = await moneySnapshot();

    await api.archivePurchase(target.id);
    const after = await moneySnapshot();

    expect(after.totalPurchases).toBe(before.totalPurchases - 1);
    expect(after.totalSales).toBe(before.totalSales - target.totalPrice);
    expect(after.totalOutstanding).toBe(before.totalOutstanding - target.remaining);
    expect(after.listedPurchases).toBe(before.listedPurchases - 1);
    expect(after.scheduleRows).toBe(before.scheduleRows - target.installmentCount);
    expect(after.impayeRows).toBe(before.impayeRows - target.overdueCount);
    // Nothing was collected on it, so this one figure must not move at all.
    expect(after.totalCollected).toBe(before.totalCollected);

    await api.restorePurchase(target.id);
    expect(await moneySnapshot()).toEqual(before);
  });

  it("drops out of its client's totals and moves to the archived section", async () => {
    const target = await unpaidPurchase();
    const before = await api.getClientDetail(target.clientId);

    await api.archivePurchase(target.id);
    const after = await api.getClientDetail(target.clientId);

    expect(after.purchases.length).toBe(before.purchases.length - 1);
    expect(after.archivedPurchases.map((p) => p.id)).toContain(target.id);
    expect(after.totalPurchased).toBe(before.totalPurchased - target.totalPrice);
    expect(after.totalOutstanding).toBe(before.totalOutstanding - target.remaining);
  });

  it("stops counting towards its client's outstanding for the client archive gate", async () => {
    const target = await unpaidPurchase();
    const before = (await api.listClients()).find((c) => c.id === target.clientId)!;

    await api.archivePurchase(target.id);
    const after = (await api.listClients()).find((c) => c.id === target.clientId)!;

    expect(after.purchaseCount).toBe(before.purchaseCount - 1);
    expect(after.totalOutstanding).toBe(before.totalOutstanding - target.remaining);
  });

  it("keeps its own detail page reachable", async () => {
    const target = await unpaidPurchase();
    await api.archivePurchase(target.id);

    const detail = await api.getPurchaseDetail(target.id);
    expect(detail.purchase.archivedAt).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(detail.installments.length).toBe(target.installmentCount);
  });
});

describe("the scope filter partitions the purchase list", () => {
  it("moves the purchase between active and archived, and all is the union", async () => {
    const target = await unpaidPurchase();
    const allBefore = await api.listPurchases("all");

    await api.archivePurchase(target.id);

    const active = await api.listPurchases("active");
    const archived = await api.listPurchases("archived");
    const all = await api.listPurchases("all");

    expect(active.some((p) => p.id === target.id)).toBe(false);
    expect(archived.some((p) => p.id === target.id)).toBe(true);
    expect(all.length).toBe(allBefore.length);
    expect(active.length + archived.length).toBe(all.length);
  });

  it("defaults to active when no scope is given", async () => {
    const target = await unpaidPurchase();
    await api.archivePurchase(target.id);

    const defaulted = await api.listPurchases();
    const explicit = await api.listPurchases("active");
    expect(defaulted.map((p) => p.id)).toEqual(explicit.map((p) => p.id));
  });
});

describe("permanent delete is the second half of a two-step", () => {
  it("refuses a purchase that has not been archived", async () => {
    const target = await unpaidPurchase();
    expect(await failureOf(() => api.deletePurchase(target.id))).toBe("PURCHASE_NOT_ARCHIVED");
    expect((await api.listPurchases()).some((p) => p.id === target.id)).toBe(true);
  });

  it("destroys an archived purchase and its installments", async () => {
    const target = await unpaidPurchase();
    await api.archivePurchase(target.id);

    await api.deletePurchase(target.id);

    expect((await api.listPurchases("all")).some((p) => p.id === target.id)).toBe(false);
    expect(await failureOf(() => api.getPurchaseDetail(target.id))).toBe("PURCHASE_NOT_FOUND");
    // The client survives.
    expect((await api.listClients()).some((c) => c.id === target.clientId)).toBe(true);
  });

  it("reports a missing id rather than succeeding silently", async () => {
    expect(await failureOf(() => api.deletePurchase(999_999))).toBe("PURCHASE_NOT_FOUND");
    expect(await failureOf(() => api.archivePurchase(999_999))).toBe("PURCHASE_NOT_FOUND");
    expect(await failureOf(() => api.restorePurchase(999_999))).toBe("PURCHASE_NOT_FOUND");
  });
});

describe("editing a purchase", () => {
  /** Build an update payload from a stored detail, with overrides applied. */
  async function payloadFor(id: number, over: Record<string, unknown> = {}) {
    const d = await api.getPurchaseDetail(id);
    return {
      clientId: d.purchase.clientId,
      productLabel: d.purchase.productLabel,
      totalPrice: d.purchase.totalPrice,
      installmentCount: d.purchase.installmentCount,
      intervalKind: d.purchase.intervalKind,
      intervalDays: d.purchase.intervalDays,
      purchaseDate: d.purchase.purchaseDate,
      installments: d.installments.map((i) => ({
        index: i.index,
        amount: i.amount,
        dueDate: i.dueDate,
      })),
      ...over,
    };
  }

  it("renames a paid purchase without disturbing its schedule", async () => {
    // The editor always sends the rows it is displaying, so an unchanged
    // schedule must not read as a reschedule and trip the payment guard.
    const paid = await paidPurchase();
    const before = await api.getPurchaseDetail(paid.id);

    const updated = await api.updatePurchase(
      paid.id,
      await payloadFor(paid.id, { productLabel: "Nouveau libellé" }),
    );

    expect(updated.purchase.productLabel).toBe("Nouveau libellé");
    expect(updated.totalPaid).toBe(before.totalPaid);
    expect(updated.installments.map((i) => i.id)).toEqual(before.installments.map((i) => i.id));
  });

  it("refuses to reschedule a purchase that has payments", async () => {
    const paid = await paidPurchase();

    // `installments: null` so the split is recomputed from the new total —
    // otherwise the stale manual list fails the sum check first, which is the
    // right error but not the one under test here.
    expect(
      await failureOf(async () =>
        api.updatePurchase(
          paid.id,
          await payloadFor(paid.id, { totalPrice: 99_999, installments: null }),
        ),
      ),
    ).toMatch(/^PURCHASE_HAS_PAYMENTS:\d+$/);

    const after = await api.getPurchaseDetail(paid.id);
    expect(after.purchase.totalPrice).not.toBe(99_999);
  });

  it("regenerates the installments of an unpaid purchase and keeps its reference", async () => {
    const target = await unpaidPurchase();
    const before = await api.getPurchaseDetail(target.id);

    const updated = await api.updatePurchase(
      target.id,
      await payloadFor(target.id, {
        totalPrice: 900,
        installmentCount: 3,
        installments: null,
      }),
    );

    expect(updated.installments.map((i) => i.amount)).toEqual([300, 300, 300]);
    expect(updated.remaining).toBe(900);
    expect(updated.purchase.reference).toBe(before.purchase.reference);
  });

  it("rejects a manual split that does not add up, changing nothing", async () => {
    const target = await unpaidPurchase();
    const before = await api.getPurchaseDetail(target.id);

    expect(
      await failureOf(async () =>
        api.updatePurchase(
          target.id,
          await payloadFor(target.id, {
            totalPrice: 1000,
            installmentCount: 2,
            installments: [
              { index: 1, amount: 400, dueDate: before.purchase.purchaseDate },
              { index: 2, amount: 500, dueDate: before.purchase.purchaseDate },
            ],
          }),
        ),
      ),
    ).toBe("SUM_MISMATCH:900:1000");

    const after = await api.getPurchaseDetail(target.id);
    expect(after.installments.length).toBe(before.installments.length);
  });

  it("refuses to edit an archived purchase", async () => {
    const target = await unpaidPurchase();
    const payload = await payloadFor(target.id, { productLabel: "Trop tard" });
    await api.archivePurchase(target.id);

    expect(await failureOf(() => api.updatePurchase(target.id, payload))).toBe("PURCHASE_ARCHIVED");
  });
});
