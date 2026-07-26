// Locale-aware formatting for money and dates. Reads the reactive settings
// store so a change to currency, date format, or language is reflected
// everywhere immediately.

import { computed } from "vue";
import { useSettingsStore } from "@/stores/settings";

export const LOCALE_TAG: Record<string, string> = {
  fr: "fr-FR",
  en: "en-US",
  // Tunisia commonly uses Western (Latin) digits even in Arabic UIs.
  ar: "ar-TN-u-nu-latn",
};

function pad(n: number, len = 2): string {
  return String(n).padStart(len, "0");
}

/** Format an ISO date (YYYY-MM-DD) using a dd/MM/yyyy-style pattern. */
export function formatDatePattern(iso: string | null | undefined, pattern: string): string {
  if (!iso) return "—";
  const [y, m, d] = iso.split("-").map(Number);
  if (!y || !m || !d) return iso;
  return pattern.replace(/yyyy/g, String(y)).replace(/MM/g, pad(m)).replace(/dd/g, pad(d));
}

export function useFormat() {
  const settings = useSettingsStore();

  const numberFormatter = computed(
    () =>
      new Intl.NumberFormat(LOCALE_TAG[settings.language] ?? "fr-FR", {
        maximumFractionDigits: 0,
        useGrouping: true,
      }),
  );

  /** Whole-unit money → "2 400 TND". */
  function money(amount: number | null | undefined): string {
    const value = amount ?? 0;
    return `${numberFormatter.value.format(value)} ${settings.currencyCode}`;
  }

  /** Just the grouped number, no currency code. */
  function number(amount: number | null | undefined): string {
    return numberFormatter.value.format(amount ?? 0);
  }

  function date(iso: string | null | undefined): string {
    return formatDatePattern(iso, settings.dateFormat);
  }

  return { money, number, date };
}
