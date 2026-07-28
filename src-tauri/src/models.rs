//! Serde-serializable data structures shared with the Vue frontend.
//!
//! Money is stored and transported as **whole currency units** (`i64`) — the
//! amount the shop keeper types in. This keeps the installment split exact
//! (integer division with the remainder placed on the last installment) and
//! avoids floating-point drift. Display formatting (currency symbol, grouping)
//! happens on the frontend according to the configured locale/currency.
//!
//! Dates are stored and transported as ISO-8601 `YYYY-MM-DD` strings.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub phone: String,
    pub address: String,
    pub email: Option<String>,
    pub created_at: String,
    /// `None` while the client is active; the ISO timestamp they were archived
    /// at otherwise. Archiving hides a client from the active list and the
    /// new-purchase picker without touching any of their history.
    pub archived_at: Option<String>,
}

/// Which slice of the client list to return. Serialized as
/// `"active" | "archived" | "all"`; absent means [`ClientScope::Active`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientScope {
    #[default]
    Active,
    Archived,
    All,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInput {
    pub first_name: String,
    pub last_name: String,
    pub phone: String,
    pub address: String,
    pub email: Option<String>,
}

/// A client row plus their aggregated purchase/balance figures.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSummary {
    #[serde(flatten)]
    pub client: Client,
    pub purchase_count: i64,
    pub total_outstanding: i64,
    pub overdue_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDetail {
    pub client: Client,
    /// Live purchases. The totals below are computed from these alone.
    pub purchases: Vec<PurchaseSummary>,
    /// Archived purchases, listed separately on the client page and excluded
    /// from every total — an archived purchase is no longer owed.
    pub archived_purchases: Vec<PurchaseSummary>,
    pub total_purchased: i64,
    pub total_paid: i64,
    pub total_outstanding: i64,
    pub overdue_count: i64,
}

// ---------------------------------------------------------------------------
// Purchase (Achat)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Purchase {
    pub id: i64,
    pub reference: String,
    pub client_id: i64,
    pub product_label: String,
    pub total_price: i64,
    pub installment_count: i64,
    pub interval_kind: String, // "weekly" | "monthly" | "custom"
    pub interval_days: Option<i64>,
    pub purchase_date: String,
    pub created_at: String,
    /// `None` while the purchase is live; the ISO date it was archived
    /// otherwise. An archived purchase leaves every money aggregate — see
    /// `m0003_purchase_archive` — and always has zero payments recorded.
    pub archived_at: Option<String>,
}

/// Which slice of the purchase list to return. Serialized as
/// `"active" | "archived" | "all"`; absent means [`PurchaseScope::Active`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PurchaseScope {
    #[default]
    Active,
    Archived,
    All,
}

/// A purchase row enriched with client name + computed status/balance,
/// used in list views and dashboards.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseSummary {
    pub id: i64,
    pub reference: String,
    pub client_id: i64,
    pub client_name: String,
    pub product_label: String,
    pub total_price: i64,
    pub paid_amount: i64,
    pub remaining: i64,
    pub installment_count: i64,
    pub purchase_date: String,
    /// "pending" | "in_progress" | "paid" | "late"
    pub status: String,
    pub overdue_count: i64,
    /// Mirrors [`Purchase::archived_at`]. Repeated rather than flattened
    /// because this struct is flat by design, unlike `ClientSummary`.
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallmentInput {
    // Sent by the frontend for clarity; the backend re-derives the position via
    // enumeration, so the field itself isn't read.
    #[allow(dead_code)]
    pub index: i64,
    pub amount: i64,
    pub due_date: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseInput {
    pub client_id: i64,
    pub product_label: String,
    pub total_price: i64,
    pub installment_count: i64,
    pub interval_kind: String,
    pub interval_days: Option<i64>,
    pub purchase_date: String,
    /// Optional manual per-installment override. When omitted, the backend
    /// computes an equal split with the remainder on the last installment.
    pub installments: Option<Vec<InstallmentInput>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseDetail {
    pub purchase: Purchase,
    pub client: Client,
    pub installments: Vec<Installment>,
    pub total_paid: i64,
    pub remaining: i64,
    /// "pending" | "in_progress" | "paid" | "late"
    pub status: String,
}

// ---------------------------------------------------------------------------
// Installment (Tranche)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installment {
    pub id: i64,
    pub purchase_id: i64,
    pub index: i64,
    pub amount: i64,
    pub due_date: String,
    pub paid_amount: i64,
    pub paid_date: Option<String>,
    /// Effective status computed against today's date:
    /// "paid" | "partial" | "late" | "pending"
    pub status: String,
}

// ---------------------------------------------------------------------------
// Payment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Payment {
    pub id: i64,
    pub installment_id: i64,
    pub installment_index: i64,
    pub purchase_id: i64,
    pub purchase_reference: String,
    pub client_id: i64,
    pub client_name: String,
    pub amount: i64,
    pub payment_date: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentInput {
    pub installment_id: i64,
    pub amount: i64,
    pub payment_date: String,
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Impayés (overdue) & dashboard
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverdueInstallment {
    pub installment_id: i64,
    pub purchase_id: i64,
    pub purchase_reference: String,
    pub index: i64,
    pub installment_count: i64,
    pub due_date: String,
    pub amount: i64,
    pub remaining: i64,
    pub days_late: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpayeClient {
    pub client_id: i64,
    pub client_name: String,
    pub phone: String,
    pub address: String,
    pub email: Option<String>,
    pub reference: String, // most-urgent purchase reference (for display)
    pub total_overdue: i64,
    pub overdue_count: i64,
    pub installments: Vec<OverdueInstallment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpayeFilter {
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub client_id: Option<i64>,
}

/// One installment enriched with client/purchase context for the schedule view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRow {
    pub installment_id: i64,
    pub purchase_id: i64,
    pub reference: String,
    pub client_id: i64,
    pub client_name: String,
    pub index: i64,
    pub installment_count: i64,
    pub due_date: String,
    pub amount: i64,
    pub paid_amount: i64,
    pub remaining: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub total_purchases: i64,
    pub total_sales: i64,
    pub total_collected: i64,
    pub total_outstanding: i64,
    pub overdue_count: i64,
    pub overdue_clients: i64,
    pub upcoming_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DueAlert {
    pub purchase_id: i64,
    pub reference: String,
    pub client_name: String,
    pub index: i64,
    pub installment_count: i64,
    pub due_date: String,
    pub days_late: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub stats: DashboardStats,
    pub recent_purchases: Vec<PurchaseSummary>,
    pub featured_purchase: Option<PurchaseDetail>,
    pub due_alerts: Vec<DueAlert>,
    pub impayes: Vec<ImpayeClient>,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub language: String,
    pub currency_code: String,
    pub date_format: String,
    pub logo_path: Option<String>,
    pub shop_name: String,
    pub shop_info: String,
    /// Horizon (in days) within which an upcoming unpaid installment is surfaced
    /// as a "due soon" alert on the Alertes page. Defaults to 7.
    pub alert_soon_days: i64,
    /// True until the user changes the language for the first time; lets the
    /// frontend apply OS-locale detection only on a genuinely fresh install.
    pub language_is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub language: Option<String>,
    pub currency_code: Option<String>,
    pub date_format: Option<String>,
    pub shop_name: Option<String>,
    pub shop_info: Option<String>,
    pub alert_soon_days: Option<i64>,
}
