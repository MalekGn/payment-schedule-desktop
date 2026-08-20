// Unit tests for the printed-document file names.
//
// The bug these follow: every document was offered as `output.pdf`, because the
// app never set a document title for the print engine to derive a name from.
// What is worth pinning here is not the happy path but the two ways a name can
// go wrong — text that cannot be represented in ASCII at all, and text that a
// shopkeeper typed and that must not be able to steer a path.

import { describe, expect, it } from "vitest";

import { clientPart, documentFilename, slugify } from "./filename";

describe("slugify", () => {
  it("folds accents rather than dropping the letters under them", () => {
    // The naive `[^A-Za-z0-9]` pass without NFD gives `Rfrigrateur`.
    expect(slugify("Réfrigérateur")).toBe("Refrigerateur");
    expect(slugify("Ben Salâh")).toBe("Ben-Salah");
    expect(slugify("Ali Ben Salah")).toBe("Ali-Ben-Salah");
  });

  it("returns nothing for a name with no Latin script", () => {
    // Not an edge case: this app's shops are Tunisian and Arabic names are
    // ordinary. Callers must fall back — see `clientPart`.
    expect(slugify("علي بن صالح")).toBe("");
    expect(slugify("")).toBe("");
    expect(slugify("   ")).toBe("");
  });

  it("strips anything that could steer a path or a dialog", () => {
    // All of this reaches here from the client and purchase forms, which the
    // backend stores with only a trim applied.
    expect(slugify("../../etc/passwd")).toBe("etc-passwd");
    expect(slugify("C:\\Windows\\System32")).toBe("C-Windows-System32");
    expect(slugify("name\"with'quotes")).toBe("name-with-quotes");
    expect(slugify("line\nbreak\ttab")).toBe("line-break-tab");
    expect(slugify("null\u0000byte")).toBe("null-byte");
  });

  it("collapses and trims separators", () => {
    expect(slugify("  --Ali--  Ben  --")).toBe("Ali-Ben");
    expect(slugify("!!!")).toBe("");
  });

  it("caps a long name without leaving a dangling separator", () => {
    const long = slugify("a".repeat(200));
    expect(long.length).toBeLessThanOrEqual(48);

    // The slice must not be able to end on the separator it just cut through.
    const sliced = slugify(`${"a".repeat(47)} tail`);
    expect(sliced.endsWith("-")).toBe(false);
  });
});

describe("clientPart — the fallback that keeps a name meaningful", () => {
  it("uses the client's name when it survives slugging", () => {
    expect(clientPart("Ali Ben Salah", 12)).toBe("Ali-Ben-Salah");
  });

  it("falls back to the id when it does not", () => {
    expect(clientPart("علي بن صالح", 12)).toBe("client-12");
    expect(clientPart("", 3)).toBe("client-3");
  });
});

describe("documentFilename — the three documents", () => {
  it("builds the names the shop actually sees", () => {
    expect(documentFilename("Echeancier", "A-000001", clientPart("Ali Ben Salah", 12))).toBe(
      "Echeancier-A-000001-Ali-Ben-Salah",
    );
    expect(documentFilename("Recu", "A-000001", "T2", "2026-08-20")).toBe(
      "Recu-A-000001-T2-2026-08-20",
    );
    expect(documentFilename("Releve", clientPart("Ali Ben Salah", 12), "2026-08-20")).toBe(
      "Releve-Ali-Ben-Salah-2026-08-20",
    );
  });

  it("drops a part that slugged away instead of leaving a gap", () => {
    // `Releve--2026-08-20` is what a naive join produces, and it reads as a bug
    // to whoever receives the file.
    expect(documentFilename("Releve", slugify("علي"), "2026-08-20")).toBe("Releve-2026-08-20");
    expect(documentFilename("Recu", null, undefined, "2026-08-20")).toBe("Recu-2026-08-20");
  });
});
