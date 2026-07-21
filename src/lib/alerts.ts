// Pure alert-derivation logic behind the Alertes page. An "alert" is any
// installment that still owes money and needs attention now: already overdue,
// due today, or falling due within a short horizon. The page (AlertesView.vue)
// feeds it the full schedule (`api.listSchedule()`) and renders/filters/sorts
// the result on the client — this module owns only the classification so it can
// be unit-tested independently of the date-sensitive view.

import { dayDiff } from "@/lib/finance";
import type { ScheduleRow } from "@/types/models";

/** How soon (in days) an unpaid installment counts as an upcoming alert. */
export const DEFAULT_SOON_DAYS = 7;

export type AlertKind = "overdue" | "dueToday" | "dueSoon";

export interface AlertRow extends ScheduleRow {
  kind: AlertKind;
  /** Days late for `overdue`; days remaining for `dueSoon`; 0 for `dueToday`. */
  days: number;
}

/**
 * Classify one schedule row against `today`. Returns an `AlertRow` when it is
 * actionable (unpaid and within the overdue…soon window), or `null` otherwise
 * (fully paid, or due further out than `soonDays`).
 */
export function classifyAlert(
  row: ScheduleRow,
  today: string,
  soonDays: number = DEFAULT_SOON_DAYS,
): AlertRow | null {
  if (row.remaining <= 0 || row.status === "paid") return null;
  const diff = dayDiff(row.dueDate, today); // dueDate − today, in whole days
  if (diff < 0) return { ...row, kind: "overdue", days: -diff };
  if (diff === 0) return { ...row, kind: "dueToday", days: 0 };
  if (diff <= soonDays) return { ...row, kind: "dueSoon", days: diff };
  return null;
}

/** Map a full schedule to just its actionable alert rows, preserving order. */
export function buildAlerts(
  rows: ScheduleRow[],
  today: string,
  soonDays: number = DEFAULT_SOON_DAYS,
): AlertRow[] {
  return rows
    .map((r) => classifyAlert(r, today, soonDays))
    .filter((r): r is AlertRow => r !== null);
}
