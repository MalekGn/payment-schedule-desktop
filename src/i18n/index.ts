import { createI18n } from "vue-i18n";
import ar from "@/locales/ar.json";
import en from "@/locales/en.json";
import fr from "@/locales/fr.json";

export const SUPPORTED_LOCALES = ["fr", "en", "ar"] as const;
export type AppLocale = (typeof SUPPORTED_LOCALES)[number];

export const RTL_LOCALES: AppLocale[] = ["ar"];

export const i18n = createI18n({
  legacy: false,
  locale: "fr",
  fallbackLocale: "fr",
  messages: { fr, en, ar },
});

export function isSupportedLocale(value: string): value is AppLocale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/**
 * Resolve the initial locale for a fresh install: the OS locale if it maps to
 * one of our three languages, otherwise French.
 */
export function resolveOsLocale(raw: string | null | undefined): AppLocale {
  if (!raw) return "fr";
  const base = raw.toLowerCase().split(/[-_]/)[0];
  return isSupportedLocale(base) ? base : "fr";
}

/** Apply the locale to vue-i18n and to the document (lang + dir for RTL). */
export function applyLocale(locale: AppLocale): void {
  i18n.global.locale.value = locale;
  const dir = RTL_LOCALES.includes(locale) ? "rtl" : "ltr";
  document.documentElement.setAttribute("lang", locale);
  document.documentElement.setAttribute("dir", dir);
}
