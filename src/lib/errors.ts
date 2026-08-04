// Turns a rejection from the `api` gateway into a sentence a shopkeeper can read.
//
// The backend (`src-tauri/src/error.rs`) never sends prose: every command
// rejects with a stable machine code, optionally followed by colon-separated
// parameters — `INVALID_AMOUNT`, `CLIENT_HAS_PURCHASES:3`, `SUM_MISMATCH:900:1000`.
// Anything the user cannot act on collapses to the opaque `INTERNAL`, with the
// real detail kept in the backend log.
//
// Before this existed, views did `ui.notify(String(e), "error")`, so a raw
// rusqlite message ("FOREIGN KEY constraint failed") reached the user verbatim
// and unlocalized. `src/api/mock.ts` throws the same codes, so the browser and
// desktop builds behave identically here.

/** Signature of vue-i18n's `t`, narrowed to what this module needs. */
type Translate = (key: string, named?: Record<string, unknown>) => string;

/**
 * Positional parameter names, by code. A code's entry lists the i18n
 * placeholders its colon-separated parameters map onto, in order.
 */
const CODE_PARAMS: Record<string, readonly string[]> = {
  CLIENT_HAS_PURCHASES: ["count"],
  ARCHIVE_HAS_OUTSTANDING: ["remaining"],
  PURCHASE_HAS_PAYMENTS: ["count"],
  SUM_MISMATCH: ["sum", "total"],
  INSTALLMENT_COUNT_MISMATCH: ["sent", "declared"],
  TEXT_TOO_LONG: ["max"],
  OVERPAYMENT: ["remaining"],
  PREVIOUS_UNPAID: ["index"],
  BELOW_PAID: ["paidAmount"],
  PAID_ABOVE_AMOUNT: ["amount"],
  // The licence status tag ("expired", "machineMismatch", …) that made an
  // import fail. Interpolated so the user learns why their file was refused.
  INVALID_LICENSE: ["status"],
};

/** A code is `SCREAMING_SNAKE_CASE`; anything else is not one of ours. */
const CODE_PATTERN = /^[A-Z][A-Z0-9_]*$/;

/** `CLIENT_HAS_PURCHASES` → `clientHasPurchases`, matching the locale keys. */
function toI18nKey(code: string): string {
  return code.toLowerCase().replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());
}

/**
 * Extract the raw message from whatever a rejected promise carried.
 *
 * `invoke` rejects with the serialized error itself (a plain string), whereas
 * the mock throws an `Error`. Both have to be understood.
 */
function rawMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

/** Split `CODE:p1:p2` into its code and parameters, if it is one of ours. */
export function parseErrorCode(e: unknown): { code: string; params: string[] } | null {
  const [code, ...params] = rawMessage(e).trim().split(":");
  return CODE_PATTERN.test(code) ? { code, params } : null;
}

/**
 * Map a caught error to a localized, user-facing message.
 *
 * Always logs the original to the console first: the detail is genuinely useful
 * when diagnosing a report, and it is the only place it survives on the
 * frontend. The toast itself stays free of internals — the same split
 * `useContactActions` already uses.
 */
export function toUserMessage(e: unknown, t: Translate): string {
  console.error("command failed:", e);

  const parsed = parseErrorCode(e);
  if (!parsed) return t("errors.generic");

  const key = `errors.${toI18nKey(parsed.code)}`;
  const names = CODE_PARAMS[parsed.code] ?? [];
  const named: Record<string, string> = {};
  names.forEach((name, i) => {
    named[name] = parsed.params[i] ?? "";
  });

  const message = t(key, named);
  // vue-i18n echoes the key back when it is missing. A code we have not
  // localized yet must degrade to the generic sentence rather than showing the
  // user a dotted key path.
  return message === key ? t("errors.generic") : message;
}
