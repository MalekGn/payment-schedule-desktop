import { convertFileSrc } from "@tauri-apps/api/core";
import { isTauri } from "@/api";

/**
 * Turn a stored logo path into a browser-usable image src.
 * - Tauri: a filesystem path → asset-protocol URL via convertFileSrc.
 * - Browser mock: already a data URL (or empty) → return as-is.
 */
export function resolveLogoSrc(path: string | null | undefined): string | null {
  if (!path) return null;
  return isTauri() ? convertFileSrc(path) : path;
}
