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
    "INSTALLMENT_COUNT_MISMATCH:5:1",
    "TEXT_TOO_LONG:120",
    "TEXT_REQUIRED",
    "INVALID_SETTING_VALUE",
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
    // Two positional params again, so a mis-ordered CODE_PARAMS entry would
    // silently swap "sent" and "declared" in the sentence.
    expect(toUserMessage("INSTALLMENT_COUNT_MISMATCH:5:1", t)).toContain("5");
    expect(toUserMessage("INSTALLMENT_COUNT_MISMATCH:5:1", t)).toContain("1");
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

  // `backup_database` returns the updated `Settings` so the renderer can clear
  // the staleness nudge from the same round trip. That contract is invisible to
  // the unit suite — it lives in the shape the gateway hands back — and the mock
  // is what the E2E build runs against, so a drift here means the browser build
  // silently keeps nudging after a successful backup.
  it("returns settings carrying the new backup date", async () => {
    const before = await api.getSettings();
    expect(before.lastBackupAt).toBeNull();

    const after = await api.backupDatabase("/tmp/payment-schedule-backup.db");
    expect(after.lastBackupAt).toBe(todayIso());

    // And it is persisted, not just returned: a later read agrees.
    expect((await api.getSettings()).lastBackupAt).toBe(todayIso());
  });

  // The automatic snapshots are taken in Rust at launch, so the browser build
  // never writes this key — but it must still travel the gateway, or the
  // Settings page renders `undefined` where the date belongs.
  it("carries the automatic-copy date through the gateway", async () => {
    const { mockDb } = await import("@/api/mock");

    expect((await api.getSettings()).lastAutoBackupAt).toBeNull();

    mockDb.settings.last_auto_backup_at = "2026-08-07";
    expect((await api.getSettings()).lastAutoBackupAt).toBe("2026-08-07");

    // And it survives a write to the settings the renderer *can* change —
    // nothing in `SettingsPatch` may clear it.
    const patched = await api.updateSettings({ shopInfo: "Rue de Marseille" });
    expect(patched.lastAutoBackupAt).toBe("2026-08-07");
  });

  // The schedule is the one part of the backup story the shop controls, so the
  // gateway has to carry it both ways — and the mock has to refuse exactly what
  // Rust refuses, or the browser and desktop builds disagree about what a valid
  // time is.
  it("round-trips the backup schedule", async () => {
    const initial = await api.getSettings();
    expect(initial.autoBackupEnabled).toBe(true);
    expect(initial.autoBackupFrequency).toBe("daily");
    expect(initial.autoBackupTime).toBe("17:00");

    const updated = await api.updateSettings({
      autoBackupFrequency: "weekly",
      autoBackupTime: "9:05",
      autoBackupEnabled: false,
    });
    expect(updated.autoBackupFrequency).toBe("weekly");
    // Normalised on write, exactly as `canonical_time` does in Rust, so the
    // value always round-trips through `<input type="time">`.
    expect(updated.autoBackupTime).toBe("09:05");
    expect(updated.autoBackupEnabled).toBe(false);

    expect((await api.getSettings()).autoBackupTime).toBe("09:05");
  });

  // Width-lenient exactly where chrono's `%H:%M` is, so the browser build never
  // refuses a time the desktop build would take.
  it.each([
    ["9:05", "09:05"],
    ["17:6", "17:06"],
    [" 09:30 ", "09:30"],
  ])("accepts %o as a backup time and stores %o", async (given, stored) => {
    expect((await api.updateSettings({ autoBackupTime: given })).autoBackupTime).toBe(stored);
  });

  it.each(["25:00", "17:60", "17", "", "5pm", "17:00:00", "17:"])(
    "refuses %o as a backup time",
    async (bad) => {
      // `codeOf`, not `.rejects`: the mock throws synchronously out of
      // `api.updateSettings`, so there is no promise for `.rejects` to unwrap.
      expect(await codeOf(() => api.updateSettings({ autoBackupTime: bad }))).toBe(
        "INVALID_SETTING_VALUE",
      );
    },
  );

  it("refuses a frequency outside the offered set", async () => {
    expect(await codeOf(() => api.updateSettings({ autoBackupFrequency: "hourly" }))).toBe(
      "INVALID_SETTING_VALUE",
    );
    // And a rejected patch writes nothing.
    expect((await api.getSettings()).autoBackupFrequency).toBe("daily");
  });

  it("keeps the recorded date out of the writable settings patch", async () => {
    // `lastBackupAt` is read-only by construction — the renderer serializes
    // `SettingsPatch`, and a field it could write would let the UI lie about
    // when the ledger was last copied. Nothing but a real backup may set it.
    await api.backupDatabase("/tmp/payment-schedule-backup.db");
    const patched = await api.updateSettings({ shopInfo: "Rue de Marseille" });

    expect(patched.shopInfo).toBe("Rue de Marseille");
    expect(patched.lastBackupAt).toBe(todayIso());
  });
});
