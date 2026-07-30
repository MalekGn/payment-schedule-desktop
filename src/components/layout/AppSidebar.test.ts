// Unit tests for the two parts of the sidebar that depend on outside state:
// the brand block beside the logo, and the "new purchase" shortcut.
//
// The name in the brand block is the licence's `licensee`, not a user-typed
// setting: it is only populated once the signature has verified, so it is
// attested. The three branches below (licence → stored setting → generic app
// title) are the whole contract, and the expired case pins down that a shop
// whose licence has lapsed still sees its own name rather than being demoted to
// the app title.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import AppSidebar from "@/components/layout/AppSidebar.vue";
import { useLicenseStore } from "@/stores/license";
import { useSettingsStore } from "@/stores/settings";
import type { License, LicenseStatusTag } from "@/types/models";

// Hoisted so the `vi.mock` factory can close over it: each test sets the route
// name before mounting, the way the real reactive route would read at setup.
const { currentRoute } = vi.hoisted(() => ({ currentRoute: { name: "dashboard" } }));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push: vi.fn() }),
  useRoute: () => currentRoute,
}));

const LICENCE: License = {
  licenseId: "PS-2026-0001",
  licensee: "Électro Sfax SARL",
  issuedAt: "2026-01-15",
  expiresAt: "2030-01-15",
  machineId: null,
  features: ["*"],
};

/** Seed the route and the two stores the sidebar reads, then mount. */
function render(
  opts: {
    status?: LicenseStatusTag;
    license?: License | null;
    shopName?: string;
    routeName?: string;
  } = {},
) {
  currentRoute.name = opts.routeName ?? "dashboard";

  const license = useLicenseStore();
  license.info = {
    status: opts.status ?? "missing",
    license: opts.license ?? null,
    expiredOn: null,
    machineId: null,
  };
  useSettingsStore().settings.shopName = opts.shopName ?? "";

  return mount(AppSidebar, {
    global: {
      plugins: [i18n],
      stubs: { RouterLink: { template: "<a><slot /></a>" } },
    },
  });
}

describe("AppSidebar brand", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("shows the licensee from a valid licence, not the app title", () => {
    const wrapper = render({ status: "valid", license: LICENCE, shopName: "Ancien nom" });
    const name = wrapper.find(".brand-name");
    expect(name.text()).toBe("Électro Sfax SARL");
    // The full value stays reachable when the two-line clamp truncates it.
    expect(name.attributes("title")).toBe("Électro Sfax SARL");
    expect(wrapper.find(".brand-line1").exists()).toBe(false);
  });

  it("keeps showing the licensee when the licence has expired", () => {
    // `license` is carried on Expired too — the signature verified, so the name
    // is still attested and losing it would be a gratuitous downgrade.
    const wrapper = render({ status: "expired", license: LICENCE });
    expect(wrapper.find(".brand-name").text()).toBe("Électro Sfax SARL");
  });

  it("falls back to the stored shop name when no licence is readable", () => {
    const wrapper = render({ status: "missing", shopName: "Électro Ménager" });
    expect(wrapper.find(".brand-name").text()).toBe("Électro Ménager");
  });

  it("falls back to the app title when neither source has a name", () => {
    const wrapper = render({ status: "missing", shopName: "   " });
    expect(wrapper.find(".brand-name").exists()).toBe(false);
    expect(wrapper.find(".brand-line1").text()).toBe(i18n.global.t("app.title"));
    expect(wrapper.find(".brand-line2").text()).toBe(i18n.global.t("app.titleLine2"));
  });
});

describe("AppSidebar new-purchase shortcut", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("is hidden on the Achats page, which has its own button", () => {
    expect(render({ routeName: "achats" }).find(".new-purchase").exists()).toBe(false);
  });

  it("is shown everywhere else, including a purchase's detail page", () => {
    expect(render({ routeName: "dashboard" }).find(".new-purchase").exists()).toBe(true);
    expect(render({ routeName: "achat-detail" }).find(".new-purchase").exists()).toBe(true);
  });
});
