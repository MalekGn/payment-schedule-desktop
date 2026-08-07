// Unit tests for the backup-recency nudge in the settings store.
//
// Backups here are manual and user-initiated: nothing schedules one, and until
// `lastBackupAt` existed nothing in the app even recorded that one had happened.
// So this computed is the entire mechanism by which a shop ever learns its data
// is unprotected — worth pinning at both edges rather than trusting the
// comparison to stay the right way round.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useSettingsStore, BACKUP_STALE_DAYS } from "@/stores/settings";

/** A fixed "today" so the day arithmetic cannot drift with the wall clock. */
const TODAY = new Date("2026-08-07T09:00:00");

/** `n` days before TODAY, as the ISO date the backend would have stored. */
function daysAgo(n: number): string {
  const d = new Date(TODAY);
  d.setDate(d.getDate() - n);
  return d.toISOString().slice(0, 10);
}

describe("backup staleness", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
    vi.setSystemTime(TODAY);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("nudges an install that has never backed up", () => {
    const settings = useSettingsStore();
    // The default a fresh database reports: the key has never been written.
    expect(settings.lastBackupAt).toBeNull();
    expect(settings.backupIsStale).toBe(true);
  });

  it("stays quiet for a recent backup", () => {
    const settings = useSettingsStore();
    settings.settings.lastBackupAt = daysAgo(1);
    expect(settings.backupIsStale).toBe(false);
  });

  it("nudges once the backup reaches the staleness threshold", () => {
    const settings = useSettingsStore();

    // The day before the threshold is still quiet…
    settings.settings.lastBackupAt = daysAgo(BACKUP_STALE_DAYS - 1);
    expect(settings.backupIsStale).toBe(false);

    // …and the threshold itself nudges.
    settings.settings.lastBackupAt = daysAgo(BACKUP_STALE_DAYS);
    expect(settings.backupIsStale).toBe(true);
  });

  it("treats a backup taken today as current", () => {
    const settings = useSettingsStore();
    settings.settings.lastBackupAt = daysAgo(0);
    expect(settings.backupIsStale).toBe(false);
  });
});
