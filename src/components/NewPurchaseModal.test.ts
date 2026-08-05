// Unit tests for the purchase form's schedule orchestration.
//
// The arithmetic underneath — `splitAmounts`, `rebalanceAmounts`, `addInterval`
// — is already covered by src/lib/finance.test.ts and cross-checked against the
// Rust implementation by the parity fixture. What has never been covered is the
// orchestration on top, which is where this component's real complexity lives:
// which rows are locked, when the schedule is regenerated and when it is left
// alone, and what a typed amount is rebalanced *against*.
//
// This form became the only place an installment's amount or due date can be
// changed, so a mistake here is not cosmetic — it is the shopkeeper unable to
// reschedule, or silently reshaping a tranche that has been paid.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { i18n } from "@/i18n";
import NewPurchaseModal from "@/components/NewPurchaseModal.vue";
import { api } from "@/api";
import type { Installment, PurchaseDetail } from "@/types/models";

vi.mock("vue-router", () => ({ useRouter: () => ({ push: vi.fn() }) }));

const inst = (o: Partial<Installment>): Installment => ({
  id: 1,
  purchaseId: 7,
  index: 1,
  amount: 250,
  dueDate: "2026-01-15",
  paidAmount: 0,
  paidDate: null,
  status: "pending",
  ...o,
});

/** A 1000-over-4 purchase; `settled` marks how many tranches are paid off. */
function detail(settled = 0): PurchaseDetail {
  const installments = [1, 2, 3, 4].map((n) =>
    inst({
      id: 40 + n,
      index: n,
      dueDate: `2026-0${n}-15`,
      ...(n <= settled
        ? { paidAmount: 250, paidDate: `2026-0${n}-20`, status: "paid" as const }
        : {}),
    }),
  );
  return {
    purchase: {
      id: 7,
      reference: "A-000007",
      clientId: 1,
      productLabel: "Réfrigérateur",
      totalPrice: 1000,
      installmentCount: 4,
      intervalKind: "monthly",
      intervalDays: null,
      purchaseDate: "2026-01-15",
      createdAt: "2026-01-15",
      archivedAt: null,
    },
    client: {
      id: 1,
      firstName: "Mohamed",
      lastName: "Trabelsi",
      phone: "+216 20 123 456",
      address: "Ariana",
      email: null,
      createdAt: "2026-01-01",
      archivedAt: null,
    },
    installments,
    totalPaid: settled * 250,
    remaining: 1000 - settled * 250,
    status: settled > 0 ? "in_progress" : "pending",
  };
}

async function render(purchase: PurchaseDetail | null = null) {
  const wrapper = mount(NewPurchaseModal, {
    props: { purchase },
    global: {
      plugins: [i18n],
      stubs: { DatePicker: { template: "<div class='date-stub' />" }, Teleport: true },
    },
  });
  // `onMounted` fetches the client list, so rows do not exist until it settles.
  await flushPromises();
  return wrapper;
}

const amounts = (w: Awaited<ReturnType<typeof render>>) =>
  w.findAll(".inst-amount").map((i) => Number((i.element as HTMLInputElement).value));

/** Type into row `i` and commit, which is what triggers the rebalance. */
async function typeAmount(w: Awaited<ReturnType<typeof render>>, i: number, value: number) {
  const input = w.findAll(".inst-amount")[i];
  await input.setValue(String(value));
  await input.trigger("change");
}

describe("NewPurchaseModal — creating", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("splits the total evenly, with the remainder on the last tranche", async () => {
    const wrapper = await render();
    await wrapper.find("#np-total").setValue("1000");
    await wrapper.find("#np-count").setValue("3");
    expect(amounts(wrapper)).toEqual([333, 333, 334]);
  });

  it("leaves every row unlocked, since nothing has been paid", async () => {
    const wrapper = await render();
    expect(wrapper.findAll(".inst-row.locked")).toHaveLength(0);
    expect(wrapper.find(".locked-note").exists()).toBe(false);
  });

  it("does not rebalance while creating — the sum check explains instead", async () => {
    const wrapper = await render();
    await wrapper.find("#np-total").setValue("1000");
    await wrapper.find("#np-count").setValue("2");
    await typeAmount(wrapper, 0, 600);
    // Free typing: the other row is left alone and the running sum goes red.
    expect(amounts(wrapper)).toEqual([600, 500]);
    expect(wrapper.find(".inst-sum").classes()).toContain("bad");
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });
});

describe("NewPurchaseModal — editing a purchase that has been paid into", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("locks a settled tranche and leaves the rest editable", async () => {
    const wrapper = await render(detail(1));
    const inputs = wrapper.findAll(".inst-amount");

    expect(inputs[0].attributes("disabled")).toBeDefined();
    expect(inputs[1].attributes("disabled")).toBeUndefined();
    expect(wrapper.find(".locked-note").text()).toBe(
      i18n.global.t("achats.form.settledRowsLocked", { count: 1 }),
    );
  });

  it("locks the anchor fields, which would regenerate every date", async () => {
    const wrapper = await render(detail(1));
    // Count, interval and purchase date rebuild the whole schedule, settled
    // rows included — so they go when any tranche is settled.
    expect(wrapper.find("#np-count").attributes("disabled")).toBeDefined();
    expect(wrapper.find("#np-interval").attributes("disabled")).toBeDefined();
    // The total is not one of them: the difference is absorbed by the rest.
    expect(wrapper.find("#np-total").attributes("disabled")).toBeUndefined();
  });

  it("holds the purchase total when a tranche is retyped", async () => {
    const wrapper = await render(detail(0));
    await typeAmount(wrapper, 1, 400);

    expect(amounts(wrapper).reduce((a, b) => a + b, 0)).toBe(1000);
    expect(wrapper.find(".inst-sum").classes()).toContain("ok");
  });

  it("never asks a settled tranche to absorb a change", async () => {
    const wrapper = await render(detail(1));
    await typeAmount(wrapper, 1, 400);

    expect(amounts(wrapper)[0]).toBe(250);
    expect(amounts(wrapper).reduce((a, b) => a + b, 0)).toBe(1000);
  });

  it("rebalances against the last agreed amounts, not the previous keystroke", async () => {
    // The `committed` snapshot exists for exactly this. Editing one row twice
    // must land where a single edit to the final value would: without it the
    // second edit would rebalance against a vector that already absorbed the
    // first, and the total would drift.
    const wrapper = await render(detail(0));
    await typeAmount(wrapper, 1, 400);
    await typeAmount(wrapper, 1, 500);

    const twice = amounts(wrapper);
    expect(twice.reduce((a, b) => a + b, 0)).toBe(1000);
    expect(twice[1]).toBe(500);

    const fresh = await render(detail(0));
    await typeAmount(fresh, 1, 500);
    expect(twice).toEqual(amounts(fresh));
  });

  it("re-splits only what is left after the settled tranches", async () => {
    const wrapper = await render(detail(2));
    await wrapper.find(".inst-head .btn").trigger("click");

    const out = amounts(wrapper);
    expect(out.slice(0, 2)).toEqual([250, 250]);
    // 500 left over the two rows still owed.
    expect(out.slice(2)).toEqual([250, 250]);
    expect(out.reduce((a, b) => a + b, 0)).toBe(1000);
  });

  it("refuses to push a tranche below what it has already collected", async () => {
    const partly = detail(0);
    partly.installments[0] = inst({ id: 41, index: 1, paidAmount: 100, status: "partial" });
    const wrapper = await render(partly);

    await typeAmount(wrapper, 0, 50);
    // rebalanceAmounts declines, so the typed figure stands and the sum line
    // says why rather than the form silently accepting it.
    expect(wrapper.find(".inst-sum").classes()).toContain("bad");
  });

  it("sends the displayed rows, with a matching count", async () => {
    const wrapper = await render(detail(0));
    const spy = vi.spyOn(api, "updatePurchase");

    await typeAmount(wrapper, 1, 400);
    await wrapper.find(".btn--primary").trigger("click");
    await flushPromises();

    const [id, input] = spy.mock.calls[0] ?? [];
    expect(id).toBe(7);
    // The backend refuses a list whose length disagrees with the count
    // (INSTALLMENT_COUNT_MISMATCH), so these must be derived together.
    expect(input?.installments).toHaveLength(input?.installmentCount ?? -1);
    expect(input?.installments?.reduce((s, i) => s + i.amount, 0)).toBe(input?.totalPrice);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });
});
