// Cross-language parity: `src/lib/finance.ts` must agree with
// `src-tauri/src/db.rs` on every case in tests/fixtures/finance-parity.json.
//
// The two implementations of the installment math are independent, and
// CLAUDE.md treats a divergence between them as a blocker — a mismatch means
// the schedule a user is shown while creating a purchase is not the schedule
// that gets written to the database. Both this file and the Rust test
// `finance_parity_fixture` in `db.rs` read the *same* fixture, so changing one
// implementation without the other fails a test instead of drifting silently.
//
// Regenerate the fixture only when the intended behaviour changes, and update
// both suites in the same commit.

import { describe, expect, it } from "vitest";

import fixture from "../../tests/fixtures/finance-parity.json";
import { addInterval, installmentStatus, splitAmounts } from "./finance";
import type { InstallmentStatus, IntervalKind } from "@/types/models";

describe("finance.ts ↔ db.rs parity", () => {
  it("has cases for every shared function", () => {
    expect(fixture.splitAmounts.length).toBeGreaterThan(0);
    expect(fixture.addInterval.length).toBeGreaterThan(0);
    expect(fixture.installmentStatus.length).toBeGreaterThan(0);
  });

  it.each(fixture.splitAmounts)("splitAmounts($total, $n)", ({ total, n, expected }) => {
    expect(splitAmounts(total, n)).toEqual(expected);
    // The invariant the split exists to preserve: money is integer, and the
    // parts must reconstruct the total exactly.
    if (n > 0) {
      expect(expected.reduce((s, v) => s + v, 0)).toBe(total);
      expect(expected.every(Number.isInteger)).toBe(true);
    }
  });

  it.each(fixture.addInterval)(
    "addInterval($date, $kind, $intervalDays, $k)",
    ({ date, kind, intervalDays, k, expected }) => {
      expect(addInterval(date, kind as IntervalKind, intervalDays, k)).toBe(expected);
    },
  );

  it.each(fixture.installmentStatus)(
    "installmentStatus($amount, $paid, $dueDate @ $today)",
    ({ amount, paid, dueDate, today, expected }) => {
      expect(installmentStatus(amount, paid, dueDate, today)).toBe(expected as InstallmentStatus);
    },
  );
});
