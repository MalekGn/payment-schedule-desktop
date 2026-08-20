// Unit tests for the document/window title the print dialog reads.
//
// Two behaviours matter. Setting the title is the fix for the reported bug —
// every document used to be offered as `output.pdf`. Restoring it is the
// regression guard: without the restore, walking out of a print route leaves the
// whole application window named after a client's receipt, and nobody notices
// until they glance at their taskbar.

import { describe, expect, it, beforeEach } from "vitest";
import { defineComponent, ref, type Ref } from "vue";
import { mount } from "@vue/test-utils";

import { mockDb } from "@/api/mock";
import { useDocumentTitle } from "@/composables/useDocumentTitle";

/** Mount a component that drives the title from `source`. */
function mountWith(source: Ref<string | null>) {
  return mount(
    defineComponent({
      setup() {
        useDocumentTitle(source);
        return () => null;
      },
    }),
  );
}

describe("useDocumentTitle", () => {
  beforeEach(() => {
    document.title = "paymentSchedule";
    mockDb.lastWindowTitle = null;
  });

  it("names the document, and the native window with it", async () => {
    const title = ref<string | null>("Echeancier-A-000001-Ali-Ben-Salah");
    mountWith(title);

    expect(document.title).toBe("Echeancier-A-000001-Ali-Ben-Salah");
    // The window rename crosses the gateway, so the mock is where it lands.
    await Promise.resolve();
    expect(mockDb.lastWindowTitle).toBe("Echeancier-A-000001-Ali-Ben-Salah");
  });

  it("leaves the title alone until there is something to say", () => {
    // `null` is the loading and not-found state. Renaming the window from
    // half-loaded data would flash a wrong name, or a bare prefix.
    mountWith(ref<string | null>(null));
    expect(document.title).toBe("paymentSchedule");
  });

  it("follows the title as the document resolves", async () => {
    const title = ref<string | null>(null);
    mountWith(title);
    expect(document.title).toBe("paymentSchedule");

    title.value = "Releve-Ali-Ben-Salah-2026-08-20";
    await Promise.resolve();
    expect(document.title).toBe("Releve-Ali-Ben-Salah-2026-08-20");
  });

  it("restores the original title when the document is left", async () => {
    const title = ref<string | null>("Recu-A-000001-T2-2026-08-20");
    const wrapper = mountWith(title);
    expect(document.title).toBe("Recu-A-000001-T2-2026-08-20");

    wrapper.unmount();
    await Promise.resolve();
    expect(document.title).toBe("paymentSchedule");
    expect(mockDb.lastWindowTitle).toBe("paymentSchedule");
  });
});
