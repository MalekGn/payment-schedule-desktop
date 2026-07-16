// Mirror of the serde models in src-tauri/src/models.rs (camelCase payloads).
// Money values are whole currency units (integers). Dates are ISO YYYY-MM-DD.

export type InstallmentStatus = "pending" | "partial" | "paid" | "late";
export type PurchaseStatus = "pending" | "in_progress" | "paid" | "late";
export type IntervalKind = "weekly" | "monthly" | "custom";

export interface Client {
  id: number;
  firstName: string;
  lastName: string;
  phone: string;
  address: string;
  email: string | null;
  createdAt: string;
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
  purchases: PurchaseSummary[];
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

export interface Settings {
  language: string;
  currencyCode: string;
  dateFormat: string;
  logoPath: string | null;
  shopName: string;
  shopInfo: string;
  languageIsDefault: boolean;
}

export interface SettingsPatch {
  language?: string;
  currencyCode?: string;
  dateFormat?: string;
  shopName?: string;
  shopInfo?: string;
}
