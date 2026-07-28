// Navigate to the actual previous page. When the app was entered directly on
// this route (deep link, refresh) there is no in-app history to go back to, so
// we fall back to a sensible list route instead of leaving the user stranded.
//
// The same applies when the previous entry is itself an unknown URL: going back
// there would only swap one "page not found" screen for another. We treat that
// as having no usable history at all.
//
// Not handled here, by design: a stored entry that points at a *valid* detail
// route whose record has since been deleted (e.g. /achats/999 → `achat-detail`).
// The router cannot know the row is gone, so that page renders its own
// missing-record state — which carries this same back button, with a list route
// as its fallback. The user still has a way out.
import { useRouter } from "vue-router";

/**
 * Whether `router.back()` leads somewhere useful.
 *
 * Pure and router-free so it can be unit tested: the caller supplies both the
 * raw history entry and a way to resolve a path to its route name.
 *
 * @param back - `history.state.back`, which is `null` on a fresh document load.
 * @param resolveName - Maps a path to the name of the route that would match it.
 * @returns `true` to call `router.back()`, `false` to use the fallback route.
 */
export function shouldGoBack(back: unknown, resolveName: (path: string) => unknown): boolean {
  if (typeof back !== "string" || back === "") return false;
  try {
    return resolveName(back) !== "not-found";
  } catch {
    // A malformed stored entry is not somewhere we can navigate to.
    return false;
  }
}

/**
 * Returns a click handler that goes back to the previous in-app page, or pushes
 * `fallback` when there is no usable history.
 *
 * @param fallback - Route path to land on instead, e.g. `"/clients"`.
 */
export function useBack(fallback: string) {
  const router = useRouter();
  return () => {
    if (shouldGoBack(router.options.history.state.back, (path) => router.resolve(path).name)) {
      router.back();
    } else {
      router.push(fallback);
    }
  };
}
