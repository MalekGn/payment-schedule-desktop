// Integration suite — the backup/restore contract across the `api` gateway.
//
// Restore is the one command in the app that discards everything the user has,
// and it is the newest, so this suite pins the parts a unit test on either side
// of the boundary cannot see on its own:
//
//   1. the shape and ordering `listBackups` promises the picker, which renders
//      it directly and has no other source of truth for it,
//   2. that a refused restore is refused *before* anything changes — the whole
//      guarantee the Rust side makes, and the reason validation happens ahead of
//      the swap rather than after it, and
//   3. that both restore failure codes reach a real sentence in all three
//      locales. A user staring at "une erreur est survenue" cannot tell "pick a
//      different file" from "your data is safe, try again", and those call for
//      opposite next moves.
//
// What this suite deliberately does *not* cover: the file swap itself. There is
// no filesystem behind the browser gateway, so `stage_and_swap`,
// `Db::replace_file` and the pre-restore snapshot are pinned by the Rust tests
// in `commands.rs`, `db.rs` and `autobackup.rs` instead.
//
// Run with:  npm run test:integration   (NOT part of the default `npm test`).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";

import ar from "@/locales/ar.json";
import en from "@/locales/en.json";
import fr from "@/locales/fr.json";
import { parseErrorCode, toUserMessage } from "@/lib/errors";
import type { BackupKind } from "@/types/models";

let api: typeof import("@/api").api;

beforeEach(async () => {
  vi.resetModules();
  ({ api } = await import("@/api"));
  vi.spyOn(console, "error").mockImplementation(() => {});
});

/** Capture the code a rejected api call produced. */
async function codeOf(run: () => Promise<unknown>): Promise<string> {
  try {
    await run();
  } catch (e) {
    return parseErrorCode(e)?.code ?? `UNPARSEABLE(${String(e)})`;
  }
  throw new Error("expected the call to reject, but it resolved");
}

const LOCALES = [
  ["fr", fr],
  ["en", en],
  ["ar", ar],
] as const;

describe("the snapshot listing", () => {
  it("is newest first, so the default choice is the least data lost", async () => {
    const backups = await api.listBackups();
    expect(backups.length).toBeGreaterThan(0);

    const dates = backups.map((b) => b.takenAt);
    expect([...dates].sort().reverse()).toEqual(dates);
  });

  it("carries everything the picker renders, with nothing left to infer", async () => {
    const backups = await api.listBackups();

    for (const entry of backups) {
      // `path` is echoed straight back to `restoreDatabase`; the renderer never
      // opens it and could not reconstruct it from the rest.
      expect(entry.path).toContain(entry.fileName);
      expect(entry.takenAt).toMatch(/^\d{4}-\d{2}-\d{2}$/);
      expect(entry.sizeBytes).toBeGreaterThan(0);
      expect(["auto", "preMigration", "preRestore"]).toContain(entry.kind);
    }
  });

  it("names every kind in every language", () => {
    const kinds: BackupKind[] = ["auto", "preMigration", "preRestore"];

    for (const [locale, messages] of LOCALES) {
      const i18n = createI18n({ legacy: false, locale, messages: { [locale]: messages } });
      const t = i18n.global.t as (key: string) => string;

      for (const kind of kinds) {
        const key = `settings.backupKind_${kind}`;
        expect(t(key), `${key} in ${locale}`).not.toBe(key);
      }
    }
  });
});

describe("a refused restore", () => {
  it("rejects a source that is not a database", async () => {
    expect(await codeOf(() => api.restoreDatabase("/tmp/holiday-photos.zip"))).toBe(
      "INVALID_BACKUP_FILE",
    );
  });

  it("leaves the ledger exactly as it was", async () => {
    // The property the Rust side is built around: validation runs before the
    // swap, so a refusal costs the user nothing. If that order were ever
    // inverted, this is the assertion that would catch it.
    const before = await api.listClients("all");
    const purchasesBefore = await api.listPurchases("all");

    await codeOf(() => api.restoreDatabase("/tmp/not-a-backup.txt"));

    expect(await api.listClients("all")).toEqual(before);
    expect(await api.listPurchases("all")).toEqual(purchasesBefore);
  });

  it("says which of the two failures happened, in every language", () => {
    for (const [locale, messages] of LOCALES) {
      const i18n = createI18n({ legacy: false, locale, messages: { [locale]: messages } });
      const t = i18n.global.t as (key: string, named?: Record<string, unknown>) => string;

      const invalid = toUserMessage("INVALID_BACKUP_FILE", t);
      const failed = toUserMessage("RESTORE_FAILED", t);

      for (const [code, message] of [
        ["INVALID_BACKUP_FILE", invalid],
        ["RESTORE_FAILED", failed],
      ] as const) {
        expect(message, `${code} in ${locale}`).not.toBe(t("errors.generic"));
        expect(message, `${code} in ${locale}`).not.toContain(code);
        expect(message.length, `${code} in ${locale}`).toBeGreaterThan(10);
      }

      // They must not collapse to the same sentence: one means "pick a
      // different file", the other means "your data is safe, try again".
      expect(invalid).not.toBe(failed);
    }
  });
});

describe("an accepted restore", () => {
  it("replaces the ledger rather than merging into it", async () => {
    const seeded = (await api.listClients("all")).length;

    await api.createClient({
      firstName: "Recorded",
      lastName: "After the snapshot",
      phone: "+216 20 000 000",
      address: "",
      email: null,
    });
    expect((await api.listClients("all")).length).toBe(seeded + 1);

    const [newest] = await api.listBackups();
    await api.restoreDatabase(newest.path);

    // Not "the client was deleted" — the whole database is the one the snapshot
    // held, and that client had not been entered when it was taken.
    expect((await api.listClients("all")).length).toBe(seeded);
  });

  it("returns the restored settings, which the caller reloads against", async () => {
    const [newest] = await api.listBackups();
    const restored = await api.restoreDatabase(newest.path);

    // A snapshot describes a whole database, settings included: what comes back
    // is the snapshot's configuration, not the one the renderer was holding.
    expect(restored).toEqual(await api.getSettings());
  });
});
