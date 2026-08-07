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

  /** Adopt a verdict, whether we asked for it or the backend pushed it. */
  function apply(next: LicenseInfo) {
    info.value = next;
    loaded.value = true;
  }

  async function load() {
    // Fail closed: if the check itself breaks, the app presents as unlicensed
    // rather than granting access it could not verify. The Rust gate would
    // refuse the calls anyway, so pretending otherwise would only produce
    // buttons that error when pressed.
    try {
      apply(await api.getLicenseStatus());
    } catch (e) {
      console.error("licence check failed; treating this install as unlicensed:", e);
      apply({ ...UNKNOWN });
    }
  }

  // In flight or settled, never reset — the guard `watch()` uses. Checking the
  // unsubscribe handle instead would not hold: it is still null while the
  // subscription is being set up, so two calls in the same tick would both get
  // through and only one handle would survive to be unsubscribed.
  let subscription: Promise<void> | null = null;
  let unsubscribe: (() => void) | null = null;

  /**
   * Register the subscription. Resolves to whether it is live.
   *
   * `async`, so a gateway that throws synchronously — the browser mock does,
   * because `Promise.resolve(mockDb.…())` evaluates its argument first — is
   * caught here just like a rejected `listen`.
   */
  async function subscribe(): Promise<boolean> {
    try {
      unsubscribe = await api.onLicenseChanged(apply);
      return true;
    } catch (e) {
      // Not fatal and not fail-closed: losing the subscription costs a stale
      // screen until the next launch, and the Rust gate still refuses. Failing
      // closed here would lock a licensed shop out over a missing listener.
      console.error("could not watch for licence changes; the screen may go stale:", e);
      return false;
    }
  }

  /**
   * Start following verdicts the backend pushes.
   *
   * The licence is re-evaluated in Rust while the app runs, so an expiry takes
   * effect without a restart. This is the half that keeps the screen honest:
   * without it the UI would go on showing a licensed install while every gated
   * command refused. Idempotent, and safe to leave unawaited.
   */
  function watch(): Promise<void> {
    // The reset lives in `.then`, not inside `subscribe`, so it runs as a
    // microtask — after the assignment below. Clearing `subscription` while
    // `subscribe()` is still on the stack would be undone by the `??=` that has
    // not run yet, memoizing a failed attempt as a live subscription.
    subscription ??= subscribe().then((live) => {
      if (!live) subscription = null;
    });
    return subscription;
  }

  /** Stop following. Exists so tests — and any future teardown — can let go. */
  function unwatch() {
    unsubscribe?.();
    unsubscribe = null;
    subscription = null;
  }

  /**
   * Install a licence file the user picked. Rejects with `INVALID_LICENSE:{status}`
   * when the file is not valid for this machine, leaving the current licence in
   * place — callers surface that through `toUserMessage`.
   */
  async function importFrom(sourcePath: string) {
    apply(await api.importLicense(sourcePath));
  }

  return {
    info,
    loaded,
    status,
    isLicensed,
    license,
    machineId,
    load,
    importFrom,
    watch,
    unwatch,
  };
});
