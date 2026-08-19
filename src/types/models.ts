// Mirror of the serde models in src-tauri/src/models.rs (camelCase payloads).
// Money values are whole currency units (integers). Dates are ISO YYYY-MM-DD.

export type InstallmentStatus = "pending" | "partial" | "paid" | "late";
export type PurchaseStatus = "pending" | "in_progress" | "paid" | "late";
export type IntervalKind = "weekly" | "monthly" | "custom";

/** Which slice of the client list to fetch. Defaults to `"active"`. */
export type ClientScope = "active" | "archived" | "all";

/** Which slice of the purchase list to fetch. Defaults to `"active"`. */
export type PurchaseScope = "active" | "archived" | "all";

export interface Client {
  id: number;
  firstName: string;
  lastName: string;
  phone: string;
  address: string;
  email: string | null;
  createdAt: string;
  /**
   * ISO date the client was archived, or `null` while they are active.
   * Archiving hides them from the active list and the new-purchase picker
   * without touching any of their history.
   */
  archivedAt: string | null;
}

export interface ClientInput {
  firstName: string;
  lastName: string;
  phone: string;
  address: string;
  email: string | null;
}

export interface ClientSummary extends Client {
  purchaseCount: number;
  totalOutstanding: number;
  overdueCount: number;
}

export interface ClientDetail {
  client: Client;
  /** Live purchases. The totals below are computed from these alone. */
  purchases: PurchaseSummary[];
  /** Archived purchases, shown separately and excluded from every total. */
  archivedPurchases: PurchaseSummary[];
  totalPurchased: number;
  totalPaid: number;
  totalOutstanding: number;
  overdueCount: number;
}

export interface Purchase {
  id: number;
  reference: string;
  clientId: number;
  productLabel: string;
  totalPrice: number;
  installmentCount: number;
  intervalKind: IntervalKind;
  intervalDays: number | null;
  purchaseDate: string;
  createdAt: string;
  /**
   * ISO date the purchase was archived, or `null` while it is live.
   * An archived purchase leaves every money view and always carries zero
   * payments — archiving is refused once any payment is recorded.
   */
  archivedAt: string | null;
}

export interface PurchaseSummary {
  id: number;
  reference: string;
  clientId: number;
  clientName: string;
  productLabel: string;
  totalPrice: number;
  paidAmount: number;
  remaining: number;
  installmentCount: number;
  purchaseDate: string;
  status: PurchaseStatus;
  overdueCount: number;
  /** Mirrors `Purchase.archivedAt`; repeated because this row is flat. */
  archivedAt: string | null;
}

export interface Installment {
  id: number;
  purchaseId: number;
  index: number;
  amount: number;
  dueDate: string;
  paidAmount: number;
  paidDate: string | null;
  status: InstallmentStatus;
}

export interface InstallmentInput {
  index: number;
  amount: number;
  dueDate: string;
}

/**
 * A partial edit of one existing installment.
 *
 * Every field is optional and an omitted one means "leave it alone", so the
 * editor sends only what the user actually touched.
 *
 * The two halves are governed by opposite rules. `amount` and `dueDate` are the
 * *schedule* — what is owed and when — and stay editable until the installment
 * settles. `paidAmount`, `paymentDate` and `note` are the *money*, editable
 * only in payment order.
 *
 * `paidAmount` is absolute, not an increment: it is the new cumulative total
 * collected on the row. The backend turns the difference into a `payment`
 * ledger entry, so `paymentDate` and `note` describe that entry rather than
 * writing `installment.paidDate`, which stays derived.
 */
export interface InstallmentEdit {
  amount?: number;
  dueDate?: string;
  paidAmount?: number;
  paymentDate?: string;
  note?: string;
}

export interface PurchaseInput {
  clientId: number;
  productLabel: string;
  totalPrice: number;
  installmentCount: number;
  intervalKind: IntervalKind;
  intervalDays: number | null;
  purchaseDate: string;
  installments?: InstallmentInput[] | null;
}

export interface PurchaseDetail {
  purchase: Purchase;
  client: Client;
  installments: Installment[];
  totalPaid: number;
  remaining: number;
  status: PurchaseStatus;
}

export interface Payment {
  id: number;
  installmentId: number;
  installmentIndex: number;
  purchaseId: number;
  purchaseReference: string;
  clientId: number;
  clientName: string;
  amount: number;
  paymentDate: string;
  note: string | null;
  createdAt: string;
}

export interface PaymentInput {
  installmentId: number;
  amount: number;
  paymentDate: string;
  note: string | null;
}

export interface OverdueInstallment {
  installmentId: number;
  purchaseId: number;
  purchaseReference: string;
  index: number;
  installmentCount: number;
  dueDate: string;
  amount: number;
  remaining: number;
  daysLate: number;
}

export interface ImpayeClient {
  clientId: number;
  clientName: string;
  phone: string;
  address: string;
  email: string | null;
  reference: string;
  totalOverdue: number;
  overdueCount: number;
  installments: OverdueInstallment[];
}

export interface ImpayeFilter {
  dateFrom?: string | null;
  dateTo?: string | null;
  clientId?: number | null;
}

export interface ScheduleRow {
  installmentId: number;
  purchaseId: number;
  reference: string;
  clientId: number;
  clientName: string;
  index: number;
  installmentCount: number;
  dueDate: string;
  amount: number;
  paidAmount: number;
  remaining: number;
  status: InstallmentStatus;
}

export interface DashboardStats {
  totalPurchases: number;
  totalSales: number;
  totalCollected: number;
  totalOutstanding: number;
  overdueCount: number;
  overdueClients: number;
  upcomingCount: number;
}

export interface DueAlert {
  purchaseId: number;
  reference: string;
  clientName: string;
  index: number;
  installmentCount: number;
  dueDate: string;
  daysLate: number;
}

export interface Dashboard {
  stats: DashboardStats;
  recentPurchases: PurchaseSummary[];
  featuredPurchase: PurchaseDetail | null;
  dueAlerts: DueAlert[];
  impayes: ImpayeClient[];
}

/**
 * Report period sizes. Mirrors `REPORT_GRANULARITIES` in `src-tauri/src/db.rs`.
 */
export const REPORT_GRANULARITIES = ["day", "month", "year"] as const;
export type ReportGranularity = (typeof REPORT_GRANULARITIES)[number];

/**
 * Aging buckets, in the order the backend reports them. Mirrors
 * `AGING_BUCKETS` in `src-tauri/src/commands.rs`; each has a
 * `rapports.aging.<key>` label in every locale file.
 */
export const AGING_BUCKETS = ["current", "1-30", "31-60", "61-90", "90+"] as const;
export type AgingBucketKey = (typeof AGING_BUCKETS)[number];

export interface ReportInput {
  dateFrom: string;
  dateTo: string;
  /** Absent means "let the backend pick one from the span". */
  granularity?: ReportGranularity;
}

export interface ReportRange {
  from: string;
  to: string;
  /**
   * The date the balance figures were taken at — always today. Echoed back so
   * the UI can label them separately from the period figures, which are
   * genuinely historical. See the `Rapports` comment in `src-tauri/src/models.rs`.
   */
  asOf: string;
  /** The granularity actually used, resolved when none was sent. */
  granularity: ReportGranularity;
}

/** Period figures (historical) and balance figures (as of `asOf`), together. */
export interface ReportTotals {
  salesCount: number;
  salesAmount: number;
  collected: number;
  /** Ledger entries, which includes signed correction rows. */
  paymentCount: number;
  outstandingNow: number;
  overdueNow: number;
  newClients: number;
}

export interface PeriodPoint {
  /** `YYYY-MM-DD`, `YYYY-MM` or `YYYY` — fixed width, so it sorts. */
  period: string;
  collected: number;
  /** What fell due in the period, paid or not. */
  due: number;
}

export interface AgingBucket {
  bucket: AgingBucketKey;
  count: number;
  amount: number;
}

export interface ClientRisk {
  clientId: number;
  clientName: string;
  outstanding: number;
  overdue: number;
  overdueCount: number;
}

export interface ProductLine {
  productLabel: string;
  purchaseCount: number;
  totalAmount: number;
}

export interface Report {
  range: ReportRange;
  totals: ReportTotals;
  collections: PeriodPoint[];
  aging: AgingBucket[];
  topClients: ClientRisk[];
  topProducts: ProductLine[];
}

export interface Settings {
  language: string;
  currencyCode: string;
  dateFormat: string;
  logoPath: string | null;
  shopName: string;
  shopInfo: string;
  /** Horizon (in days) for "due soon" alerts on the Alertes page. */
  alertSoonDays: number;
  languageIsDefault: boolean;
  /**
   * ISO date of the last successful backup, or `null` if this install has never
   * taken one. Read-only — written by `backupDatabase` on the Rust side, which
   * is why it has no counterpart in {@link SettingsPatch}.
   */
  lastBackupAt: string | null;
  /**
   * ISO date of the last automatic snapshot taken at launch, or `null` if none
   * has run. Read-only, and deliberately **not** interchangeable with
   * {@link Settings.lastBackupAt}: automatic copies sit beside the database on
   * the same disk, so they never stand in for one the user took off the machine.
   */
  lastAutoBackupAt: string | null;
  /** Whether the scheduled automatic backup runs at all. On by default. */
  autoBackupEnabled: boolean;
  /** `"daily" | "weekly" | "monthly"` — see `BACKUP_FREQUENCIES`. */
  autoBackupFrequency: string;
  /** Time of day the automatic backup is due, `HH:MM` in local time. */
  autoBackupTime: string;
}

export interface SettingsPatch {
  language?: string;
  currencyCode?: string;
  dateFormat?: string;
  shopName?: string;
  shopInfo?: string;
  alertSoonDays?: number;
  /**
   * The backup schedule is writable, unlike the two `last*At` stamps on
   * {@link Settings}: it is configuration, not a record of what happened.
   */
  autoBackupEnabled?: boolean;
  autoBackupFrequency?: string;
  autoBackupTime?: string;
}

// ---------------------------------------------------------------------------
// Licence
// ---------------------------------------------------------------------------

/**
 * Verdict on the installed licence. Mirrors the tags produced by
 * `LicenseStatus::tag` in `src-tauri/src/license.rs` — a wire contract, so
 * renaming a member here means renaming it there.
 *
 * Only `"valid"` grants access. `"clockTampered"` means the system date reads
 * earlier than the latest date this install has seen.
 */
export type LicenseStatusTag =
  | "valid"
  | "expired"
  | "machineMismatch"
  | "invalidSignature"
  | "malformed"
  | "missing"
  | "clockTampered";

/** A licence whose signature has verified. Every field is vendor-attested. */
export interface License {
  licenseId: string;
  licensee: string;
  issuedAt: string;
  expiresAt: string;
  /** Fingerprint this licence is bound to, or null for a floating licence. */
  machineId: string | null;
  /** Reserved for later feature gating; `["*"]` means everything. */
  features: string[];
}

export interface LicenseInfo {
  status: LicenseStatusTag;
  /** Present only when the signature verified, so it can be trusted. */
  license: License | null;
  /** ISO date, set only when `status` is `"expired"`. */
  expiredOn: string | null;
  /** **This machine's** fingerprint, which the customer reports to get a licence. */
  machineId: string | null;
}
