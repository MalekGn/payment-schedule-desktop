// Licence state for the UI.
//
// This store decides what the user *sees*. It does not decide what they can
// *do*: the gated Tauri commands refuse on their own in Rust, because the
// renderer is a WebView the user controls and a `v-if` is not a control. Treat
// everything here as presentation.

import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { api } from "@/api";
import type { LicenseInfo, LicenseStatusTag } from "@/types/models";

/** What the app assumes before the first check completes, and if it fails. */
const UNKNOWN: LicenseInfo = {
  status: "missing",
  license: null,
  expiredOn: null,
  machineId: null,
};

export const useLicenseStore = defineStore("license", () => {
  const info = ref<LicenseInfo>({ ...UNKNOWN });
  const loaded = ref(false);

  const status = computed<LicenseStatusTag>(() => info.value.status);
  /** Only a verified, in-date, right-machine licence unlocks the app. */
  const isLicensed = computed(() => info.value.status === "valid");
  const license = computed(() => info.value.license);
  /** This machine's fingerprint — the value a customer reports to buy a licence. */
  const machineId = computed(() => info.value.machineId);

  async function load() {
    // Fail closed: if the check itself breaks, the app presents as unlicensed
    // rather than granting access it could not verify. The Rust gate would
    // refuse the calls anyway, so pretending otherwise would only produce
    // buttons that error when pressed.
    try {
      info.value = await api.getLicenseStatus();
    } catch (e) {
      console.error("licence check failed; treating this install as unlicensed:", e);
      info.value = { ...UNKNOWN };
    } finally {
      loaded.value = true;
    }
  }

  /**
   * Install a licence file the user picked. Rejects with `INVALID_LICENSE:{status}`
   * when the file is not valid for this machine, leaving the current licence in
   * place — callers surface that through `toUserMessage`.
   */
  async function importFrom(sourcePath: string) {
    info.value = await api.importLicense(sourcePath);
  }

  return { info, loaded, status, isLicensed, license, machineId, load, importFrom };
});
