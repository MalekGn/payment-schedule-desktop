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

/**
 * Add `k` intervals to an ISO date. Monthly clamps to end-of-month.
 *
 * Saturates to `date` when the result would fall outside the representable
 * range, matching the checked arithmetic in `add_interval` in `db.rs`. Without
 * the guard an extreme `intervalDays` produced an `Invalid Date` here and a
 * *panic* on the Rust side — and with `panic = "abort"` in the release profile
 * that aborted the whole app.
 */
export function addInterval(
  date: string,
  kind: IntervalKind,
  intervalDays: number | null,
  k: number,
): string {
  const d = parseIso(date);
  const shifted = (days: number): string => {
    const out = new Date(d.getTime());
    out.setUTCDate(out.getUTCDate() + days);
    return Number.isNaN(out.getTime()) ? date : isoDate(out);
  };

  if (kind === "weekly") return shifted(7 * k);
  if (kind === "custom") return shifted((intervalDays ?? 30) * k);

  // monthly, with end-of-month clamping
  const day = d.getUTCDate();
  const target = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + k, 1));
  if (Number.isNaN(target.getTime())) return date;
  const lastDay = new Date(
    Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0),
  ).getUTCDate();
  target.setUTCDate(Math.min(day, lastDay));
  return Number.isNaN(target.getTime()) ? date : isoDate(target);
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

/**
 * Re-split `pool` across the installments at `absorbers` (indices into
 * `amounts`), refusing any distribution that would push a row below what has
 * already been collected on it.
 *
 * `null` means this absorber set cannot take the change: either the pool went
 * negative, or an even split lands under someone's `paidAmount` — which would
 * break the `paidAmount <= amount` invariant the outstanding aggregates rely on.
 */
function applyPool(
  amounts: number[],
  paidAmounts: number[],
  absorbers: number[],
  pool: number,
): number[] | null {
  if (absorbers.length === 0 || pool < 0) return null;
  const parts = splitAmounts(pool, absorbers.length);
  const next = [...amounts];
  for (let k = 0; k < absorbers.length; k++) {
    if (parts[k] < paidAmounts[absorbers[k]]) return null;
    next[absorbers[k]] = parts[k];
  }
  return next;
}

/**
 * The new amount vector after setting installment `index` (0-based) to
 * `newAmount`, holding the purchase total fixed.
 *
 * `sum(amounts) === purchase.totalPrice` is assumed by every read model in the
 * app, so a single-installment edit has to move the difference somewhere rather
 * than change the total. The delta lands on the installments *after* the edited
 * one first — those are the ones still ahead of the client — and only falls back
 * to the earlier unsettled ones when there is nothing later to absorb it, which
 * is what makes the final installment editable at all.
 *
 * Fully-paid installments are never absorbers: their amount is settled history.
 *
 * Returns `null` when neither absorber set can take the change; the caller turns
 * that into `NO_REBALANCE_ROOM`. Mirrors `rebalance_amounts` in `db.rs`.
 */
export function rebalanceAmounts(
  amounts: number[],
  paidAmounts: number[],
  index: number,
  newAmount: number,
): number[] | null {
  if (index < 0 || index >= amounts.length) return null;
  if (newAmount < 0 || newAmount < paidAmounts[index]) return null;

  const delta = newAmount - amounts[index];
  const base = [...amounts];
  base[index] = newAmount;
  if (delta === 0) return base;

  const unsettled = (i: number): boolean => i !== index && paidAmounts[i] < amounts[i];
  const later: number[] = [];
  const all: number[] = [];
  for (let i = 0; i < amounts.length; i++) {
    if (!unsettled(i)) continue;
    all.push(i);
    if (i > index) later.push(i);
  }

  const sumOf = (set: number[]): number => set.reduce((s, i) => s + amounts[i], 0);
  return (
    applyPool(base, paidAmounts, later, sumOf(later) - delta) ??
    applyPool(base, paidAmounts, all, sumOf(all) - delta)
  );
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
export function purchaseStatus(statuses: InstallmentStatus[], anyPaid: boolean): PurchaseStatus {
  if (statuses.length > 0 && statuses.every((s) => s === "paid")) return "paid";
  if (statuses.some((s) => s === "late")) return "late";
  if (anyPaid) return "in_progress";
  return "pending";
}
