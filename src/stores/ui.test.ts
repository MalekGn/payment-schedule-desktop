// Unit tests for the toast queue.
//
// Two rules here exist because of specific past failures, and both are invisible
// until they break. Errors do *not* expire on their own — 3.5 s is not long
// enough to read a sentence, less so in a second language, and an error the user
// missed is one they are about to walk into again. That decision is what makes
// the ceiling necessary: a repeatedly failing action would otherwise grow the
// stack without bound, because nothing removes those toasts.
//
// The timers are the whole point, so this file establishes the fake-timer
// convention for the repo. `AppToasts.vue` is deliberately not tested: it is six
// lines of script over this store plus a `TransitionGroup`, so asserting through
// it would mean fighting transition timing to re-test what is here.

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useUiStore } from "@/stores/ui";

/** How long a transient toast is meant to stay up. */
const LIFETIME = 3500;

describe("toasts — how long they stay up", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });

  it("clears a success toast once its time is up", () => {
    const ui = useUiStore();
    ui.notify("Enregistré");
    expect(ui.toasts).toHaveLength(1);

    vi.advanceTimersByTime(LIFETIME - 1);
    expect(ui.toasts).toHaveLength(1);

    vi.advanceTimersByTime(1);
    expect(ui.toasts).toHaveLength(0);
  });

  it("clears an info toast on the same schedule", () => {
    const ui = useUiStore();
    ui.notify("Sauvegarde en cours", "info");
    vi.advanceTimersByTime(LIFETIME);
    expect(ui.toasts).toHaveLength(0);
  });

  it("keeps an error until the user dismisses it", () => {
    const ui = useUiStore();
    ui.notify("La sauvegarde a échoué", "error");

    // Far past any plausible timeout: errors have no timer at all.
    vi.advanceTimersByTime(60 * 60 * 1000);
    expect(ui.toasts).toHaveLength(1);

    ui.dismiss(ui.toasts[0].id);
    expect(ui.toasts).toHaveLength(0);
  });

  afterEach(() => {
    vi.useRealTimers();
  });
});

describe("toasts — the ceiling that keeps errors from piling up", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });

  it("keeps at most four, dropping the oldest first", () => {
    const ui = useUiStore();
    for (const n of [1, 2, 3, 4, 5]) ui.notify(`erreur ${n}`, "error");

    expect(ui.toasts).toHaveLength(4);
    // The first one is what goes: a user watching the stack sees the newest.
    expect(ui.toasts.map((t) => t.message)).toEqual([
      "erreur 2",
      "erreur 3",
      "erreur 4",
      "erreur 5",
    ]);
  });

  it("does not let a repeatedly failing action grow the stack", () => {
    const ui = useUiStore();
    for (let n = 0; n < 50; n++) ui.notify("échec", "error");
    expect(ui.toasts).toHaveLength(4);
  });

  afterEach(() => {
    vi.useRealTimers();
  });
});

describe("toasts — dismissing", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });

  it("removes only the toast asked for", () => {
    const ui = useUiStore();
    ui.notify("un", "error");
    ui.notify("deux", "error");

    ui.dismiss(ui.toasts[0].id);
    expect(ui.toasts.map((t) => t.message)).toEqual(["deux"]);
  });

  it("ignores an id that is no longer there", () => {
    const ui = useUiStore();
    ui.notify("un", "error");
    const id = ui.toasts[0].id;

    ui.dismiss(id);
    expect(() => ui.dismiss(id)).not.toThrow();
    expect(ui.toasts).toHaveLength(0);
  });

  it("does not let an evicted toast's timer take a later one with it", () => {
    const ui = useUiStore();
    // Five transient toasts: the first is evicted by the ceiling while its
    // 3.5 s timer is still pending. Ids are monotonic, so that timer must find
    // nothing to remove rather than matching whatever now sits in its place.
    for (const n of [1, 2, 3, 4, 5]) ui.notify(`message ${n}`);
    expect(ui.toasts).toHaveLength(4);

    vi.advanceTimersByTime(LIFETIME);
    expect(ui.toasts).toHaveLength(0);
  });

  afterEach(() => {
    vi.useRealTimers();
  });
});
