// Single typed gateway to the backend. On the desktop app it forwards to the
// Rust Tauri commands via `invoke`; in a plain browser (dev preview / tests)
// it delegates to the in-memory mock so every screen stays functional.

import type {
  Client,
  ClientDetail,
  ClientInput,
  ClientSummary,
  Dashboard,
  ImpayeClient,
  ImpayeFilter,
  Payment,
  PaymentInput,
  PurchaseDetail,
  PurchaseInput,
  PurchaseSummary,
  ScheduleRow,
  Settings,
  SettingsPatch,
} from "@/types/models";
import { mockDb } from "./mock";

export const isTauri = (): boolean =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

async function openWithOs(url: string): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  return openUrl(url);
}

export const api = {
  // -- clients --
  listClients: (): Promise<ClientSummary[]> =>
    isTauri() ? invoke("list_clients") : Promise.resolve(mockDb.listClients()),
  getClientDetail: (id: number): Promise<ClientDetail> =>
    isTauri() ? invoke("get_client_detail", { id }) : Promise.resolve(mockDb.getClientDetail(id)),
  createClient: (input: ClientInput): Promise<Client> =>
    isTauri() ? invoke("create_client", { input }) : Promise.resolve(mockDb.createClient(input)),
  updateClient: (id: number, input: ClientInput): Promise<Client> =>
    isTauri()
      ? invoke("update_client", { id, input })
      : Promise.resolve(mockDb.updateClient(id, input)),
  deleteClient: (id: number, force: boolean): Promise<void> =>
    isTauri()
      ? invoke("delete_client", { id, force })
      : Promise.resolve(mockDb.deleteClient(id, force)),

  // -- purchases --
  listPurchases: (search?: string): Promise<PurchaseSummary[]> =>
    isTauri()
      ? invoke("list_purchases", { search: search ?? null })
      : Promise.resolve(mockDb.listPurchases(search)),
  getPurchaseDetail: (id: number): Promise<PurchaseDetail> =>
    isTauri()
      ? invoke("get_purchase_detail", { id })
      : Promise.resolve(mockDb.getPurchaseDetail(id)),
  createPurchase: (input: PurchaseInput): Promise<PurchaseDetail> =>
    isTauri()
      ? invoke("create_purchase", { input })
      : Promise.resolve(mockDb.createPurchase(input)),
  deletePurchase: (id: number): Promise<void> =>
    isTauri() ? invoke("delete_purchase", { id }) : Promise.resolve(mockDb.deletePurchase(id)),

  // -- payments --
  recordPayment: (input: PaymentInput): Promise<PurchaseDetail> =>
    isTauri() ? invoke("record_payment", { input }) : Promise.resolve(mockDb.recordPayment(input)),
  listPaymentsForPurchase: (purchaseId: number): Promise<Payment[]> =>
    isTauri()
      ? invoke("list_payments_for_purchase", { purchaseId })
      : Promise.resolve(mockDb.listPaymentsForPurchase(purchaseId)),
  listAllPayments: (limit = 500): Promise<Payment[]> =>
    isTauri()
      ? invoke("list_all_payments", { limit })
      : Promise.resolve(mockDb.listAllPayments(limit)),
  listPaymentsForClient: (clientId: number): Promise<Payment[]> =>
    isTauri()
      ? invoke("list_payments_for_client", { clientId })
      : Promise.resolve(mockDb.listPaymentsForClient(clientId)),

  // -- impayés / dashboard --
  listImpayes: (filter?: ImpayeFilter): Promise<ImpayeClient[]> =>
    isTauri()
      ? invoke("list_impayes", { filter: filter ?? null })
      : Promise.resolve(mockDb.listImpayes(filter)),
  listSchedule: (): Promise<ScheduleRow[]> =>
    isTauri() ? invoke("list_schedule") : Promise.resolve(mockDb.listSchedule()),
  getDashboard: (upcomingDays = 7): Promise<Dashboard> =>
    isTauri()
      ? invoke("get_dashboard", { upcomingDays })
      : Promise.resolve(mockDb.getDashboard(upcomingDays)),

  // -- settings --
  getSettings: (): Promise<Settings> =>
    isTauri() ? invoke("get_settings") : Promise.resolve(mockDb.getSettings()),
  updateSettings: (patch: SettingsPatch): Promise<Settings> =>
    isTauri()
      ? invoke("update_settings", { patch })
      : Promise.resolve(mockDb.updateSettings(patch)),
  setLogo: (sourcePath: string): Promise<Settings> =>
    isTauri() ? invoke("set_logo", { sourcePath }) : Promise.resolve(mockDb.setLogo(sourcePath)),
  clearLogo: (): Promise<Settings> =>
    isTauri() ? invoke("clear_logo") : Promise.resolve(mockDb.clearLogo()),

  /**
   * Write a consistent snapshot of the database to `dest`.
   *
   * The only recovery path the app has: deleting a client cascades through
   * their purchases, installments and payments, and cannot be undone.
   */
  backupDatabase: (dest: string): Promise<void> =>
    isTauri() ? invoke("backup_database", { dest }) : Promise.resolve(mockDb.backupDatabase(dest)),

  // -- system --
  /**
   * Hand a URI to the OS default handler (`tel:`, `sms:` — the capability scope
   * in `src-tauri/capabilities/default.json` allows nothing else).
   *
   * Never navigate the WebView to these schemes directly: it cannot load them
   * and replaces the whole SPA with its native error page. Rejects when the OS
   * has no handler registered, which callers must surface to the user.
   */
  openExternal: (url: string): Promise<void> =>
    isTauri() ? openWithOs(url) : Promise.resolve(mockDb.openExternal(url)),
};
