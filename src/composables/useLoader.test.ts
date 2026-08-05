// Unit tests for the fetch wrapper every list view loads through.
//
// This composable exists because of one specific bug: the hand-rolled versions
// it replaced set `loading = true`, awaited, and set it false on the success
// path only — so any failed load left the spinner turning forever with no
// message and no way back. The `finally` here is the fix, and it is exactly the
// kind of thing that silently regresses, so the rejection path is what these
// tests are for.
//
// It needs `useI18n` and Pinia, so it is driven through a throwaway host
// component rather than called directly — the alternative would be extracting a
// pure helper, but there is no decision to extract here, only wiring.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { defineComponent } from "vue";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import { useLoader } from "@/composables/useLoader";
import { useUiStore } from "@/stores/ui";

/** Mount the composable in a host so `useI18n` has a component instance. */
function harness(load: () => Promise<void>) {
  let loader!: ReturnType<typeof useLoader>;
  const Host = defineComponent({
    setup() {
      loader = useLoader(load);
      return () => null;
    },
  });
  const wrapper = mount(Host, { global: { plugins: [i18n] } });
  return { loader, wrapper };
}

describe("useLoader — a load that succeeds", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("starts out loading, so the first paint shows a spinner not an empty list", () => {
    const { loader } = harness(async () => {});
    expect(loader.loading.value).toBe(true);
    expect(loader.error.value).toBe("");
  });

  it("stops loading and reports no error", async () => {
    const { loader } = harness(async () => {});
    await loader.run();
    expect(loader.loading.value).toBe(false);
    expect(loader.error.value).toBe("");
    expect(useUiStore().toasts).toHaveLength(0);
  });
});

describe("useLoader — a load that fails", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("stops loading, which is the regression this composable exists to prevent", async () => {
    const { loader } = harness(async () => {
      throw new Error("PURCHASE_NOT_FOUND");
    });
    await loader.run();
    // Without the `finally`, this stayed true and the screen span forever.
    expect(loader.loading.value).toBe(false);
  });

  it("surfaces a localized sentence rather than the raw error", async () => {
    const { loader } = harness(async () => {
      throw new Error("PURCHASE_NOT_FOUND");
    });
    await loader.run();

    expect(loader.error.value).toBe(i18n.global.t("errors.purchaseNotFound"));
    // The original still reaches the console for diagnosis.
    expect(console.error).toHaveBeenCalled();
  });

  it("falls back to the generic sentence for a code it does not know", async () => {
    const { loader } = harness(async () => {
      throw new Error("WHAT_IS_THIS");
    });
    await loader.run();
    // Never a dotted key path in front of a shopkeeper.
    expect(loader.error.value).toBe(i18n.global.t("errors.generic"));
  });

  it("raises the failure as a toast as well as inline", async () => {
    const { loader } = harness(async () => {
      throw new Error("PURCHASE_NOT_FOUND");
    });
    await loader.run();

    const ui = useUiStore();
    // Inline for the retry button, toast because the list area may be scrolled
    // out of view.
    expect(ui.toasts).toHaveLength(1);
    expect(ui.toasts[0].kind).toBe("error");
    expect(ui.toasts[0].message).toBe(loader.error.value);
  });

  it("clears the previous error when the user retries", async () => {
    let fail = true;
    const { loader } = harness(async () => {
      if (fail) throw new Error("PURCHASE_NOT_FOUND");
    });

    await loader.run();
    expect(loader.error.value).not.toBe("");

    fail = false;
    await loader.run();
    // A stale message beside fresh data is worse than no message.
    expect(loader.error.value).toBe("");
    expect(loader.loading.value).toBe(false);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });
});
