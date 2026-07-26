import { describe, it, expect } from "vitest";
import { addInterval, dayDiff, installmentStatus, purchaseStatus, splitAmounts } from "./finance";

describe("splitAmounts", () => {
  it("splits evenly when divisible", () => {
    expect(splitAmounts(2400, 6)).toEqual([400, 400, 400, 400, 400, 400]);
  });

  it("puts the remainder on the last installment", () => {
    const parts = splitAmounts(1000, 3);
    expect(parts).toEqual([333, 333, 334]);
    expect(parts.reduce((a, b) => a + b, 0)).toBe(1000);
  });

  it("handles n = 1", () => {
    expect(splitAmounts(999, 1)).toEqual([999]);
  });

  it("returns empty for non-positive n", () => {
    expect(splitAmounts(500, 0)).toEqual([]);
  });
});

describe("addInterval", () => {
  it("adds months with the first installment on the purchase date (k=0)", () => {
    expect(addInterval("2026-01-15", "monthly", null, 0)).toBe("2026-01-15");
    expect(addInterval("2026-01-15", "monthly", null, 1)).toBe("2026-02-15");
  });

  it("clamps month-end overflow", () => {
    expect(addInterval("2026-01-31", "monthly", null, 1)).toBe("2026-02-28");
  });

  it("adds weeks and custom days", () => {
    expect(addInterval("2026-01-01", "weekly", null, 2)).toBe("2026-01-15");
    expect(addInterval("2026-01-01", "custom", 10, 3)).toBe("2026-01-31");
  });
});

describe("installmentStatus", () => {
  const today = "2026-07-16";
  it("is paid when fully covered", () => {
    expect(installmentStatus(400, 400, "2026-08-01", today)).toBe("paid");
    expect(installmentStatus(400, 450, "2026-01-01", today)).toBe("paid");
  });
  it("is late when past due with a balance", () => {
    expect(installmentStatus(400, 0, "2026-06-01", today)).toBe("late");
    expect(installmentStatus(400, 100, "2026-06-01", today)).toBe("late");
  });
  it("is partial when part-paid and not yet due", () => {
    expect(installmentStatus(400, 100, "2026-08-01", today)).toBe("partial");
  });
  it("is pending otherwise", () => {
    expect(installmentStatus(400, 0, "2026-08-01", today)).toBe("pending");
  });
});

describe("purchaseStatus", () => {
  it("is paid only when every installment is paid", () => {
    expect(purchaseStatus(["paid", "paid"], true)).toBe("paid");
  });
  it("is late when any installment is late", () => {
    expect(purchaseStatus(["paid", "late", "pending"], true)).toBe("late");
  });
  it("is in_progress when some paid but none late", () => {
    expect(purchaseStatus(["paid", "pending"], true)).toBe("in_progress");
  });
  it("is pending when nothing paid", () => {
    expect(purchaseStatus(["pending", "pending"], false)).toBe("pending");
  });
});

describe("dayDiff", () => {
  it("counts whole days", () => {
    expect(dayDiff("2026-07-16", "2026-07-10")).toBe(6);
    expect(dayDiff("2026-07-10", "2026-07-16")).toBe(-6);
  });
});
