// Call / SMS actions for a client's phone number.
//
// These must never be plain `<a href="tel:…">` links. Tauri's WebView does not
// delegate external URI schemes to the OS, so such an anchor navigates the
// WebView itself, fails to load the scheme, and replaces the entire SPA with the
// WebView's native error page ("The URL can't be shown"). The window has no
// browser chrome and the router never sees the navigation, so the user cannot
// get back — they have to quit the app. See docs/e2e/qa-report.md (2026-07-26).
//
// Instead we hand the URI to the OS through the api gateway (Tauri's opener
// plugin) and report failure as a toast. On desktop Linux there is frequently no
// `tel:`/`sms:` handler registered at all, so failure is the normal path there
// rather than an edge case — the message includes the number so the user can
// still act on it.
import { useI18n } from "vue-i18n";
import { api } from "@/api";
import { useUiStore } from "@/stores/ui";

export type ContactKind = "call" | "message";

/**
 * Characters that can legitimately appear in a hand-written phone number.
 * `phone` is free text from the client form, so this is the only guard between
 * that field and the OS URI handler — it must reject prose, other schemes, and
 * URI syntax (`:`, `?`, `#`, `,`) outright.
 */
const PHONE_CHARS = /^[+0-9\s()./-]+$/;

/** Plausible digit counts, from a short local number to E.164 plus slack. */
const MIN_DIGITS = 3;
const MAX_DIGITS = 20;

const SCHEME: Record<ContactKind, string> = { call: "tel", message: "sms" };

/**
 * Build a `tel:`/`sms:` URI for a phone number, discarding the separators people
 * type (`(216) 98-123.456` → `tel:21698123456`) and keeping a leading `+`.
 *
 * @returns The URI, or `null` when the number is not dialable.
 */
export function contactUri(kind: ContactKind, phone: string): string | null {
  const raw = phone.trim();
  if (!PHONE_CHARS.test(raw)) return null;
  // A "+" is only meaningful as the international prefix; anywhere else it is
  // a malformed number we should refuse rather than silently normalize.
  if (raw.lastIndexOf("+") > 0) return null;

  const digits = raw.replace(/\D/g, "");
  if (digits.length < MIN_DIGITS || digits.length > MAX_DIGITS) return null;

  return `${SCHEME[kind]}:${raw.startsWith("+") ? "+" : ""}${digits}`;
}

/**
 * Returns `call(phone)` / `message(phone)` handlers for contact buttons. Both
 * are safe to bind directly to `@click`; failures surface as error toasts.
 */
export function useContactActions() {
  const { t } = useI18n();
  const ui = useUiStore();

  async function contact(kind: ContactKind, phone: string): Promise<void> {
    const uri = contactUri(kind, phone);
    if (!uri) {
      ui.notify(t("impaye.invalidPhone", { phone }), "error");
      return;
    }
    try {
      await api.openExternal(uri);
    } catch (e) {
      // Keep the detail in the console for diagnostics; the toast stays clean
      // (no scheme internals or plugin error text in front of the user).
      console.error(`openExternal failed for ${SCHEME[kind]}:`, e);
      const key = kind === "call" ? "impaye.callFailed" : "impaye.messageFailed";
      ui.notify(t(key, { phone }), "error");
    }
  }

  return {
    call: (phone: string) => contact("call", phone),
    message: (phone: string) => contact("message", phone),
  };
}
