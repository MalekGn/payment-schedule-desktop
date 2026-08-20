import { createRouter, createWebHistory, type RouteRecordRaw } from "vue-router";

// `meta.licensed` marks a route as a licensed feature. `App.vue` renders
// `LicenseRequiredPanel` in its place when the install has no valid licence —
// one gate site rather than a check inside each view.
//
// The routes *without* the flag are the unlicensed baseline: reading clients and
// purchases, plus Settings, which has to stay reachable or a user could never
// install the licence that unlocks the rest. This only decides what is shown;
// the Rust commands refuse on their own.
const routes: RouteRecordRaw[] = [
  {
    path: "/",
    name: "dashboard",
    component: () => import("@/views/DashboardView.vue"),
    meta: { licensed: true },
  },
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
  {
    path: "/paiements",
    name: "paiements",
    component: () => import("@/views/PaiementsView.vue"),
    meta: { licensed: true },
  },
  {
    path: "/echeances",
    name: "echeances",
    component: () => import("@/views/EcheancesView.vue"),
    meta: { licensed: true },
  },
  {
    path: "/impayes",
    name: "impayes",
    component: () => import("@/views/ImpayesView.vue"),
    meta: { licensed: true },
  },
  {
    path: "/alertes",
    name: "alertes",
    component: () => import("@/views/AlertesView.vue"),
    meta: { licensed: true },
  },
  {
    path: "/rapports",
    name: "rapports",
    component: () => import("@/views/RapportsView.vue"),
    meta: { licensed: true },
  },
  { path: "/parametres", name: "parametres", component: () => import("@/views/SettingsView.vue") },

  // Printable documents. `meta.print` makes `App.vue` render the route without
  // the sidebar and header — see the comment there for why these are their own
  // routes rather than a print stylesheet laid over the app.
  //
  // Licensed even though `get_purchase_detail` itself is not: producing the
  // shop's paperwork is a licensed feature, where the unlicensed baseline is
  // only reading your own ledger. The receipt and the statement need it
  // regardless, since the payment lists are already gated.
  {
    path: "/imprimer/echeancier/:id",
    name: "print-schedule",
    component: () => import("@/views/PrintView.vue"),
    props: (route) => ({ kind: "schedule", id: route.params.id }),
    meta: { licensed: true, print: true },
  },
  {
    // The payment arrives as `?payment=<id>` rather than a second path segment,
    // matching the `?client=` deep link the Impayés page already uses.
    path: "/imprimer/recu/:id",
    name: "print-receipt",
    component: () => import("@/views/PrintView.vue"),
    props: (route) => ({
      kind: "receipt",
      id: route.params.id,
      paymentId: route.query.payment,
    }),
    meta: { licensed: true, print: true },
  },
  {
    path: "/imprimer/releve/:id",
    name: "print-statement",
    component: () => import("@/views/PrintView.vue"),
    props: (route) => ({ kind: "statement", id: route.params.id }),
    meta: { licensed: true, print: true },
  },
  // Catch-all. The "not-found" name is matched by string elsewhere — `useBack`
  // (to avoid sending the user back into another unknown URL) and `AppHeader`'s
  // NAV_KEY (page title). Renaming it silently degrades both; grep before you do.
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
