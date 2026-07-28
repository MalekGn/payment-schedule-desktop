// Unit tests for the Recent purchases card's layout contract.
//
// The card lives in the dashboard's narrow grid track, so its table must stay
// inside the card: it is wrapped in a `.table-scroll` container (src/style.css)
// and the product column truncates rather than widening the table. CSS itself
// isn't exercised in jsdom, so what these tests pin down is the markup the CSS
// hangs off: the wrapper, the `.ellipsis` product cell, and its `title`
// fallback for the truncated text.

import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia } from "pinia";
import { i18n } from "@/i18n";
import RecentPurchasesCard from "@/components/dashboard/RecentPurchasesCard.vue";
import type { PurchaseSummary } from "@/types/models";

vi.mock("vue-router", () => ({
  useRouter: () => ({ push: vi.fn() }),
}));

const make = (o: Partial<PurchaseSummary> = {}): PurchaseSummary => ({
  id: 1,
  reference: "A-000007",
  clientId: 1,
  clientName: "Mohamed Trabelsi",
  productLabel: "Réfrigérateur Samsung RT38 No Frost Inox",
  totalPrice: 2400,
  paidAmount: 400,
  remaining: 2000,
  installmentCount: 6,
  purchaseDate: "2026-07-26",
  status: "in_progress",
  overdueCount: 0,
  archivedAt: null,
  ...o,
});

function render(purchases: PurchaseSummary[]) {
  return mount(RecentPurchasesCard, {
    props: { purchases },
    global: {
      plugins: [createPinia(), i18n],
      stubs: { RouterLink: { template: "<a><slot /></a>" } },
    },
  });
}

describe("RecentPurchasesCard layout", () => {
  it("keeps the table inside a .table-scroll container", () => {
    const wrapper = render([make()]);
    const scroll = wrapper.find(".table-scroll");
    expect(scroll.exists()).toBe(true);
    expect(scroll.find("table.recent-table").exists()).toBe(true);
  });

  it("marks the product cell as the truncating column and keeps the full text in a title", () => {
    const p = make();
    const cell = render([p]).find("td.ellipsis");
    expect(cell.exists()).toBe(true);
    expect(cell.text()).toBe(p.productLabel);
    expect(cell.attributes("title")).toBe(p.productLabel);
  });

  it("renders one row per purchase", () => {
    const wrapper = render([make({ id: 1 }), make({ id: 2, reference: "A-000006" })]);
    expect(wrapper.findAll("tbody tr")).toHaveLength(2);
  });

  it("shows the empty state and no table (nor wrapper) when there is nothing to list", () => {
    const wrapper = render([]);
    expect(wrapper.find("table").exists()).toBe(false);
    expect(wrapper.find(".table-scroll").exists()).toBe(false);
    expect(wrapper.text()).toContain(i18n.global.t("dashboard.empty.purchases"));
  });
});
