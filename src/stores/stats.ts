import { defineStore } from "pinia";
import { ref } from "vue";
import { api } from "@/api";

// Lightweight global counters for the sidebar badges. Views call `refresh()`
// after any mutation (new purchase, recorded payment, deletion) so the badges
// stay in sync without prop drilling.
export const useStatsStore = defineStore("stats", () => {
  const overdueClients = ref(0);
  const overdueInstallments = ref(0);

  async function refresh() {
    try {
      const dash = await api.getDashboard();
      overdueClients.value = dash.stats.overdueClients;
      overdueInstallments.value = dash.stats.overdueCount;
    } catch (e) {
      // Non-fatal: the badges keep their last value rather than blocking the
      // mutation the caller just completed. But it must not be invisible —
      // silently frozen counters look identical to "nothing is overdue".
      console.error("stats.refresh failed; sidebar badges are stale:", e);
    }
  }

  return { overdueClients, overdueInstallments, refresh };
});
