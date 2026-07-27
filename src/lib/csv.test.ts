import { describe, expect, it } from "vitest";

import { buildImpayesCsv, csvField, csvRow, toCsv, CSV_BOM } from "./csv";
import type { ImpayeClient } from "@/types/models";

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
