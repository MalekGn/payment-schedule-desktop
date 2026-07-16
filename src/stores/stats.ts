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
    } catch {
      /* non-fatal for badges */
    }
  }

  return { overdueClients, overdueInstallments, refresh };
});
