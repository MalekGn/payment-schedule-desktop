// Unit tests for the money/number/date formatting every screen renders through.
//
// Two contracts worth pinning. `formatDatePattern` is pure and does a naive
// token substitution, so its interesting behaviour is the *failure* paths — what
// it does with a missing date and with one it cannot parse, since both reach it
// from the database rather than from a picker. And `useFormat`'s three functions
// read the settings store at call time rather than closing over it, which is the
// only reason changing the currency in Paramètres updates figures already on
// screen.
//
// Grouping separators are normalised before comparison: `fr-FR` groups with a
// narrow no-break space, and hardcoding that byte makes the test fail on a
// different ICU build rather than on a real regression. `\s` matches every such
// space, so the class does not need to name them.

import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { formatDatePattern, useFormat } from "@/composables/useFormat";
import { useSettingsStore } from "@/stores/settings";

/** Collapse every kind of space so the assertions survive an ICU update. */
const norm = (s: string) => s.replace(/\s+/g, " ");

describe("formatDatePattern — the four patterns Paramètres offers", () => {
  it("renders each one with a zero-padded day and month", () => {
    expect(formatDatePattern("2026-08-04", "dd/MM/yyyy")).toBe("04/08/2026");
    expect(formatDatePattern("2026-08-04", "MM/dd/yyyy")).toBe("08/04/2026");
    expect(formatDatePattern("2026-08-04", "yyyy-MM-dd")).toBe("2026-08-04");
    expect(formatDatePattern("2026-08-04", "dd-MM-yyyy")).toBe("04-08-2026");
  });

  it("pads a single-digit day and month", () => {
    expect(formatDatePattern("2026-01-05", "dd/MM/yyyy")).toBe("05/01/2026");
  });
});

describe("formatDatePattern — values that are not a date", () => {
  it("shows a dash for a missing date rather than an empty cell", () => {
    // `paidDate` is null on every unpaid installment, so this is the common
    // case, not an edge one.
    expect(formatDatePattern(null, "dd/MM/yyyy")).toBe("—");
    expect(formatDatePattern(undefined, "dd/MM/yyyy")).toBe("—");
    expect(formatDatePattern("", "dd/MM/yyyy")).toBe("—");
  });

  it("returns the raw value when it cannot be read as a date", () => {
    // Better to surface the stored text than to invent a date from it.
    expect(formatDatePattern("tomorrow", "dd/MM/yyyy")).toBe("tomorrow");
    expect(formatDatePattern("2026-00-04", "dd/MM/yyyy")).toBe("2026-00-04");
  });
});

describe("useFormat — reacting to the settings store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("appends the configured currency, and follows a change to it", () => {
    const settings = useSettingsStore();
    const fmt = useFormat();

    settings.settings.currencyCode = "TND";
    expect(norm(fmt.money(2400))).toBe("2 400 TND");

    // The same `fmt` object: figures already rendered on screen must update
    // when the shop switches currency, which is why these read at call time.
    settings.settings.currencyCode = "EUR";
    expect(norm(fmt.money(2400))).toBe("2 400 EUR");
  });

  it("treats a missing amount as zero rather than rendering NaN", () => {
    useSettingsStore().settings.currencyCode = "TND";
    const fmt = useFormat();
    expect(norm(fmt.money(null))).toBe("0 TND");
    expect(norm(fmt.money(undefined))).toBe("0 TND");
    expect(fmt.number(null)).toBe("0");
  });

  it("groups thousands without a currency code for bare numbers", () => {
    setActivePinia(createPinia());
    const fmt = useFormat();
    expect(norm(fmt.number(1234567))).toBe("1 234 567");
  });

  it("shows no fraction digits, because money is whole units", () => {
    const fmt = useFormat();
    // The backend stores whole currency units; a decimal point here would
    // suggest a precision the data does not have.
    expect(fmt.number(1500)).not.toContain(".");
    expect(fmt.number(1500)).not.toContain(",");
  });

  it("formats dates with the configured pattern, and follows a change", () => {
    const settings = useSettingsStore();
    const fmt = useFormat();

    settings.settings.dateFormat = "dd/MM/yyyy";
    expect(fmt.date("2026-08-04")).toBe("04/08/2026");

    settings.settings.dateFormat = "yyyy-MM-dd";
    expect(fmt.date("2026-08-04")).toBe("2026-08-04");
  });

  it("uses Latin digits in Arabic, so figures stay comparable across languages", () => {
    const settings = useSettingsStore();
    const fmt = useFormat();
    settings.settings.language = "ar";
    settings.settings.currencyCode = "TND";
    // The locale tag carries `-u-nu-latn` precisely for this; without it the
    // same purchase would read ٢٤٠٠ and stop being comparable at a glance with
    // the French and English screens. Asserted as "no Arabic-Indic digits"
    // rather than on the grouping separator, which ar-TN renders as "." — the
    // separator is ICU's business, the numbering system is ours.
    const arabic = norm(fmt.money(2400));
    expect(arabic).toMatch(/[0-9]/);
    expect(arabic).not.toMatch(/[\u0660-\u0669\u06f0-\u06f9]/);
    expect(arabic.replace(/\D/g, "")).toBe("2400");
  });
});
