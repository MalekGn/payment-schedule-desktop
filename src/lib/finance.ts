// Pure installment/payment math — the TS counterpart of src-tauri/src/db.rs.
// Kept framework-free so it can be unit-tested and reused by the browser
// mock backend (src/api/mock.ts).

import type { InstallmentStatus, IntervalKind, PurchaseStatus } from "@/types/models";

/** ISO date (YYYY-MM-DD) for a Date, using UTC to stay tz-stable. */
export function isoDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

export function parseIso(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d));
}

export function todayIso(): string {
  return isoDate(new Date());
}

/** Whole-day difference a - b (positive when a is later). */
export function dayDiff(a: string, b: string): number {
  const ms = parseIso(a).getTime() - parseIso(b).getTime();
  return Math.round(ms / 86_400_000);
}

/** Add `k` intervals to an ISO date. Monthly clamps to end-of-month. */
export function addInterval(
  date: string,
  kind: IntervalKind,
  intervalDays: number | null,
  k: number,
): string {
  const d = parseIso(date);
  if (kind === "weekly") {
    d.setUTCDate(d.getUTCDate() + 7 * k);
    return isoDate(d);
  }
  if (kind === "custom") {
    d.setUTCDate(d.getUTCDate() + (intervalDays ?? 30) * k);
    return isoDate(d);
  }
  // monthly, with end-of-month clamping
  const day = d.getUTCDate();
  const target = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + k, 1));
  const lastDay = new Date(
    Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0),
  ).getUTCDate();
  target.setUTCDate(Math.min(day, lastDay));
  return isoDate(target);
}

/**
 * Equal split of `total` across `n` installments, with the rounding remainder
 * placed on the last installment so the parts sum exactly to `total`.
 */
export function splitAmounts(total: number, n: number): number[] {
  if (n <= 0) return [];
  const base = Math.trunc(total / n);
  const remainder = total - base * n;
  return Array.from({ length: n }, (_, i) => (i === n - 1 ? base + remainder : base));
}

/** Effective installment status against `today`. */
export function installmentStatus(
  amount: number,
  paid: number,
  dueDate: string,
  today = todayIso(),
): InstallmentStatus {
  if (paid >= amount) return "paid";
  if (dayDiff(dueDate, today) < 0) return "late";
  if (paid > 0) return "partial";
  return "pending";
}

/** Roll installment statuses up to a purchase-level status. */
export function purchaseStatus(
  statuses: InstallmentStatus[],
  anyPaid: boolean,
): PurchaseStatus {
  if (statuses.length > 0 && statuses.every((s) => s === "paid")) return "paid";
  if (statuses.some((s) => s === "late")) return "late";
  if (anyPaid) return "in_progress";
  return "pending";
}
