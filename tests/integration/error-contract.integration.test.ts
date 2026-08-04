// Integration suite — the error contract between backend, gateway and UI text.
//
// Every command rejects with a stable machine code (see
// `src-tauri/src/error.rs`), which `src/lib/errors.ts` turns into a localized
// sentence. This suite pins both halves together:
//
//   1. the mock throws exactly the codes the Rust guards produce, so the
//      browser/E2E builds reject the same inputs as the desktop app, and
//   2. every one of those codes resolves to a real message in all three
//      locales — never to a raw code, a dotted key path, or SQL text.
//
// The failure this guards against is the one the audit called a blocker:
// `ui.notify(String(e), "error")` used to put `FOREIGN KEY constraint failed`
// in front of a shopkeeper, unlocalized.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";

import ar from "@/locales/ar.json";
import en from "@/locales/en.json";
import fr from "@/locales/fr.json";
import { parseErrorCode, toUserMessage } from "@/lib/errors";
import { todayIso } from "@/lib/finance";
import type { PurchaseInput } from "@/types/models";

let api: typeof import("@/api").api;

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
  // `toUserMessage` logs the original error; keep the suite output readable.
  vi.spyOn(console, "error").mockImplementation(() => {});
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

/** Capture the code a rejected api call produced. */
async function codeOf(run: () => Promise<unknown>): Promise<string> {
  try {
    await run();
  } catch (e) {
    return parseErrorCode(e)?.code ?? `UNPARSEABLE(${String(e)})`;
  }
  throw new Error("expected the call to reject, but it resolved");
}

describe("create_purchase rejects invalid input with stable codes", () => {
  it.each([
    ["INVALID_TOTAL_PRICE", { totalPrice: 0 }],
    ["INVALID_TOTAL_PRICE", { totalPrice: -500 }],
    ["INVALID_INSTALLMENT_COUNT", { installmentCount: 0 }],
    // The upper bound is what stops a hostile count sizing an allocation and
    // an insert loop on the Rust side.
    ["INVALID_INSTALLMENT_COUNT", { installmentCount: 121 }],
    ["INVALID_INTERVAL_KIND", { intervalKind: "fortnightly" as never }],
    ["INVALID_INTERVAL_DAYS", { intervalKind: "custom" as never, intervalDays: 0 }],
    ["INVALID_INTERVAL_DAYS", { intervalKind: "custom" as never, intervalDays: 400 }],
    ["INVALID_DATE", { purchaseDate: "15/01/2024" }],
  ])("%s", async (expected, over) => {
    expect(await codeOf(() => api.createPurchase(newPurchase(over)))).toBe(expected);
  });

  it("rejects a malformed manual due date rather than storing it", async () => {
    const code = await codeOf(() =>
      api.createPurchase(
        newPurchase({
          installmentCount: 1,
          installments: [{ index: 1, amount: 1000, dueDate: "tomorrow" }],
        }),
      ),
    );
    expect(code).toBe("INVALID_DATE");
  });

  it("still reports the sum and the total on a mismatch", async () => {
    const input = newPurchase({
      installmentCount: 2,
      installments: [
        { index: 1, amount: 400, dueDate: todayIso() },
        { index: 2, amount: 500, dueDate: todayIso() },
      ],
    });
    try {
      await api.createPurchase(input);
      throw new Error("expected rejection");
    } catch (e) {
      expect(parseErrorCode(e)).toEqual({ code: "SUM_MISMATCH", params: ["900", "1000"] });
    }
  });

  it("leaves no purchase behind when it rejects", async () => {
    const before = (await api.listPurchases()).length;
    await codeOf(() => api.createPurchase(newPurchase({ totalPrice: -1 })));
    expect((await api.listPurchases()).length).toBe(before);
  });
});

describe("record_payment rejects overpayment", () => {
  it("reports the remaining balance and records nothing", async () => {
    const detail = await api.createPurchase(newPurchase({ totalPrice: 900, installmentCount: 3 }));
    const inst = detail.installments[0];
    expect(inst.amount).toBe(300);

    await api.recordPayment({
      installmentId: inst.id,
      amount: 100,
      paymentDate: todayIso(),
      note: null,
    });

    try {
      await api.recordPayment({
        installmentId: inst.id,
        amount: 250,
        paymentDate: todayIso(),
        note: null,
      });
      throw new Error("expected rejection");
    } catch (e) {
      expect(parseErrorCode(e)).toEqual({ code: "OVERPAYMENT", params: ["200"] });
    }

    // The partial payment stands; the overpayment left no trace, and the
    // installment never reports more paid than it is worth.
    const after = await api.getPurchaseDetail(detail.purchase.id);
    expect(after.installments[0].paidAmount).toBe(100);
    expect(after.installments[0].paidDate).toBeNull();
    expect(after.totalPaid).toBe(100);
    expect(after.installments.every((i) => i.paidAmount <= i.amount)).toBe(true);
  });

  it("accepts a payment for exactly the remaining balance", async () => {
    const detail = await api.createPurchase(newPurchase({ totalPrice: 900, installmentCount: 3 }));
    const inst = detail.installments[0];

    await api.recordPayment({
      installmentId: inst.id,
      amount: 100,
      paymentDate: todayIso(),
      note: null,
    });
    const after = await api.recordPayment({
      installmentId: inst.id,
      amount: 200,
      paymentDate: todayIso(),
      note: null,
    });

    expect(after.installments[0].paidAmount).toBe(300);
    expect(after.installments[0].paidDate).toBe(todayIso());
    expect(after.installments[0].status).toBe("paid");
  });

  it.each([
    ["INVALID_AMOUNT", { amount: 0 }],
    ["INVALID_AMOUNT", { amount: -5 }],
    ["INVALID_DATE", { paymentDate: "not-a-date" }],
    ["INSTALLMENT_NOT_FOUND", { installmentId: 999999 }],
  ])("%s", async (expected, over) => {
    const detail = await api.createPurchase(newPurchase());
    const code = await codeOf(() =>
      api.recordPayment({
        installmentId: detail.installments[0].id,
        amount: 10,
        paymentDate: todayIso(),
        note: null,
        ...over,
      }),
    );
    expect(code).toBe(expected);
  });
});

describe("every code maps to a localized sentence", () => {
  // Kept in step with the inventory in src-tauri/src/error.rs.
  const CODES = [
    "INTERNAL",
    "CLIENT_HAS_PURCHASES:3",
    "ARCHIVE_HAS_OUTSTANDING:750",
    "CLIENT_ARCHIVED",
    "CLIENT_NOT_FOUND",
    "PURCHASE_NOT_FOUND",
    "PURCHASE_HAS_PAYMENTS:2",
    "PURCHASE_ARCHIVED",
    "PURCHASE_NOT_ARCHIVED",
    "INSTALLMENT_NOT_FOUND",
    "AMOUNT_LOCKED",
    "DUE_DATE_LOCKED",
    "DUE_DATE_OUT_OF_ORDER",
    "FUTURE_PAID_DATE",
    "PREVIOUS_UNPAID:2",
    "BELOW_PAID:100",
    "PAID_ABOVE_AMOUNT:250",
    "NO_PAYMENT_TO_DATE",
    "PAYMENT_DATE_LOCKED",
    "SCHEDULE_VIA_PURCHASE",
    "NO_REBALANCE_ROOM",
    "INVALID_DATE",
    "INVALID_TOTAL_PRICE",
    "INVALID_INSTALLMENT_COUNT",
    "INVALID_INTERVAL_KIND",
    "INVALID_INTERVAL_DAYS",
    "INVALID_AMOUNT",
    "SUM_MISMATCH:900:1000",
    "OVERPAYMENT:200",
    "INVALID_LOGO_TYPE",
    "LOGO_TOO_LARGE",
    "BACKUP_FAILED",
  ];

  const LOCALES = { fr, en, ar } as const;

  for (const locale of Object.keys(LOCALES) as (keyof typeof LOCALES)[]) {
    describe(locale, () => {
      const i18n = createI18n({
        legacy: false,
        locale,
        fallbackLocale: locale,
        messages: LOCALES,
      });
      const t = i18n.global.t as unknown as (k: string, n?: Record<string, unknown>) => string;

      it.each(CODES)("%s reads as prose", (code) => {
        const message = toUserMessage(code, t);
        expect(message.length).toBeGreaterThan(0);
        // Not the code, not a dotted key path, not a leftover placeholder.
        expect(message).not.toContain(code.split(":")[0]);
        expect(message).not.toMatch(/^errors\./);
        expect(message).not.toMatch(/\{[a-z]+\}/i);
      });
    });
  }

  it("interpolates the parameters a code carries", () => {
    const i18n = createI18n({ legacy: false, locale: "en", messages: LOCALES });
    const t = i18n.global.t as unknown as (k: string, n?: Record<string, unknown>) => string;

    expect(toUserMessage("SUM_MISMATCH:900:1000", t)).toContain("900");
    expect(toUserMessage("SUM_MISMATCH:900:1000", t)).toContain("1000");
    expect(toUserMessage("OVERPAYMENT:200", t)).toContain("200");
    expect(toUserMessage("CLIENT_HAS_PURCHASES:3", t)).toContain("3");
    expect(toUserMessage("ARCHIVE_HAS_OUTSTANDING:750", t)).toContain("750");
    expect(toUserMessage("PREVIOUS_UNPAID:2", t)).toContain("2");
    expect(toUserMessage("BELOW_PAID:100", t)).toContain("100");
    expect(toUserMessage("PAID_ABOVE_AMOUNT:250", t)).toContain("250");
  });

  it("falls back to the generic message for anything unrecognised", () => {
    const i18n = createI18n({ legacy: false, locale: "en", messages: LOCALES });
    const t = i18n.global.t as unknown as (k: string, n?: Record<string, unknown>) => string;

    // The shapes a leaked internal error would have taken before AppError.
    expect(toUserMessage(new Error("FOREIGN KEY constraint failed"), t)).toBe(en.errors.generic);
    expect(toUserMessage("UNIQUE constraint failed: client.id", t)).toBe(en.errors.generic);
    expect(toUserMessage("/home/malek/.local/share/app/payment_schedule.db", t)).toBe(
      en.errors.generic,
    );
    // A code we have not localized yet must not surface its key path.
    expect(toUserMessage("SOME_FUTURE_CODE", t)).toBe(en.errors.generic);
  });
});

describe("backup", () => {
  it("is reachable through the gateway", async () => {
    const { mockDb } = await import("@/api/mock");
    await api.backupDatabase("/tmp/payment-schedule-backup.db");
    expect(mockDb.lastBackupPath).toBe("/tmp/payment-schedule-backup.db");
  });
});
