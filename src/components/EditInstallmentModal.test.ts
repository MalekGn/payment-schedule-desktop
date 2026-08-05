// Unit tests for the tranche editor's payload builder and its two locks.
//
// `save()` assembles a deliberately *sparse* `InstallmentEdit`: an omitted field
// means "leave it alone", so every field it does send is a decision. Getting
// that wrong is silent — the call succeeds, and the wrong thing changes. That is
// what these tests pin, by asserting the exact object handed to
// `api.updateInstallment` for each shape of edit.
//
// The single most important assertion here is the negative one: `amount` and
// `dueDate` are never sent. The backend refuses them (`SCHEDULE_VIA_PURCHASE`)
// so a regression would surface as a broken save rather than corrupt data — but
// it would surface to a shopkeeper, not to us.
//
// The backend contract itself is covered by tests/integration/installment-edit
// and by cargo test; what is only reachable from here is the component's own
// gating — which fields it decides to send at all.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import EditInstallmentModal from "@/components/EditInstallmentModal.vue";
import { api } from "@/api";
import type { Installment } from "@/types/models";

const inst = (o: Partial<Installment> = {}): Installment => ({
  id: 41,
  purchaseId: 7,
  index: 2,
  amount: 250,
  dueDate: "2026-02-15",
  paidAmount: 0,
  paidDate: null,
  status: "pending",
  ...o,
});

/** Tranche 1 settled, tranche 2 the one under edit — the ordinary case. */
const SIBLINGS: Installment[] = [
  inst({ id: 40, index: 1, paidAmount: 250, paidDate: "2026-01-20", status: "paid" }),
  inst(),
];

function render(installment: Installment, siblings: Installment[] = SIBLINGS) {
  return mount(EditInstallmentModal, {
    props: { installment, siblings, installmentCount: 4, purchaseReference: "A-000007" },
    global: {
      plugins: [i18n],
      // DatePicker teleports to <body> and measures with getBoundingClientRect,
      // which jsdom reports as all zeros. Its own behaviour is not under test.
      stubs: { DatePicker: { template: "<div class='date-stub' />" }, Teleport: true },
    },
  });
}

/** The payload a save handed to the gateway. */
async function saved(wrapper: ReturnType<typeof render>) {
  const spy = vi.spyOn(api, "updateInstallment");
  await wrapper.find(".btn--primary").trigger("click");
  await flushPromises();
  return spy.mock.calls[0];
}

describe("EditInstallmentModal — what a save actually sends", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("never sends the schedule, which is the purchase editor's to own", async () => {
    const wrapper = render(inst());
    await wrapper.find("#inst-paid").setValue("100");

    const [, edit] = (await saved(wrapper)) ?? [];
    // The invariant the whole schedule/money split rests on.
    expect(edit).not.toHaveProperty("amount");
    expect(edit).not.toHaveProperty("dueDate");
  });

  it("sends the collected figure with a date for the entry it creates", async () => {
    const wrapper = render(inst());
    await wrapper.find("#inst-paid").setValue("100");

    const [id, edit] = (await saved(wrapper)) ?? [];
    expect(id).toBe(41);
    expect(edit).toMatchObject({ paidAmount: 100 });
    // A date is only meaningful as the date of the entry this save writes.
    expect(edit).toHaveProperty("paymentDate");
  });

  it("omits the note when the field was left empty", async () => {
    const wrapper = render(inst());
    await wrapper.find("#inst-paid").setValue("100");

    const [, edit] = (await saved(wrapper)) ?? [];
    expect(edit).not.toHaveProperty("note");
  });

  it("sends a trimmed note alongside the figure", async () => {
    const wrapper = render(inst());
    await wrapper.find("#inst-paid").setValue("100");
    await wrapper.find("#inst-note").setValue("   chèque   ");

    const [, edit] = (await saved(wrapper)) ?? [];
    expect(edit).toMatchObject({ note: "chèque" });
  });

  it("sends nothing at all when the money half is locked", async () => {
    // Tranche 2 while tranche 1 is still owing: cash is collected in order.
    const owing = [inst({ id: 40, index: 1, paidAmount: 0, status: "late" }), inst()];
    const wrapper = render(inst(), owing);

    const [, edit] = (await saved(wrapper)) ?? [];
    expect(edit).toEqual({});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });
});

describe("EditInstallmentModal — the locks it shows", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("disables the money fields while the previous tranche is owing", () => {
    const owing = [inst({ id: 40, index: 1, paidAmount: 0, status: "late" }), inst()];
    const wrapper = render(inst(), owing);

    expect(wrapper.find("fieldset").attributes("disabled")).toBeDefined();
    expect(wrapper.find(".lock-note").text()).toBe(
      i18n.global.t("achats.installmentEdit.lockedByPrevious", { index: 1 }),
    );
  });

  it("leaves the money fields open once the previous tranche is settled", () => {
    const wrapper = render(inst());
    expect(wrapper.find("#inst-paid").attributes("disabled")).toBeUndefined();
  });

  it("shows the amount and due date read-only, pointing at the purchase editor", () => {
    const wrapper = render(inst());
    // They are shown — the shopkeeper still needs to see what is owed — but as
    // figures, not inputs.
    expect(wrapper.find("#inst-amount").exists()).toBe(false);
    expect(wrapper.find(".edit-info").text()).toContain("250");
    expect(wrapper.find(".edit-note").text()).toBe(
      i18n.global.t("achats.installmentEdit.scheduleElsewhere"),
    );
  });

  it("refuses to collect more than the tranche is worth, before any round trip", async () => {
    const wrapper = render(inst());
    await wrapper.find("#inst-paid").setValue("900");

    expect(wrapper.find(".field-error").text()).toBe(
      i18n.global.t("errors.paidAboveAmount", { amount: 250 }),
    );
    expect(wrapper.find(".btn--primary").attributes("disabled")).toBeDefined();
  });

  it("asks for confirmation before touching a settled tranche", async () => {
    const settled = inst({ paidAmount: 250, paidDate: "2026-02-20", status: "paid" });
    const wrapper = render(settled, [SIBLINGS[0], settled]);
    const spy = vi.spyOn(api, "updateInstallment");

    await wrapper.find("#inst-paid").setValue("180");
    await wrapper.find(".btn--primary").trigger("click");
    await flushPromises();

    // Correcting money already collected is a deliberate second step.
    expect(spy).not.toHaveBeenCalled();
    expect(wrapper.findComponent({ name: "ConfirmDialog" }).exists()).toBe(true);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });
});
