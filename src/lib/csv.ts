// CSV output. Pure and framework-free so it can be unit-tested without a DOM —
// the view keeps only the Blob/anchor plumbing.
//
// # Why this exists
//
// The Impayés export used to build rows with `` `"${c.clientName}"` `` — a bare
// template wrap. Two defects fell out of that, and both are reachable from data
// a shopkeeper types into the client form, which the backend stores with only
// `.trim()` applied:
//
//   * A name containing a `"` produced `"Ali "Bibi" Ben Salah"`, which every
//     spreadsheet parses as a broken field, silently corrupting the rest of the
//     row.
//   * A name beginning with `=`, `+`, `-` or `@` was written straight into the
//     cell, so `=cmd|'/c calc'!A1` becomes a live DDE formula the moment the
//     file is opened in Excel. That is CSV formula injection, and the export is
//     the one place in this app where operator-entered text leaves the sandbox
//     and lands in another program's execution context.

import type { ImpayeClient } from "@/types/models";

/**
 * A leading character that makes a spreadsheet treat the cell as a formula.
 * Tab and CR are included because Excel strips them and then evaluates what is
 * left, so `\t=1+1` is just as live as `=1+1`.
 *
 * Note this fires on every international phone number (`+216 …`), which is the
 * common case here rather than an edge one. That is still the right outcome:
 * unguarded, Excel parses `+216 98 123 456` as a formula and shows `#NAME?`,
 * so the apostrophe fixes the phone column as well as securing it.
 */
const FORMULA_TRIGGER = /^[=+\-@\t\r]/;

/**
 * Byte-order mark. Excel needs it to read the file as UTF-8; without it the
 * accented French and the Arabic column headers arrive as mojibake.
 */
export const CSV_BOM = "﻿";

/** RFC 4180 says CRLF, and Excel is the strictest consumer here. */
const CRLF = "\r\n";

/**
 * Render one value as a CSV field.
 *
 * Numbers are emitted bare: they cannot carry a delimiter, and quoting them
 * would stop spreadsheets treating them as numeric. A negative number starts
 * with `-` but is not a formula, which is exactly why the type is part of the
 * contract rather than something to sniff at runtime.
 */
export function csvField(value: string | number): string {
  if (typeof value === "number") {
    return Number.isFinite(value) ? String(value) : "";
  }
  // Neutralize first, then escape — so the apostrophe itself ends up inside the
  // quoted field rather than in front of it.
  const guarded = FORMULA_TRIGGER.test(value) ? `'${value}` : value;
  return `"${guarded.replace(/"/g, '""')}"`;
}

/** Render one row. Every field goes through {@link csvField}. */
export function csvRow(values: readonly (string | number)[]): string {
  return values.map(csvField).join(",");
}

/**
 * Render a complete CSV document, BOM included.
 *
 * Headers are escaped like any other field: they are localized strings, so they
 * can contain commas in some languages.
 */
export function toCsv(
  header: readonly string[],
  rows: readonly (readonly (string | number)[])[],
): string {
  return CSV_BOM + [csvRow(header), ...rows.map(csvRow)].join(CRLF) + CRLF;
}

/** Column labels for the Impayés export, in order, already localized. */
export interface ImpayesCsvLabels {
  client: string;
  phone: string;
  reference: string;
  installment: string;
  dueDate: string;
  amount: string;
  daysLate: string;
}

/**
 * Flatten the overdue view — one row per overdue installment, carrying its
 * client's name and phone so each row stands alone in a spreadsheet.
 */
export function buildImpayesCsv(
  clients: readonly ImpayeClient[],
  labels: ImpayesCsvLabels,
): string {
  const header = [
    labels.client,
    labels.phone,
    labels.reference,
    labels.installment,
    labels.dueDate,
    labels.amount,
    labels.daysLate,
  ];
  const rows: (string | number)[][] = [];
  for (const c of clients) {
    for (const i of c.installments) {
      rows.push([
        c.clientName,
        c.phone,
        i.purchaseReference,
        `${i.index}/${i.installmentCount}`,
        i.dueDate,
        i.remaining,
        i.daysLate,
      ]);
    }
  }
  return toCsv(header, rows);
}
