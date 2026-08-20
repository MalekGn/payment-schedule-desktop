// Opening the OS print dialog, once the page is actually ready to be printed.
//
// `window.print()` captures the document as it stands at the moment it is
// called. Calling it too early is the classic way to ship a letterhead with a
// blank box where the logo should be: the image request is still in flight, the
// print snapshot is taken without it, and nothing on screen ever looks wrong.
// So this waits for the DOM update, the webfonts, and every image in the
// document to decode before handing over.

import { nextTick } from "vue";

/**
 * Wait until the document can be painted in full.
 *
 * Every step is optional by design — `document.fonts` is absent in older
 * engines, and `img.decode()` rejects for an image that failed to load, which is
 * a reason to print anyway rather than to block. The point is to give the
 * browser its best chance, never to prevent printing.
 */
async function whenPaintable(): Promise<void> {
  await nextTick();
  try {
    await document.fonts?.ready;
  } catch {
    // Fonts are cosmetic here; a failure must not stop the print.
  }
  const images = [...document.querySelectorAll("img")];
  await Promise.all(
    images.map((img) =>
      // A broken logo still prints — as the alt text — so a rejection here is
      // "carry on", not "abort".
      img.decode().catch(() => undefined),
    ),
  );
}

export interface Printer {
  /** Open the OS print dialog once the document is ready. Never throws. */
  print: () => Promise<void>;
}

export function usePrint(): Printer {
  async function print(): Promise<void> {
    await whenPaintable();
    // Guarded because `window.print` is absent under Vitest's jsdom, where the
    // action bar is still rendered and clickable.
    if (typeof window.print === "function") window.print();
  }

  return { print };
}
