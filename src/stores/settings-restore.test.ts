// Unit tests for the restore half of the settings store.
//
// Restore is the one action in the app that discards everything the user has,
// and the store is where the frontend's side of that contract lives: a rejected
// restore must leave the store untouched (the backend promises the database is
// untouched too, and the two have to agree), and the codes the backend rejects
// with have to reach a sentence in every language rather than the generic
// fallback — a user staring at "an error occurred" cannot tell "pick a
// different file" from "your data is safe, try again".

import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { createI18n } from "vue-i18n";

import { mockDb } from "@/api/mock";
import { useSettingsStore } from "@/stores/settings";
import { toUserMessage } from "@/lib/errors";
import fr from "@/locales/fr.json";
import en from "@/locales/en.json";
import ar from "@/locales/ar.json";

/** The error a rejected call carried, as the integration suites read it. */
async function failureOf(call: () => Promise<unknown>): Promise<unknown> {
  try {
    await call();
  } catch (e) {
    return e;
  }
  throw new Error("expected the call to fail");
}

describe("restore", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("lists the snapshots newest first, classified", async () => {
    const settings = useSettingsStore();
    const backups = await settings.listBackups();

    expect(backups.length).toBeGreaterThan(0);
    expect(backups.map((b) => b.kind)).toContain("auto");

    const dates = backups.map((b) => b.takenAt);
    expect([...dates].sort().reverse()).toEqual(dates);
  });

  it("adopts the settings the restored database carries", async () => {
    const settings = useSettingsStore();
    const [first] = await settings.listBackups();

    // A snapshot describes a whole database, settings included: whatever the
    // store was holding is replaced by what came back, not merged with it.
    settings.settings.currencyCode = "EUR";
    settings.settings.shopInfo = "typed since the snapshot was taken";

    await settings.restoreDatabase(first.path);

    expect(settings.currencyCode).toBe("TND");
    expect(settings.settings.shopInfo).toBe("");
    expect(mockDb.lastRestoreSource).toBe(first.path);
  });

  it("brings the ledger back to what the snapshot held", async () => {
    const settings = useSettingsStore();
    const seeded = mockDb.listClients("all").length;

    mockDb.createClient({
      firstName: "Recorded",
      lastName: "After the snapshot",
      phone: "+216 20 000 000",
      address: "",
      email: null,
    });
    expect(mockDb.listClients("all").length).toBe(seeded + 1);

    const [first] = await settings.listBackups();
    await settings.restoreDatabase(first.path);

    // Not "the client was deleted" — the whole database was replaced by one
    // taken before they existed.
    expect(mockDb.listClients("all").length).toBe(seeded);
  });

  it("rejects a source that is not a database, and changes nothing", async () => {
    const settings = useSettingsStore();
    const before = { ...settings.settings };

    const e = await failureOf(() => settings.restoreDatabase("/tmp/holiday-photos.zip"));

    expect(String(e)).toContain("INVALID_BACKUP_FILE");
    expect(settings.settings).toEqual(before);
  });

  it("gives both restore failures a real sentence in all three languages", () => {
    for (const [locale, messages] of [
      ["fr", fr],
      ["en", en],
      ["ar", ar],
    ] as const) {
      const i18n = createI18n({ legacy: false, locale, messages: { [locale]: messages } });
      const t = i18n.global.t as (key: string, named?: Record<string, unknown>) => string;

      for (const code of ["INVALID_BACKUP_FILE", "RESTORE_FAILED"]) {
        const message = toUserMessage(code, t);
        expect(message, `${code} in ${locale}`).not.toBe(t("errors.generic"));
        expect(message).not.toContain(code);
      }
    }
  });
});
