// Single typed gateway to the backend. On the desktop app it forwards to the
// Rust Tauri commands via `invoke`; in a plain browser (dev preview / tests)
// it delegates to the in-memory mock so every screen stays functional.

import type {
  BackupEntry,
  Client,
  ClientDetail,
  ClientInput,
  ClientScope,
  ClientSummary,
  Dashboard,
  ImpayeClient,
  ImpayeFilter,
  InstallmentEdit,
  LicenseInfo,
  Payment,
  PaymentInput,
  PurchaseDetail,
  PurchaseInput,
  PurchaseScope,
  PurchaseSummary,
  Report,
  ReportInput,
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

async function setNativeWindowTitle(title: string): Promise<void> {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().setTitle(title);
}

async function openWithOs(url: string): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  return openUrl(url);
}

/**
 * Ask for a destination, then have the backend write the CSV there.
 *
 * The desktop half of {@link api.saveCsv}. A `Blob` and an `<a download>` — what
 * the export buttons used to do — is a browser mechanism with no counterpart in
 * the WebView, so the click did nothing at all in the shipped app. The renderer
 * cannot write the file itself either: there is deliberately no `fs` plugin. So
 * this mirrors the backup flow — the dialog yields a path, `export_csv` writes
 * it.
 *
 * Resolves `false` when the user dismisses the dialog, which is a cancellation
 * and not a failure; the caller should stay quiet about it.
 */
async function writeCsvToChosenPath(suggestedName: string, contents: string): Promise<boolean> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  const dest = await save({
    defaultPath: suggestedName,
    filters: [{ name: "CSV", extensions: ["csv"] }],
  });
  if (typeof dest !== "string") return false;
  await invoke<void>("export_csv", { dest, contents });
  return true;
}

/**
 * Subscribe to a backend-pushed Tauri event, resolving to the unsubscribe.
 *
 * The counterpart to `invoke`, and the only other direction the boundary runs
 * in. Lazily imported for the same reason: `@tauri-apps/api` must not be pulled
 * into the browser bundle that the mock serves.
 */
async function listenTo<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => handler(e.payload));
}

/** Rust emits this when the licence verdict changes. See `commands.rs`. */
const LICENSE_CHANGED_EVENT = "license://changed";

export const api = {
  // -- clients --
  /**
   * List clients, active ones only unless a wider `scope` is asked for.
   *
   * The default is what makes archived clients disappear from every caller
   * that does not opt in — notably the new-purchase client picker, so an
   * archived client cannot take on new debt.
   */
  listClients: (scope: ClientScope = "active"): Promise<ClientSummary[]> =>
    isTauri() ? invoke("list_clients", { scope }) : Promise.resolve(mockDb.listClients(scope)),
  getClientDetail: (id: number): Promise<ClientDetail> =>
    isTauri() ? invoke("get_client_detail", { id }) : Promise.resolve(mockDb.getClientDetail(id)),
  createClient: (input: ClientInput): Promise<Client> =>
    isTauri() ? invoke("create_client", { input }) : Promise.resolve(mockDb.createClient(input)),
  updateClient: (id: number, input: ClientInput): Promise<Client> =>
    isTauri()
      ? invoke("update_client", { id, input })
      : Promise.resolve(mockDb.updateClient(id, input)),
  /**
   * Hide a client from the active list, keeping every purchase, installment
   * and payment. Rejects with `ARCHIVE_HAS_OUTSTANDING:{remaining}` while they
   * still owe money.
   */
  archiveClient: (id: number): Promise<void> =>
    isTauri() ? invoke("archive_client", { id }) : Promise.resolve(mockDb.archiveClient(id)),
  restoreClient: (id: number): Promise<void> =>
    isTauri() ? invoke("restore_client", { id }) : Promise.resolve(mockDb.restoreClient(id)),
  /**
   * Delete a client outright. Only ever succeeds for a client with no
   * purchases; anyone with history rejects with `CLIENT_HAS_PURCHASES:{n}` and
   * must be archived instead.
   */
  deleteClient: (id: number): Promise<void> =>
    isTauri() ? invoke("delete_client", { id }) : Promise.resolve(mockDb.deleteClient(id)),

  // -- purchases --
  /**
   * List purchases, live ones only unless a wider `scope` is asked for.
   *
   * The default is what keeps archived purchases out of every caller that does
   * not opt in — and, more importantly, out of every total.
   */
  listPurchases: (scope: PurchaseScope = "active", search?: string): Promise<PurchaseSummary[]> =>
    isTauri()
      ? invoke("list_purchases", { scope, search: search ?? null })
      : Promise.resolve(mockDb.listPurchases(scope, search)),
  getPurchaseDetail: (id: number): Promise<PurchaseDetail> =>
    isTauri()
      ? invoke("get_purchase_detail", { id })
      : Promise.resolve(mockDb.getPurchaseDetail(id)),
  createPurchase: (input: PurchaseInput): Promise<PurchaseDetail> =>
    isTauri()
      ? invoke("create_purchase", { input })
      : Promise.resolve(mockDb.createPurchase(input)),
  /**
   * Edit a purchase, and the **only** way to change an installment's `amount`
   * or `dueDate`. The product label is always accepted; the schedule is applied
   * onto the stored rows position by position, so a purchase carrying payments
   * can still have its unpaid installments moved.
   *
   * A settled installment is history and refuses the edit (`AMOUNT_LOCKED`,
   * `DUE_DATE_LOCKED`), no row may fall below what it has collected
   * (`BELOW_PAID:{paidAmount}`), and shortening the schedule past a row that
   * carries cash rejects with `PURCHASE_HAS_PAYMENTS:{n}`. Also rejects
   * `SUM_MISMATCH:{sum}:{total}` and `DUE_DATE_OUT_OF_ORDER` (due dates must
   * run in position order). `clientId` is ignored — a purchase cannot change
   * hands.
   */
  updatePurchase: (id: number, input: PurchaseInput): Promise<PurchaseDetail> =>
    isTauri()
      ? invoke("update_purchase", { id, input })
      : Promise.resolve(mockDb.updatePurchase(id, input)),
  /**
   * Remove a purchase from every list and every total, reversibly. Rejects
   * with `PURCHASE_HAS_PAYMENTS:{n}` once cash has been recorded against it.
   */
  archivePurchase: (id: number): Promise<void> =>
    isTauri() ? invoke("archive_purchase", { id }) : Promise.resolve(mockDb.archivePurchase(id)),
  restorePurchase: (id: number): Promise<void> =>
    isTauri() ? invoke("restore_purchase", { id }) : Promise.resolve(mockDb.restorePurchase(id)),
  /**
   * Destroy an archived purchase for good. Only ever succeeds once it has been
   * archived (`PURCHASE_NOT_ARCHIVED` otherwise), which makes the two-step real
   * rather than something the UI could forget.
   */
  deletePurchase: (id: number): Promise<void> =>
    isTauri() ? invoke("delete_purchase", { id }) : Promise.resolve(mockDb.deletePurchase(id)),

  // -- installments --
  /**
   * Record money against one installment. Omitted fields are left alone.
   *
   * This deals only in money — `paidAmount`, `paymentDate`, `note`. The
   * schedule belongs to `updatePurchase`, so sending `amount` or `dueDate` here
   * rejects with `SCHEDULE_VIA_PURCHASE` whatever their values.
   *
   * `paidAmount` is the new cumulative total collected, editable only once the
   * previous installment is fully paid (`PREVIOUS_UNPAID:{index}`) and capped at
   * the installment's amount (`PAID_ABOVE_AMOUNT:{amount}`). Moving it writes a
   * correction entry into the payment ledger, which is what keeps
   * `SUM(payments)` in step with it.
   *
   * `paymentDate` dates that new entry and nothing else: once an entry is on
   * record its date is history (`PAYMENT_DATE_LOCKED`), and a date or note with
   * no entry to carry it rejects with `NO_PAYMENT_TO_DATE` rather than being
   * dropped. A future date rejects with `FUTURE_PAID_DATE`.
   */
  updateInstallment: (id: number, edit: InstallmentEdit): Promise<PurchaseDetail> =>
    isTauri()
      ? invoke("update_installment", { id, edit })
      : Promise.resolve(mockDb.updateInstallment(id, edit)),

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

  // -- rapports --
  /**
   * Aggregated figures over a date range.
   *
   * Aggregation happens in the Rust core rather than here for a reason worth
   * not undoing: `listAllPayments` caps at 500 rows, so totalling it in the
   * renderer would under-report revenue on any shop past its five-hundredth
   * payment, and do it silently.
   */
  getReport: (input: ReportInput): Promise<Report> =>
    isTauri() ? invoke("get_report", { input }) : Promise.resolve(mockDb.getReport(input)),

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
   * Still the only recovery path the app has, but a much narrower one now:
   * neither a client with history nor a purchase carrying a payment can be
   * deleted at all, and a purchase must be archived before it can be
   * destroyed. What a backup still protects is that final, deliberate step.
   */
  backupDatabase: (dest: string): Promise<Settings> =>
    isTauri() ? invoke("backup_database", { dest }) : Promise.resolve(mockDb.backupDatabase(dest)),

  /**
   * Every snapshot the app has taken, newest first.
   *
   * Never rejects on an unreadable `backups/` — the Rust side answers with an
   * empty list — because the Settings page must still render the file picker,
   * which does not need that directory at all.
   */
  listBackups: (): Promise<BackupEntry[]> =>
    isTauri() ? invoke("list_backups") : Promise.resolve(mockDb.listBackups()),

  /**
   * Replace the working database with the snapshot at `source`.
   *
   * The counterpart to {@link api.backupDatabase}, and the reason it was worth
   * writing: until this existed the app could produce verified snapshots and
   * never read one back.
   *
   * Rejects with `INVALID_BACKUP_FILE` when the file is not a usable
   * paymentSchedule database — checked *before* anything is replaced — and with
   * `RESTORE_FAILED` when the swap itself could not complete, in which case the
   * current data is untouched.
   *
   * **The caller must reload the WebView on success.** Every store, route and
   * computed in the app is derived from a database that no longer exists; the
   * returned {@link Settings} are the restored ones (the snapshot may carry a
   * different language or currency) and are only good for the reload.
   */
  restoreDatabase: (source: string): Promise<Settings> =>
    isTauri()
      ? invoke("restore_database", { source })
      : Promise.resolve(mockDb.restoreDatabase(source)),

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

  /**
   * Rename the application window.
   *
   * Used by the printable documents. The print dialog derives the file name it
   * suggests from the document title, and on Linux the GTK print job commonly
   * takes its name from the parent window instead — so both are set, and this is
   * the half that needs to cross the Tauri boundary. `core:window:allow-set-title`
   * is already granted in `capabilities/default.json`.
   */
  setWindowTitle: (title: string): Promise<void> =>
    isTauri() ? setNativeWindowTitle(title) : Promise.resolve(mockDb.setWindowTitle(title)),

  /**
   * Save a CSV the caller has already rendered.
   *
   * Resolves `true` when a file was written and `false` when the user cancelled
   * the save dialog — so a caller can tell "done" from "never mind" without
   * catching anything. A genuine failure still rejects, with `EXPORT_FAILED`.
   */
  saveCsv: (suggestedName: string, contents: string): Promise<boolean> =>
    isTauri()
      ? writeCsvToChosenPath(suggestedName, contents)
      : Promise.resolve(mockDb.saveCsv(suggestedName, contents)),

  // -- licence --
  /**
   * The current licence verdict. Never rejects: "no licence installed" is a
   * status to render, not an error to catch.
   *
   * This drives what the UI shows, but it is not what enforces anything — the
   * gated commands refuse on their own in Rust. Hiding a control here is a
   * courtesy to the user, not a security boundary.
   */
  getLicenseStatus: (): Promise<LicenseInfo> =>
    isTauri() ? invoke("get_license_status") : Promise.resolve(mockDb.getLicenseStatus()),

  /**
   * Validate the licence file at `sourcePath` and, only if it is valid, install
   * it into the app-data directory.
   *
   * Rejects with `INVALID_LICENSE:{status}` when the file is not a valid
   * licence for this machine, leaving any existing licence untouched.
   */
  importLicense: (sourcePath: string): Promise<LicenseInfo> =>
    isTauri()
      ? invoke("import_license", { sourcePath })
      : Promise.resolve(mockDb.importLicense(sourcePath)),

  /**
   * Watch for licence verdicts the backend pushes on its own.
   *
   * The backend re-evaluates the licence periodically, so an expiry takes effect
   * while the app is running rather than at the next launch. This is how the UI
   * hears about it; without it the screen would keep claiming the install is
   * licensed while every gated command refused.
   *
   * The only subscription in the gateway. Resolves to an unsubscribe function —
   * call it, or the handler outlives whatever registered it.
   */
  onLicenseChanged: (handler: (info: LicenseInfo) => void): Promise<() => void> =>
    isTauri()
      ? listenTo<LicenseInfo>(LICENSE_CHANGED_EVENT, handler)
      : Promise.resolve(mockDb.onLicenseChanged(handler)),
};
