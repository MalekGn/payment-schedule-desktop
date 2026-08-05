// Unit tests for the i18n layer, and for the locale invariant `CLAUDE.md` calls
// a blocker: "every user-facing string lands in all three of
// src/locales/{ar,fr,en}.json".
//
// Until now nothing enforced that. The integration suite imports the locale
// files, but only to check that error *codes* resolve to prose, and it is
// opt-in. So a key added to fr.json and forgotten in ar.json shipped as a French
// sentence in the middle of an Arabic screen, and the only thing standing in the
// way was somebody remembering to diff the files by hand.
//
// Placeholder parity is checked as well as key parity, because a translation
// that drops `{count}` is worse than a missing one: the key resolves, the
// sentence renders, and the number the sentence is about simply is not there.
// It compares the distinct names a string offers rather than how often each is
// used — see `placeholders` for why plural forms make the stricter reading wrong.
//
// The RTL *layout* that `applyLocale` triggers cannot be asserted in jsdom (no
// styles), so this only pins the `dir` attribute the CSS hangs off. Mirrored
// layout stays a manual check, as CLAUDE.md says.

import { describe, it, expect, afterEach } from "vitest";
import { SUPPORTED_LOCALES, applyLocale, i18n, isSupportedLocale, resolveOsLocale } from "@/i18n";
import ar from "@/locales/ar.json";
import en from "@/locales/en.json";
import fr from "@/locales/fr.json";

type Messages = Record<string, unknown>;
const BUNDLES: Record<string, Messages> = { fr, en, ar };

/** Every leaf key, dotted — `errors.belowPaid`, not the nested object. */
function flatten(node: Messages, prefix = ""): Map<string, string> {
  const out = new Map<string, string>();
  for (const [key, value] of Object.entries(node)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === "object") {
      for (const [k, v] of flatten(value as Messages, path)) out.set(k, v);
    } else {
      out.set(path, String(value));
    }
  }
  return out;
}

/**
 * The distinct `{name}` placeholders a translation offers.
 *
 * Deduplicated on purpose. Several strings are vue-i18n plural forms
 * ("{n} jour de retard | {n} jours de retard"), and a branch may legitimately
 * not interpolate at all where the language says it lexically — Arabic's
 * singular here is "متأخر بيوم", "late by a day". Counting occurrences would
 * flag that as a defect. What must match is *which* names a sentence can use,
 * so that a translation dropping `{count}` outright is still caught.
 */
const placeholders = (text: string) =>
  [...new Set([...text.matchAll(/{(\w+)}/g)].map((m) => m[1]))].sort();

const FLAT = Object.fromEntries(
  Object.entries(BUNDLES).map(([loc, msgs]) => [loc, flatten(msgs)]),
) as Record<string, Map<string, string>>;

describe("locale files — every string exists in all three languages", () => {
  it("ships the same set of keys in fr, en and ar", () => {
    const reference = [...FLAT.fr.keys()].sort();
    for (const locale of SUPPORTED_LOCALES) {
      const missing = reference.filter((k) => !FLAT[locale].has(k));
      const extra = [...FLAT[locale].keys()].filter((k) => !FLAT.fr.has(k)).sort();
      // Named rather than a bare boolean: a failure has to say which keys, or
      // fixing it means diffing 379 lines by hand — the very chore this
      // replaces.
      expect({ locale, missing, extra }).toEqual({ locale, missing: [], extra: [] });
    }
  });

  it("has no empty translations, which render as a blank screen element", () => {
    for (const locale of SUPPORTED_LOCALES) {
      const blank = [...FLAT[locale].entries()].filter(([, v]) => v.trim() === "").map(([k]) => k);
      expect({ locale, blank }).toEqual({ locale, blank: [] });
    }
  });

  it("interpolates the same placeholders in every language", () => {
    for (const key of FLAT.fr.keys()) {
      const perLocale = SUPPORTED_LOCALES.map((loc) => ({
        locale: loc,
        params: placeholders(FLAT[loc].get(key) ?? ""),
      }));
      const reference = perLocale[0].params;
      for (const { locale, params } of perLocale) {
        // A dropped `{count}` still resolves and still renders — it just
        // silently loses the number the sentence is about.
        expect({ key, locale, params }).toEqual({ key, locale, params: reference });
      }
    }
  });
});

describe("resolveOsLocale — choosing a language on a fresh install", () => {
  it("takes the base tag when the OS offers one we ship", () => {
    expect(resolveOsLocale("ar-TN")).toBe("ar");
    expect(resolveOsLocale("en-US")).toBe("en");
    expect(resolveOsLocale("fr")).toBe("fr");
  });

  it("accepts either separator and any casing, as OSes differ", () => {
    expect(resolveOsLocale("AR_tn")).toBe("ar");
    expect(resolveOsLocale("EN-gb")).toBe("en");
  });

  it("falls back to French for a language we do not ship", () => {
    expect(resolveOsLocale("de-DE")).toBe("fr");
    expect(resolveOsLocale("ja")).toBe("fr");
  });

  it("falls back to French when the OS tells us nothing", () => {
    expect(resolveOsLocale(null)).toBe("fr");
    expect(resolveOsLocale(undefined)).toBe("fr");
    expect(resolveOsLocale("")).toBe("fr");
  });
});

describe("isSupportedLocale", () => {
  it("admits exactly the three languages we ship", () => {
    expect(SUPPORTED_LOCALES.every(isSupportedLocale)).toBe(true);
    expect(isSupportedLocale("de")).toBe(false);
    // Guards a persisted settings value, which is why the region form matters:
    // "ar-TN" is not itself a locale we can switch to.
    expect(isSupportedLocale("ar-TN")).toBe(false);
  });
});

describe("applyLocale — what the document ends up carrying", () => {
  it("marks Arabic right-to-left and everything else left-to-right", () => {
    applyLocale("ar");
    expect(document.documentElement.getAttribute("lang")).toBe("ar");
    expect(document.documentElement.getAttribute("dir")).toBe("rtl");

    applyLocale("fr");
    expect(document.documentElement.getAttribute("lang")).toBe("fr");
    expect(document.documentElement.getAttribute("dir")).toBe("ltr");
  });

  it("switches the strings vue-i18n resolves", () => {
    applyLocale("en");
    const english = i18n.global.t("common.save");
    applyLocale("fr");
    expect(i18n.global.t("common.save")).not.toBe(english);
  });

  afterEach(() => {
    // `i18n` is a module singleton shared by every test file in the run, so a
    // locale left switched here would break assertions elsewhere.
    applyLocale("fr");
  });
});
