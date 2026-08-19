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

import type { ImpayeClient, Report } from "@/types/models";

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

/** Column and section labels for the report export, already localized. */
export interface ReportCsvLabels {
  /** Section headings. */
  section: {
    totals: string;
    collections: string;
    aging: string;
    clients: string;
    products: string;
  };
  /** The two-column layout the totals block uses. */
  figure: string;
  value: string;
  /** Totals rows, in the order they are written. */
  totals: {
    range: string;
    asOf: string;
    salesCount: string;
    salesAmount: string;
    collected: string;
    paymentCount: string;
    outstandingNow: string;
    overdueNow: string;
    newClients: string;
  };
  period: string;
  collected: string;
  due: string;
  bucket: string;
  count: string;
  amount: string;
  client: string;
  outstanding: string;
  overdue: string;
  overdueCount: string;
  product: string;
  purchaseCount: string;
  /** Localized name for each aging bucket, keyed by its backend key. */
  agingBucket: Record<string, string>;
}

/**
 * Flatten a report into one spreadsheet.
 *
 * Written as stacked sections rather than one wide table because the five
 * blocks have genuinely different shapes; a single table would need a sparse
 * row per block and read worse in every spreadsheet. Blank lines separate them,
 * which is what Excel and LibreOffice both treat as a section break.
 *
 * Every field goes through {@link csvField}, so the formula-injection guard
 * covers client names and product labels here exactly as it does the Impayés
 * export — these are the same operator-entered strings.
 */
export function buildReportCsv(report: Report, labels: ReportCsvLabels): string {
  const { range, totals } = report;
  const lines: string[] = [];
  const section = (title: string, header: readonly string[]) => {
    if (lines.length > 0) lines.push("");
    lines.push(csvRow([title]));
    lines.push(csvRow(header));
  };

  section(labels.section.totals, [labels.figure, labels.value]);
  for (const [label, value] of [
    [labels.totals.range, `${range.from} — ${range.to}`],
    [labels.totals.asOf, range.asOf],
    [labels.totals.salesCount, totals.salesCount],
    [labels.totals.salesAmount, totals.salesAmount],
    [labels.totals.collected, totals.collected],
    [labels.totals.paymentCount, totals.paymentCount],
    [labels.totals.outstandingNow, totals.outstandingNow],
    [labels.totals.overdueNow, totals.overdueNow],
    [labels.totals.newClients, totals.newClients],
  ] as [string, string | number][]) {
    lines.push(csvRow([label, value]));
  }

  section(labels.section.collections, [labels.period, labels.collected, labels.due]);
  for (const p of report.collections) lines.push(csvRow([p.period, p.collected, p.due]));

  section(labels.section.aging, [labels.bucket, labels.count, labels.amount]);
  for (const b of report.aging) {
    lines.push(csvRow([labels.agingBucket[b.bucket] ?? b.bucket, b.count, b.amount]));
  }

  section(labels.section.clients, [
    labels.client,
    labels.outstanding,
    labels.overdue,
    labels.overdueCount,
  ]);
  for (const c of report.topClients) {
    lines.push(csvRow([c.clientName, c.outstanding, c.overdue, c.overdueCount]));
  }

  section(labels.section.products, [labels.product, labels.purchaseCount, labels.amount]);
  for (const p of report.topProducts) {
    lines.push(csvRow([p.productLabel, p.purchaseCount, p.totalAmount]));
  }

  return CSV_BOM + lines.join("\r\n") + "\r\n";
}
