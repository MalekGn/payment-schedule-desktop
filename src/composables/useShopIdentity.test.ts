// Unit tests for the shop identity the sidebar and every printed document share.
//
// The fallback chain is the whole point. It used to exist only inline in
// `AppSidebar.vue`, and the printed letterhead needs the same answer — two
// copies is how the sidebar and a contract handed to a client end up naming the
// shop differently. These pin the order and the empty cases.

import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";

import { useShopIdentity } from "@/composables/useShopIdentity";
import { useLicenseStore } from "@/stores/license";
import { useSettingsStore } from "@/stores/settings";

/** A minimal licence payload; only `licensee` matters to this composable. */
function licenceNamed(licensee: string) {
  return {
    licenseId: "PS-TEST",
    licensee,
    issuedAt: "2026-01-01",
    expiresAt: "2030-01-01",
    machineId: null,
    features: [],
  };
}

describe("useShopIdentity — which name the shop goes by", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("prefers the licence, because that name is vendor-attested", () => {
    const license = useLicenseStore();
    const settings = useSettingsStore();
    license.info.license = licenceNamed("Électro Sfax");
    settings.settings.shopName = "typed by the user";

    expect(useShopIdentity().shopName.value).toBe("Électro Sfax");
  });

  it("falls back to the stored setting when no licence is readable", () => {
    useSettingsStore().settings.shopName = "Boutique Ben Ali";
    expect(useShopIdentity().shopName.value).toBe("Boutique Ben Ali");
  });

  it("does not let a whitespace-only licence name defeat the fallback", () => {
    // A licence whose `licensee` is blank is still a *valid* licence, so the
    // chain has to trim before deciding — otherwise the letterhead renders an
    // empty name rather than the one the shop typed.
    const license = useLicenseStore();
    license.info.license = licenceNamed("   ");
    useSettingsStore().settings.shopName = "Boutique Ben Ali";

    expect(useShopIdentity().shopName.value).toBe("Boutique Ben Ali");
  });

  it("reports an empty name rather than inventing one", () => {
    // Callers decide what to show instead: the sidebar and the letterhead both
    // fall back to the app title, which is a template concern, not this one's.
    expect(useShopIdentity().shopName.value).toBe("");
  });

  it("trims the contact block, so a stray newline is not a letterhead line", () => {
    useSettingsStore().settings.shopInfo = "\n  12 rue de Tunis\n  71 000 000\n ";
    expect(useShopIdentity().shopInfo.value).toBe("12 rue de Tunis\n  71 000 000");
  });

  it("reports no logo when none is configured", () => {
    expect(useShopIdentity().logoSrc.value).toBeNull();
    useSettingsStore().settings.logoPath = "data:image/png;base64,AAAA";
    // Outside Tauri the stored value is already a usable src; `resolveLogoSrc`
    // only rewrites it through the asset protocol on the desktop path.
    expect(useShopIdentity().logoSrc.value).toBe("data:image/png;base64,AAAA");
  });
});
