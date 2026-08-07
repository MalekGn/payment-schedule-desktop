// Unit tests for the backup-recency nudge in the settings store.
//
// Backups here are manual and user-initiated: nothing schedules one, and until
// `lastBackupAt` existed nothing in the app even recorded that one had happened.
// So this computed is the entire mechanism by which a shop ever learns its data
// is unprotected — worth pinning at both edges rather than trusting the
// comparison to stay the right way round.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useSettingsStore, BACKUP_STALE_DAYS, BACKUP_FREQUENCIES } from "@/stores/settings";

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

  // The automatic snapshots taken at launch live in the app-data directory,
  // beside payment_schedule.db. One disk failure or one stolen machine takes
  // both, so they are not a substitute for a copy the user carried off — and
  // letting one quiet the nudge would tell a shop they are covered when the
  // only copies they have are on the machine that just died.
  it("is not satisfied by an automatic snapshot", () => {
    const settings = useSettingsStore();
    settings.settings.lastBackupAt = null;
    settings.settings.lastAutoBackupAt = daysAgo(0);

    expect(settings.backupIsStale).toBe(true);
  });

  it("still nudges when the manual backup is stale but the automatic one is fresh", () => {
    const settings = useSettingsStore();
    settings.settings.lastBackupAt = daysAgo(BACKUP_STALE_DAYS + 10);
    settings.settings.lastAutoBackupAt = daysAgo(0);

    expect(settings.backupIsStale).toBe(true);
  });

  // The schedule is configuration, not a record of what happened, so it must
  // not feed the nudge either — an install set to "monthly" is *more* in need
  // of the reminder, not less.
  it("ignores the schedule entirely", () => {
    const settings = useSettingsStore();
    settings.settings.lastBackupAt = null;
    settings.settings.autoBackupEnabled = true;
    settings.settings.autoBackupFrequency = "daily";

    expect(settings.backupIsStale).toBe(true);
  });
});

describe("backup schedule defaults", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  // The defaults are the whole feature for a shop that never opens Settings:
  // on, daily, 17:00. A silent drift to disabled would be invisible until the
  // day someone needed the copy.
  it("ships enabled, daily, at 17:00", () => {
    const settings = useSettingsStore();

    expect(settings.autoBackupEnabled).toBe(true);
    expect(settings.autoBackupFrequency).toBe("daily");
    expect(settings.autoBackupTime).toBe("17:00");
  });

  it("offers exactly the three cadences the backend accepts", () => {
    // Mirrors `BACKUP_FREQUENCIES` in `db.rs`; a value outside it is refused
    // with INVALID_SETTING_VALUE, so the <select> must not offer one.
    expect(BACKUP_FREQUENCIES).toEqual(["daily", "weekly", "monthly"]);
  });
});
