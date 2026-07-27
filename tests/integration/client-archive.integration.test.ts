// Integration suite — archiving, restoring, and the guards around both.
//
// Archiving replaced the destructive `force` cascade on `delete_client`. The
// property these tests exist to protect is narrow but load-bearing: an archived
// client always has a zero balance, which is the only reason impayés, the
// dashboard and the reports are allowed to skip an `archived_at` filter. If a
// future change lets a debtor be archived — or lets an archived client take on
// a new purchase — money silently leaves the books, and the failure is invisible
// on every screen that shows a total.
//
// Runs against a freshly re-seeded in-memory backend reached through the real
// `api` facade, like the other suites here.
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
 * `Promise.resolve(mockDb.x())`, so the mock runs *before* the promise exists
 * and a rejection surfaces as a synchronous throw out of `api.x(...)` — under
 * Tauri the same failure arrives as a rejected promise. Awaiting inside a
 * `try` is what the rest of the integration suites do, and it is the only
 * shape that reads both backends identically.
 */
async function failureOf(call: () => Promise<unknown>): Promise<string> {
  try {
    await call();
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
  throw new Error("expected the call to fail, but it resolved");
}

/** Pay every installment of every purchase this client has. */
async function settleEverything(clientId: number): Promise<void> {
  const detail = await api.getClientDetail(clientId);
  for (const purchase of detail.purchases) {
    const full = await api.getPurchaseDetail(purchase.id);
    for (const inst of full.installments) {
      const remaining = inst.amount - inst.paidAmount;
      if (remaining > 0) {
        await api.recordPayment({
          installmentId: inst.id,
          amount: remaining,
          paymentDate: full.purchase.purchaseDate,
          note: null,
        });
      }
    }
  }
}

/** The seeded client who owes nothing — Salma Jlassi, one fully paid purchase. */
async function settledClient() {
  const clients = await api.listClients();
  const c = clients.find((x) => x.totalOutstanding === 0 && x.purchaseCount > 0);
  expect(c, "expected a seeded client with a fully paid purchase").toBeDefined();
  return c!;
}

/** A seeded client who still owes money. */
async function indebtedClient() {
  const clients = await api.listClients();
  const c = clients.find((x) => x.totalOutstanding > 0);
  expect(c, "expected a seeded client with an outstanding balance").toBeDefined();
  return c!;
}

describe("archiving is gated on the client owing nothing", () => {
  it("refuses to archive a client who still has a balance", async () => {
    const debtor = await indebtedClient();

    expect(await failureOf(() => api.archiveClient(debtor.id))).toBe(
      `ARCHIVE_HAS_OUTSTANDING:${debtor.totalOutstanding}`,
    );

    // The refusal must not have written the stamp.
    const after = (await api.listClients()).find((c) => c.id === debtor.id)!;
    expect(after.archivedAt).toBeNull();
  });

  it("refuses both delete and archive for the same indebted client", async () => {
    // The deliberate consequence of the policy: someone who owes you money can
    // be neither erased nor hidden. Asserted explicitly so a later "helpful"
    // change cannot quietly open one of the two doors.
    const debtor = await indebtedClient();

    expect(await failureOf(() => api.deleteClient(debtor.id))).toMatch(/^CLIENT_HAS_PURCHASES:/);
    expect(await failureOf(() => api.archiveClient(debtor.id))).toMatch(
      /^ARCHIVE_HAS_OUTSTANDING:/,
    );

    const still = (await api.listClients()).find((c) => c.id === debtor.id);
    expect(still).toBeDefined();
    expect(still!.archivedAt).toBeNull();
  });

  it("allows the archive once every installment is settled", async () => {
    const debtor = await indebtedClient();
    await settleEverything(debtor.id);

    await api.archiveClient(debtor.id);

    const archived = (await api.listClients("archived")).find((c) => c.id === debtor.id);
    expect(archived).toBeDefined();
    expect(archived!.archivedAt).not.toBeNull();
  });

  it("archives a client who has no purchases at all", async () => {
    // The empty-aggregate case: SUM over no installments must read as 0, not null.
    const fresh = await api.createClient({
      firstName: "Sans",
      lastName: "Achat",
      phone: "",
      address: "",
      email: null,
    });

    await api.archiveClient(fresh.id);

    expect((await api.listClients("archived")).some((c) => c.id === fresh.id)).toBe(true);
  });
});

describe("the scope filter partitions the client list", () => {
  it("moves the client between active and archived, and all is the union", async () => {
    const target = await settledClient();
    const activeBefore = await api.listClients("active");
    const allBefore = await api.listClients("all");

    await api.archiveClient(target.id);

    const active = await api.listClients("active");
    const archived = await api.listClients("archived");
    const all = await api.listClients("all");

    expect(active.some((c) => c.id === target.id)).toBe(false);
    expect(archived.some((c) => c.id === target.id)).toBe(true);
    expect(all.some((c) => c.id === target.id)).toBe(true);

    expect(active.length).toBe(activeBefore.length - 1);
    expect(all.length).toBe(allBefore.length);
    expect(active.length + archived.length).toBe(all.length);
  });

  it("defaults to active when no scope is given", async () => {
    const target = await settledClient();
    await api.archiveClient(target.id);

    const defaulted = await api.listClients();
    const explicit = await api.listClients("active");
    expect(defaulted.map((c) => c.id)).toEqual(explicit.map((c) => c.id));
  });
});

describe("archiving hides the client, never their history", () => {
  it("keeps the detail page, purchases and payments reachable", async () => {
    const target = await settledClient();
    const detailBefore = await api.getClientDetail(target.id);
    const paymentsBefore = await api.listPaymentsForClient(target.id);
    const purchasesBefore = await api.listPurchases();

    await api.archiveClient(target.id);

    // A deep link or a back-navigation from one of their purchases still works.
    const detailAfter = await api.getClientDetail(target.id);
    expect(detailAfter.client.archivedAt).not.toBeNull();
    expect(detailAfter.purchases.length).toBe(detailBefore.purchases.length);
    expect(detailAfter.totalPurchased).toBe(detailBefore.totalPurchased);
    expect(detailAfter.totalPaid).toBe(detailBefore.totalPaid);

    expect((await api.listPaymentsForClient(target.id)).length).toBe(paymentsBefore.length);
    expect((await api.listPurchases()).length).toBe(purchasesBefore.length);
  });

  it("leaves every money aggregate byte-identical", async () => {
    // This is the invariant that lets impayés/dashboard/reports skip the filter.
    const target = await settledClient();
    const dashBefore = await api.getDashboard();
    const impayesBefore = await api.listImpayes();
    const scheduleBefore = await api.listSchedule();

    await api.archiveClient(target.id);

    expect(await api.getDashboard()).toEqual(dashBefore);
    expect(await api.listImpayes()).toEqual(impayesBefore);
    expect(await api.listSchedule()).toEqual(scheduleBefore);
  });

  it("stamps an ISO date, not a timestamp, so the UI can format it", async () => {
    const target = await settledClient();
    await api.archiveClient(target.id);

    const archived = (await api.listClients("archived")).find((c) => c.id === target.id)!;
    expect(archived.archivedAt).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });
});

describe("an archived client cannot take on new debt", () => {
  it("refuses a new purchase until the client is restored", async () => {
    const target = await settledClient();
    await api.archiveClient(target.id);

    const input = {
      clientId: target.id,
      productLabel: "Télévision",
      totalPrice: 600,
      installmentCount: 3,
      intervalKind: "monthly" as const,
      intervalDays: null,
      purchaseDate: "2024-03-01",
      installments: null,
    };

    expect(await failureOf(() => api.createPurchase(input))).toBe("CLIENT_ARCHIVED");

    await api.restoreClient(target.id);
    const created = await api.createPurchase(input);
    expect(created.purchase.clientId).toBe(target.id);
  });
});

describe("restoring puts the client back", () => {
  it("clears the stamp and returns them to the active list", async () => {
    const target = await settledClient();
    await api.archiveClient(target.id);

    await api.restoreClient(target.id);

    const active = await api.listClients("active");
    const restored = active.find((c) => c.id === target.id);
    expect(restored).toBeDefined();
    expect(restored!.archivedAt).toBeNull();
    expect((await api.listClients("archived")).some((c) => c.id === target.id)).toBe(false);
  });

  it("is a no-op when the client is already active", async () => {
    const target = await settledClient();

    await api.restoreClient(target.id);
    await api.restoreClient(target.id);

    const restored = (await api.listClients()).find((c) => c.id === target.id)!;
    expect(restored.archivedAt).toBeNull();
  });

  it("does not move the stamp when a client is archived twice", async () => {
    const target = await settledClient();
    await api.archiveClient(target.id);
    const first = (await api.listClients("archived")).find((c) => c.id === target.id)!.archivedAt;

    await api.archiveClient(target.id);

    const second = (await api.listClients("archived")).find((c) => c.id === target.id)!.archivedAt;
    expect(second).toBe(first);
  });
});

describe("archive and restore report a client that is not there", () => {
  it("rejects both with CLIENT_NOT_FOUND", async () => {
    expect(await failureOf(() => api.archiveClient(999_999))).toBe("CLIENT_NOT_FOUND");
    expect(await failureOf(() => api.restoreClient(999_999))).toBe("CLIENT_NOT_FOUND");
  });
});
