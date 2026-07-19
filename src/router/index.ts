import { createRouter, createWebHistory, type RouteRecordRaw } from "vue-router";

const routes: RouteRecordRaw[] = [
  { path: "/", name: "dashboard", component: () => import("@/views/DashboardView.vue") },
  { path: "/achats", name: "achats", component: () => import("@/views/AchatsView.vue") },
  {
    path: "/achats/:id",
    name: "achat-detail",
    component: () => import("@/views/PurchaseDetailView.vue"),
    props: true,
  },
  { path: "/clients", name: "clients", component: () => import("@/views/ClientsView.vue") },
  {
    path: "/clients/:id",
    name: "client-detail",
    component: () => import("@/views/ClientDetailView.vue"),
    props: true,
  },
  { path: "/paiements", name: "paiements", component: () => import("@/views/PaiementsView.vue") },
  { path: "/echeances", name: "echeances", component: () => import("@/views/EcheancesView.vue") },
  { path: "/impayes", name: "impayes", component: () => import("@/views/ImpayesView.vue") },
  { path: "/alertes", name: "alertes", component: () => import("@/views/AlertesView.vue") },
  { path: "/rapports", name: "rapports", component: () => import("@/views/RapportsView.vue") },
  { path: "/parametres", name: "parametres", component: () => import("@/views/SettingsView.vue") },
  {
    path: "/:pathMatch(.*)*",
    name: "not-found",
    component: () => import("@/views/NotFoundView.vue"),
  },
];

export const router = createRouter({
  history: createWebHistory(),
  routes,
  scrollBehavior() {
    return { top: 0 };
  },
});
