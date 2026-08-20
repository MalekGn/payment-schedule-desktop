// Building the name a printed document suggests when it is saved.
//
// # Why this exists
//
// The print dialog derives the suggested file name from the document title. This
// app never set one, so every document — schedule, receipt, statement — was
// offered as the print engine's own fallback, `output.pdf`. A shop printing a
// week of paperwork ended up with `output.pdf`, `output(1).pdf`, `output(2).pdf`
// and no way to tell them apart without opening each.
//
// The names built here are deliberately ASCII and locale-independent, matching
// the CSV exports which already ship `impayes-…` and `rapport-…` whatever the UI
// language is. A file name that changes script when the user switches to Arabic
// makes a folder of documents unsortable and unsearchable.

/**
 * Longest a single slugged part may be.
 *
 * Product labels and client names are bounded at 120 characters by the backend
 * (`SHORT_TEXT_MAX`), and several parts are joined into one name, so a cap per
 * part keeps the whole comfortably inside the ~255-byte limit every filesystem
 * this ships on enforces.
 */
const PART_MAX = 48;

/**
 * Reduce operator-entered text to something safe to put in a file name.
 *
 * Returns `""` when nothing survives, which is a real and ordinary case here
 * rather than an edge one: an Arabic client name has no ASCII to keep, and this
 * app's shops are Tunisian. Callers must have a fallback — see
 * {@link clientPart}.
 *
 * Decomposing first (`NFD`) and then dropping the combining marks is what turns
 * `Réfrigérateur` into `Refrigerateur` rather than into `Rfrigrateur`.
 */
export function slugify(value: string): string {
  return (
    value
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "")
      // Everything else goes, not just the characters a path treats specially:
      // this text comes from the client form, and an allow-list is the only way to
      // be sure a name cannot steer a path or smuggle a control character into a
      // dialog.
      .replace(/[^A-Za-z0-9]+/g, "-")
      .replace(/-+/g, "-")
      .replace(/^-|-$/g, "")
      .slice(0, PART_MAX)
      // Slicing can leave a trailing separator behind.
      .replace(/-$/, "")
  );
}

/**
 * How a client is named in a file name, with the fallback that keeps the name
 * meaningful when their name is not Latin script.
 */
export function clientPart(name: string, id: number): string {
  return slugify(name) || `client-${id}`;
}

/**
 * Join the parts of a document name, dropping any that slugged away.
 *
 * Dropping rather than keeping empties is the point: `Releve--2026-08-20` is
 * what a naive join produces for an Arabic-named client, and it looks like a
 * bug to whoever receives the file.
 */
export function documentFilename(...parts: (string | number | null | undefined)[]): string {
  return parts
    .map((p) => (p == null ? "" : slugify(String(p))))
    .filter((p) => p !== "")
    .join("-");
}
