// Navigate to the actual previous page. When the app was entered directly on
// this route (deep link, refresh) there is no in-app history to go back to, so
// we fall back to a sensible list route instead of leaving the user stranded.
import { useRouter } from "vue-router";

export function useBack(fallback: string) {
  const router = useRouter();
  return () => {
    if (router.options.history.state.back) {
      router.back();
    } else {
      router.push(fallback);
    }
  };
}
