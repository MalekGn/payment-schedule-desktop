// Licence enforcement across the real `api` facade.
//
// Scope note, so this suite is not mistaken for a security test: it exercises
// the *contract* — which calls the UI makes, what the store reports, what the
// error code resolves to. The enforcement itself lives in Rust
// (`require_license` in `src-tauri/src/commands.rs`) and is covered by
// `cargo test`; the browser mock has no gate, because a gate implemented in the
// renderer would be exactly the thing that does not count.
//
// What this suite protects is the other half: that the app *presents* the
// licence state correctly and that a refusal from the backend turns into a
// localized sentence rather than a raw code.

import { describe, it, expect, beforeEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { createI18n } from "vue-i18n";
import fr from "@/locales/fr.json";
import en from "@/locales/en.json";
import ar from "@/locales/ar.json";
import { toUserMessage } from "@/lib/errors";

let api: typeof import("@/api").api;
let mockDb: typeof import("@/api/mock").mockDb;
let useLicenseStore: typeof import("@/stores/license").useLicenseStore;

beforeEach(async () => {
  vi.resetModules();
  setActivePinia(createPinia());
  ({ api } = await import("@/api"));
  ({ mockDb } = await import("@/api/mock"));
  ({ useLicenseStore } = await import("@/stores/license"));
  vi.spyOn(console, "error").mockImplementation(() => {});
});

describe("the licence verdict reaches the UI", () => {
  it("reports a valid licence with everything the settings screen renders", async () => {
    const info = await api.getLicenseStatus();

    expect(info.status).toBe("valid");
    expect(info.license).not.toBeNull();
    expect(info.license?.licensee).toBeTruthy();
    expect(info.license?.expiresAt).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    // Needed before a machine-bound licence can be issued at all.
    expect(info.machineId).toBeTruthy();
  });

  it("withholds the licence body for verdicts where nothing is attested", async () => {
    for (const status of ["invalidSignature", "malformed", "missing"] as const) {
      mockDb.setLicense(status);
      const info = await api.getLicenseStatus();
      expect(info.status).toBe(status);
      // Showing fields out of an unverified file would present forged data as
      // if the vendor had signed it.
      expect(info.license).toBeNull();
    }
  });

  it("drives the store through a full unlicensed → imported → licensed cycle", async () => {
    mockDb.setLicense("missing");
    const license = useLicenseStore();

    await license.load();
    expect(license.isLicensed).toBe(false);
    expect(license.status).toBe("missing");

    await license.importFrom("/home/user/licence.json");

    expect(license.isLicensed).toBe(true);
    expect(license.status).toBe("valid");
    // No reload is required: the command returns the new verdict directly, so
    // the UI unlocks without restarting the app.
    expect(license.license?.licenseId).toBeTruthy();
  });

  it("keeps the previous verdict when an import is refused", async () => {
    mockDb.setLicense("expired");
    const license = useLicenseStore();
    await license.load();

    await expect(license.importFrom("/home/user/not-a-licence.txt")).rejects.toThrow();

    // A bad file must never displace what was there — the Rust command
    // validates before it copies for exactly this reason.
    expect(license.status).toBe("expired");
    expect(license.isLicensed).toBe(false);
  });
});

describe("the unlicensed baseline stays usable", () => {
  it("still lists clients and purchases, which are never gated", async () => {
    mockDb.setLicense("missing");

    // The deliberate shape of the baseline: an expired or absent licence must
    // never hold a shop keeper's own records hostage.
    expect((await api.listClients()).length).toBeGreaterThan(0);
    expect((await api.listPurchases()).length).toBeGreaterThan(0);

    const clients = await api.listClients();
    const detail = await api.getClientDetail(clients[0].id);
    expect(detail.client.id).toBe(clients[0].id);
  });
});

describe("a refusal becomes a localized sentence, never a raw code", () => {
  const locales = { fr, en, ar } as const;

  it("resolves LICENSE_REQUIRED in all three languages", () => {
    for (const [locale, messages] of Object.entries(locales)) {
      const i18n = createI18n({
        legacy: false,
        locale,
        messages: { [locale]: messages },
      });
      const t = i18n.global.t as unknown as (k: string, n?: Record<string, unknown>) => string;

      const message = toUserMessage(new Error("LICENSE_REQUIRED"), t);
      expect(message).toBeTruthy();
      // The failure this guards: a missing key makes vue-i18n echo the key back,
      // so the user would read "errors.licenseRequired" as their error message.
      expect(message).not.toContain("LICENSE_REQUIRED");
      expect(message).not.toContain("errors.");
    }
  });

  it("interpolates the rejected status into INVALID_LICENSE", () => {
    for (const [locale, messages] of Object.entries(locales)) {
      const i18n = createI18n({
        legacy: false,
        locale,
        messages: { [locale]: messages },
      });
      const t = i18n.global.t as unknown as (k: string, n?: Record<string, unknown>) => string;

      const message = toUserMessage(new Error("INVALID_LICENSE:machineMismatch"), t);
      expect(message).toContain("machineMismatch");
      expect(message).not.toContain("INVALID_LICENSE");
      expect(message).not.toContain("{status}");
    }
  });
});
