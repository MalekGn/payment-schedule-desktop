// The licence store decides what the UI *shows*. It is deliberately not what
// stops anything happening — the gated Tauri commands refuse on their own in
// Rust — so what matters here is that it reports the verdict faithfully and
// fails closed when it cannot get one.

import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useLicenseStore } from "@/stores/license";
import { mockDb } from "@/api/mock";

describe("licence store — reporting the verdict", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockDb.setLicense("valid");
  });

  it("starts unlicensed before any check has run", () => {
    const license = useLicenseStore();
    // The pre-flight state must not grant access: `main.ts` mounts the app once
    // `load()` settles, but a component reading the store earlier must not see
    // a licence that was never verified.
    expect(license.loaded).toBe(false);
    expect(license.isLicensed).toBe(false);
  });

  it("unlocks the app only for a valid licence", async () => {
    const license = useLicenseStore();
    await license.load();

    expect(license.loaded).toBe(true);
    expect(license.status).toBe("valid");
    expect(license.isLicensed).toBe(true);
    expect(license.license?.licenseId).toBe("PS-MOCK-0001");
  });

  it("treats every non-valid verdict as unlicensed", async () => {
    const license = useLicenseStore();

    for (const status of [
      "expired",
      "machineMismatch",
      "invalidSignature",
      "malformed",
      "missing",
      "clockTampered",
    ] as const) {
      mockDb.setLicense(status);
      await license.load();
      expect(license.status).toBe(status);
      expect(license.isLicensed).toBe(false);
    }
  });

  it("carries the expiry date so the gate can say when the licence lapsed", async () => {
    mockDb.setLicense("expired");
    const license = useLicenseStore();
    await license.load();

    expect(license.info.expiredOn).toBe("2026-02-01");
    // The licence itself is still carried: its signature verified, so showing
    // the licensee name back to the user is safe.
    expect(license.license?.licensee).toBeTruthy();
  });

  it("exposes this machine's fingerprint, which the customer must report", async () => {
    const license = useLicenseStore();
    await license.load();
    // Without this the vendor cannot issue a machine-bound licence at all.
    expect(license.machineId).toMatch(/^[0-9a-z]+$/);
  });

  it("fails closed when the licence check itself throws", async () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(mockDb, "getLicenseStatus").mockImplementation(() => {
      throw new Error("backend unreachable");
    });

    const license = useLicenseStore();
    await license.load();

    // Granting access we could not verify would produce buttons that error when
    // pressed, because the Rust gate would refuse the calls anyway.
    expect(license.isLicensed).toBe(false);
    expect(license.loaded).toBe(true);
    expect(spy).toHaveBeenCalled();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    mockDb.setLicense("valid");
  });
});

describe("licence store — following the backend mid-session", () => {
  // Rust re-evaluates the licence on a timer, so a verdict can change under a
  // user who is mid-task. These cover the half of that fix that lives here: the
  // screen has to follow, or the app goes on claiming to be licensed while every
  // gated command refuses.

  beforeEach(() => {
    setActivePinia(createPinia());
    mockDb.setLicense("valid");
  });

  it("adopts a verdict the backend pushes, with no second load()", async () => {
    const license = useLicenseStore();
    await license.load();
    await license.watch();
    expect(license.isLicensed).toBe(true);

    // What the Rust watcher does when the expiry date passes.
    mockDb.setLicense("expired");

    expect(license.isLicensed).toBe(false);
    expect(license.status).toBe("expired");
    // The date has to come through too: `LicenseRequiredPanel` says *when* the
    // licence lapsed, and an empty date there reads as a bug.
    expect(license.info.expiredOn).toBe("2026-02-01");
  });

  it("unlocks again when a licence is installed elsewhere in the session", async () => {
    mockDb.setLicense("missing");
    const license = useLicenseStore();
    await license.load();
    await license.watch();

    mockDb.setLicense("valid");
    expect(license.isLicensed).toBe(true);
  });

  it("subscribes once however often watch() is called", async () => {
    const license = useLicenseStore();
    const spy = vi.spyOn(mockDb, "onLicenseChanged");

    // Both in the same tick, before the first has resolved — the case a guard on
    // the unsubscribe handle would miss.
    await Promise.all([license.watch(), license.watch()]);
    await license.watch();

    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("stops following once unwatched", async () => {
    const license = useLicenseStore();
    await license.load();
    await license.watch();
    license.unwatch();

    mockDb.setLicense("expired");
    expect(license.isLicensed).toBe(true);
  });

  it("keeps the current verdict when the subscription cannot be set up", async () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    vi.spyOn(mockDb, "onLicenseChanged").mockImplementation(() => {
      throw new Error("no event bridge");
    });

    const license = useLicenseStore();
    await license.load();
    await license.watch();

    // Deliberately *not* fail-closed: a missing listener costs a stale screen
    // until the next launch, and the Rust gate still refuses on its own.
    // Locking a paying shop out over it would be the worse failure.
    expect(license.isLicensed).toBe(true);
    expect(spy).toHaveBeenCalled();
  });

  afterEach(() => {
    // Handlers outlive the store instance otherwise: `mockDb` is a module
    // singleton shared by every test in the run.
    useLicenseStore().unwatch();
    vi.restoreAllMocks();
    mockDb.setLicense("valid");
  });
});

describe("licence store — importing", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    mockDb.setLicense("missing");
  });

  it("adopts the new verdict after a successful import", async () => {
    const license = useLicenseStore();
    await license.load();
    expect(license.isLicensed).toBe(false);

    await license.importFrom("/tmp/licence.json");
    expect(license.isLicensed).toBe(true);
  });

  it("rejects a file that is not a licence and leaves the current state alone", async () => {
    const license = useLicenseStore();
    await license.load();

    await expect(license.importFrom("/tmp/holiday-photo.png")).rejects.toThrow(/^INVALID_LICENSE:/);
    expect(license.isLicensed).toBe(false);
  });

  afterEach(() => {
    mockDb.setLicense("valid");
  });
});
