import { describe, expect, it } from "vitest";

import { buildImpayesCsv, buildReportCsv, csvField, csvRow, toCsv, CSV_BOM } from "./csv";
import type { ImpayeClient, Report } from "@/types/models";

describe("csvField", () => {
  it("quotes plain strings", () => {
    expect(csvField("Mohamed Trabelsi")).toBe('"Mohamed Trabelsi"');
    expect(csvField("")).toBe('""');
  });

  it("doubles embedded quotes", () => {
    // The original bug: `"Ali "Bibi" Ben Salah"` splits into three fields and
    // shifts every column to its right.
    expect(csvField('Ali "Bibi" Ben Salah')).toBe('"Ali ""Bibi"" Ben Salah"');
    expect(csvField('"')).toBe('""""');
  });

  it("keeps delimiters and newlines inside the field", () => {
    expect(csvField("Trabelsi, Mohamed")).toBe('"Trabelsi, Mohamed"');
    expect(csvField("line1\nline2")).toBe('"line1\nline2"');
  });

  it("preserves non-latin text untouched", () => {
    expect(csvField("محمد الطرابلسي")).toBe('"محمد الطرابلسي"');
    expect(csvField("Échéance")).toBe('"Échéance"');
  });

  describe("formula injection", () => {
    // Every one of these is reachable: `clientName` is first_name + last_name
    // as typed into the client form, stored with only `.trim()` applied.
    it.each([
      ["=cmd|'/c calc'!A1", "'=cmd|'/c calc'!A1"],
      ["=1+1", "'=1+1"],
      ["+1", "'+1"],
      ["-1+2", "'-1+2"],
      ["@SUM(A1)", "'@SUM(A1)"],
      ["\t=1+1", "'\t=1+1"],
      ["\r=1+1", "'\r=1+1"],
    ])("neutralizes %j", (input, guarded) => {
      expect(csvField(input)).toBe(`"${guarded}"`);
    });

    it("applies the guard before escaping, so the apostrophe stays in the cell", () => {
      expect(csvField('="a"')).toBe('"\'=""a"""');
    });

    it("leaves values that merely contain a trigger alone", () => {
      expect(csvField("Ben-Salah")).toBe('"Ben-Salah"');
      expect(csvField("a=b")).toBe('"a=b"');
    });
  });

  describe("numbers", () => {
    it("emits them bare so spreadsheets treat them as numeric", () => {
      expect(csvField(1500)).toBe("1500");
      expect(csvField(0)).toBe("0");
    });

    it("does not apostrophe-prefix a negative number", () => {
      // `-5` starts with a trigger character but is not a formula. Quoting or
      // prefixing it would break the arithmetic in the sheet.
      expect(csvField(-5)).toBe("-5");
    });

    it("renders non-finite values as empty rather than 'NaN'", () => {
      expect(csvField(Number.NaN)).toBe("");
      expect(csvField(Number.POSITIVE_INFINITY)).toBe("");
    });
  });
});

describe("csvRow / toCsv", () => {
  it("joins fields with commas", () => {
    expect(csvRow(["a", 1, "b"])).toBe('"a",1,"b"');
  });

  it("emits a BOM, CRLF line endings and a trailing newline", () => {
    const out = toCsv(["h1", "h2"], [["a", 1]]);
    expect(out.startsWith(CSV_BOM)).toBe(true);
    expect(out).toBe(`${CSV_BOM}"h1","h2"\r\n"a",1\r\n`);
  });

  it("escapes header cells too", () => {
    // Localized headers can contain commas in some languages.
    expect(toCsv(["Montant, TND"], [])).toBe(`${CSV_BOM}"Montant, TND"\r\n`);
  });
});

describe("buildImpayesCsv", () => {
  const LABELS = {
    client: "Client",
    phone: "Phone",
    reference: "Ref.",
    installment: "Installment",
    dueDate: "Due date",
    amount: "Amount",
    daysLate: "Overdue since",
  };

  function client(over: Partial<ImpayeClient> = {}): ImpayeClient {
    return {
      clientId: 1,
      clientName: "Mohamed Trabelsi",
      phone: "+216 98 123 456",
      address: "Tunis",
      email: null,
      reference: "A-000001",
      totalOverdue: 400,
      overdueCount: 1,
      installments: [
        {
          installmentId: 10,
          purchaseId: 1,
          purchaseReference: "A-000001",
          index: 2,
          installmentCount: 6,
          dueDate: "2026-06-01",
          amount: 400,
          remaining: 400,
          daysLate: 56,
        },
      ],
      ...over,
    };
  }

  it("emits one row per overdue installment, plus the header", () => {
    const csv = buildImpayesCsv([client()], LABELS);
    const lines = csv.replace(CSV_BOM, "").trimEnd().split("\r\n");

    expect(lines).toHaveLength(2);
    expect(lines[0]).toBe(
      '"Client","Phone","Ref.","Installment","Due date","Amount","Overdue since"',
    );
    // Note the apostrophe on the phone — see the dedicated case below.
    expect(lines[1]).toBe(
      '"Mohamed Trabelsi","\'+216 98 123 456","A-000001","2/6","2026-06-01",400,56',
    );
  });

  it("guards the leading + on international phone numbers", () => {
    // Every Tunisian number starts `+216`, so this is the common case, not an
    // edge one. The guard is still the right call: an unprefixed `+216 98 …`
    // is parsed by Excel as a formula and renders as `#NAME?`, which is worse
    // than a visible apostrophe.
    const csv = buildImpayesCsv([client({ phone: "+216 98 123 456" })], LABELS);
    expect(csv).toContain('"\'+216 98 123 456"');
  });

  it("leaves a local-format phone number unprefixed", () => {
    const csv = buildImpayesCsv([client({ phone: "98 123 456" })], LABELS);
    expect(csv).toContain('"98 123 456"');
  });

  it("flattens several clients and several installments", () => {
    const many = client({
      clientId: 2,
      installments: [client().installments[0], { ...client().installments[0], installmentId: 11 }],
    });
    const csv = buildImpayesCsv([client(), many], LABELS);
    const lines = csv.replace(CSV_BOM, "").trimEnd().split("\r\n");
    expect(lines).toHaveLength(4); // header + 1 + 2
  });

  it("survives a hostile client name without breaking the row", () => {
    const csv = buildImpayesCsv([client({ clientName: "=cmd|'/c calc'!A1 \"x\", 999" })], LABELS);
    const lines = csv.replace(CSV_BOM, "").trimEnd().split("\r\n");

    expect(lines).toHaveLength(2);
    // Neutralized, quotes doubled, comma contained — and the row still has its
    // seven fields, so no column is shifted.
    expect(lines[1]).toContain('"\'=cmd|\'/c calc\'!A1 ""x"", 999"');
    expect(lines[1].endsWith(",400,56")).toBe(true);
  });

  it("renders an empty list as a header-only file", () => {
    const csv = buildImpayesCsv([], LABELS);
    expect(csv.replace(CSV_BOM, "").trimEnd().split("\r\n")).toHaveLength(1);
  });
});

describe("buildReportCsv", () => {
  const REPORT: Report = {
    range: { from: "2026-01-01", to: "2026-01-31", asOf: "2026-02-02", granularity: "day" },
    totals: {
      salesCount: 2,
      salesAmount: 3000,
      collected: 1200,
      paymentCount: 3,
      outstandingNow: 1800,
      overdueNow: 400,
      newClients: 1,
    },
    collections: [
      { period: "2026-01-01", collected: 700, due: 900 },
      { period: "2026-01-02", collected: 0, due: 0 },
    ],
    aging: [
      { bucket: "current", count: 2, amount: 1400 },
      { bucket: "1-30", count: 1, amount: 400 },
      { bucket: "31-60", count: 0, amount: 0 },
      { bucket: "61-90", count: 0, amount: 0 },
      { bucket: "90+", count: 0, amount: 0 },
    ],
    // A client name that is a live formula in Excel, and one carrying a quote.
    topClients: [
      {
        clientId: 1,
        clientName: "=cmd|'/c calc'!A1",
        outstanding: 1400,
        overdue: 0,
        overdueCount: 0,
      },
      {
        clientId: 2,
        clientName: 'Ali "Bibi" Ben Salah',
        outstanding: 400,
        overdue: 400,
        overdueCount: 1,
      },
    ],
    topProducts: [{ productLabel: "+Réfrigérateur", purchaseCount: 2, totalAmount: 3000 }],
  };

  const LABELS = {
    section: {
      totals: "Synthèse",
      collections: "Encaissements",
      aging: "Ancienneté",
      clients: "Clients",
      products: "Produits",
    },
    figure: "Indicateur",
    value: "Valeur",
    totals: {
      range: "Période",
      asOf: "Arrêté au",
      salesCount: "Nombre de ventes",
      salesAmount: "Ventes",
      collected: "Encaissé",
      paymentCount: "Nombre de paiements",
      outstandingNow: "Reste à recouvrer",
      overdueNow: "En retard",
      newClients: "Nouveaux clients",
    },
    period: "Période",
    collected: "Encaissé",
    due: "Échu",
    bucket: "Ancienneté",
    count: "Tranches",
    amount: "Montant",
    client: "Client",
    outstanding: "Reste à payer",
    overdue: "En retard",
    overdueCount: "Tranches en retard",
    product: "Produit",
    purchaseCount: "Ventes",
    agingBucket: { current: "Pas encore échu", "1-30": "1 à 30 jours" },
  };

  const lines = () => buildReportCsv(REPORT, LABELS).replace(CSV_BOM, "").split("\r\n");

  it("starts with the BOM Excel needs to read UTF-8", () => {
    expect(buildReportCsv(REPORT, LABELS).startsWith(CSV_BOM)).toBe(true);
  });

  it("writes each section under its localized heading", () => {
    const out = lines();
    for (const heading of Object.values(LABELS.section)) {
      expect(out).toContain(`"${heading}"`);
    }
  });

  it("neutralizes a client name that would execute as a formula", () => {
    // The whole reason this export is guarded: these names come from the client
    // form, which stores what the shopkeeper typed with only a trim applied.
    const out = lines();
    expect(out.some((l) => l.startsWith(`"'=cmd|'/c calc'!A1"`))).toBe(true);
    expect(out.some((l) => l.includes(`"'+Réfrigérateur"`))).toBe(true);
    // The guard must not have fired on a name that merely contains a quote.
    expect(out.some((l) => l.includes(`"Ali ""Bibi"" Ben Salah"`))).toBe(true);
  });

  it("emits money bare so spreadsheets treat it as numeric", () => {
    expect(lines()).toContain('"Ventes",3000');
  });

  it("carries the as-of date, so a balance is never read as a period figure", () => {
    expect(lines()).toContain('"Arrêté au","2026-02-02"');
    expect(lines()).toContain('"Période","2026-01-01 — 2026-01-31"');
  });

  it("keeps every collections bucket, including the empty ones", () => {
    const out = lines();
    expect(out).toContain('"2026-01-01",700,900');
    expect(out).toContain('"2026-01-02",0,0');
  });

  it("falls back to the raw bucket key when a label is missing", () => {
    // `agingBucket` above deliberately omits three of the five.
    expect(lines().some((l) => l.startsWith('"90+"'))).toBe(true);
  });
});
