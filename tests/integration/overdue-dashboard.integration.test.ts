// Integration suite — overdue (impayés), dashboard, and cascade-delete flows.
//
// These cross-check that the several read models the UI depends on stay
// mutually consistent when served from the same backend state, and that a
// mutation on one screen (recording a payment, deleting a client) is reflected
// everywhere it should be. As with the lifecycle suite, each test runs against
// a freshly re-seeded in-memory backend reached through the real `api` facade.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { todayIso } from "@/lib/finance";

let api: typeof import("@/api").api;

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
});

describe("the payment ledger bounds what it will return", () => {
  it("clamps a hostile limit instead of returning the whole table", async () => {
    const all = await api.listAllPayments();
    expect(all.length).toBeGreaterThan(1);

    // SQLite reads a negative LIMIT as *no* limit, so this used to return
    // everything. The mock had the opposite bug — `.slice(0, -1)` quietly drops
    // the last row — so mirroring the clamp is what keeps the two agreeing.
    for (const hostile of [-1, -999, 0]) {
      expect(await api.listAllPayments(hostile)).toHaveLength(1);
    }

    // An ordinary request is untouched, and the ceiling is generous enough that
    // the seeded ledger fits under it.
    expect(await api.listAllPayments(2)).toHaveLength(2);
    expect(await api.listAllPayments(1_000_000)).toHaveLength(all.length);
  });
});

describe("the dashboard aggregates reconcile with the individual read models", () => {
  it("agrees with impayés, clients, purchases, and payments on the seeded data", async () => {
    const [dash, impayes, clients, purchases, payments] = await Promise.all([
      api.getDashboard(),
      api.listImpayes(),
      api.listClients(),
      api.listPurchases(),
      api.listAllPayments(),
    ]);

    // Overdue: one impayé group per client, installment counts line up.
    expect(dash.stats.overdueClients).toBe(impayes.length);
    expect(dash.stats.overdueCount).toBe(impayes.reduce((s, c) => s + c.overdueCount, 0));

    // Money aggregates are just fan-ins of the per-entity numbers.
    expect(dash.stats.totalPurchases).toBe(purchases.length);
    expect(dash.stats.totalSales).toBe(purchases.reduce((s, p) => s + p.totalPrice, 0));
    expect(dash.stats.totalCollected).toBe(payments.reduce((s, p) => s + p.amount, 0));
    expect(dash.stats.totalOutstanding).toBe(clients.reduce((s, c) => s + c.totalOutstanding, 0));

    // The embedded impayés preview is a most-owed-first prefix of the full list.
    expect(dash.impayes.length).toBeLessThanOrEqual(impayes.length);
    expect(dash.impayes.map((c) => c.clientId)).toEqual(
      impayes.slice(0, dash.impayes.length).map((c) => c.clientId),
    );
  });
});

describe("the ImpayeFilter narrows the overdue list through the api facade", () => {
  it("restricts to a single client and preserves that client's installments", async () => {
    const all = await api.listImpayes();
    expect(all.length).toBeGreaterThan(0);
    const target = all[0];

    const one = await api.listImpayes({ clientId: target.clientId });
    expect(one).toHaveLength(1);
    expect(one[0].clientId).toBe(target.clientId);
    expect(one[0].installments.map((i) => i.installmentId)).toEqual(
      target.installments.map((i) => i.installmentId),
    );
  });

  it("returns an empty list when the date window excludes every due date", async () => {
    expect(await api.listImpayes({ dateFrom: "2999-01-01" })).toEqual([]);
    expect(await api.listImpayes({ dateTo: "1900-01-01" })).toEqual([]);
  });
});

describe("settling an overdue installment clears it from every overdue view", () => {
  it("drops the installment from impayés and decrements the dashboard counts", async () => {
    const before = await api.listImpayes();
    const client = before[0];
    const inst = client.installments[0];

    const dashBefore = await api.getDashboard();

    // Pay the overdue installment in full -> it no longer qualifies as overdue.
    await api.recordPayment({
      installmentId: inst.installmentId,
      amount: inst.remaining,
      paymentDate: todayIso(),
      note: null,
    });

    const dashAfter = await api.getDashboard();
    expect(dashAfter.stats.overdueCount).toBe(dashBefore.stats.overdueCount - 1);

    const after = await api.listImpayes({ clientId: client.clientId });
    const stillListed = after.flatMap((c) => c.installments).map((i) => i.installmentId);
    expect(stillListed).not.toContain(inst.installmentId);
  });
});

describe("deleting a client is confined to clients with no history", () => {
  it("refuses the delete outright when the client has purchases", async () => {
    const clients = await api.listClients();
    const withPurchases = clients.find((c) => c.purchaseCount > 0)!;
    expect(withPurchases).toBeDefined();
    const purchasesBefore = await api.listPurchases();

    let error: unknown;
    try {
      await api.deleteClient(withPurchases.id);
    } catch (e) {
      error = e;
    }
    expect(String(error)).toMatch(
      new RegExp(`CLIENT_HAS_PURCHASES:${withPurchases.purchaseCount}`),
    );

    // There is no `force` to escalate to any more, so nothing anywhere moved.
    expect((await api.listClients()).length).toBe(clients.length);
    expect((await api.listPurchases()).length).toBe(purchasesBefore.length);
    const dash = await api.getDashboard();
    expect(dash.stats.totalPurchases).toBe(purchasesBefore.length);
  });

  it("deletes a client who has no purchases", async () => {
    const before = await api.listClients();
    const fresh = await api.createClient({
      firstName: "Zied",
      lastName: "Zzzsupprime",
      phone: "+216 20 000 000",
      address: "",
      email: null,
    });
    expect((await api.listClients()).length).toBe(before.length + 1);

    await api.deleteClient(fresh.id);

    const after = await api.listClients();
    expect(after.length).toBe(before.length);
    expect(after.some((c) => c.id === fresh.id)).toBe(false);
  });

  it("reports a delete of an id that is already gone", async () => {
    // Awaited inside a try, not `.rejects`: on the browser/mock path the
    // gateway runs the mock synchronously, so the failure is thrown out of
    // `api.deleteClient(...)` rather than carried by a promise.
    let error: unknown;
    try {
      await api.deleteClient(999_999);
    } catch (e) {
      error = e;
    }
    expect(String(error)).toMatch(/CLIENT_NOT_FOUND/);
  });
});
