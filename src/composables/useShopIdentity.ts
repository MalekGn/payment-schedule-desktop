// Who the documents and the sidebar say the shop is.
//
// Extracted from `AppSidebar.vue`, which had the only copy. The printed
// documents need exactly the same answer, and two copies of a fallback chain is
// how the sidebar and the letterhead end up disagreeing about the shop's name.

import { computed, type ComputedRef } from "vue";

import { resolveLogoSrc } from "@/lib/assets";
import { useLicenseStore } from "@/stores/license";
import { useSettingsStore } from "@/stores/settings";

export interface ShopIdentity {
  /**
   * The shop's name, or `""` when nothing is known — callers decide what to
   * show instead (the sidebar falls back to the app title).
   *
   * The licence is the source of truth: `license.license` is populated only once
   * the signature has verified — including for an expired or wrong-machine
   * licence — so the name it carries is vendor-attested rather than user-typed.
   * The stored setting is the fallback for an install with no readable licence.
   */
  shopName: ComputedRef<string>;
  /** Free-form contact block from Settings, or `""`. */
  shopInfo: ComputedRef<string>;
  /** Logo as an `<img src>`, or `null` when none is configured. */
  logoSrc: ComputedRef<string | null>;
}

/** Must be called from `setup()` — it reads two Pinia stores. */
export function useShopIdentity(): ShopIdentity {
  const license = useLicenseStore();
  const settings = useSettingsStore();

  return {
    shopName: computed(() => license.license?.licensee.trim() || settings.shopName.trim()),
    shopInfo: computed(() => settings.shopInfo.trim()),
    logoSrc: computed(() => resolveLogoSrc(settings.logoPath)),
  };
}
