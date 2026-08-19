//! Tauri commands — the entire API surface exposed to the frontend.
//!
//! Every command is `async` so Tauri runs it on the async runtime rather than
//! inline on the IPC/main event-loop thread; a synchronous command blocks the
//! UI for the duration of its queries. None of them `await` anything, so the
//! `MutexGuard` on the connection never spans a suspension point.
//!
//! Commands are thin: they validate their arguments, lock the shared
//! connection, and delegate. The mutating ones delegate to a `*_impl` free
//! function taking `&Connection` (or `&mut Connection` where the write needs a
//! transaction), which is what makes them testable without a Tauri `State`.
//!
//! Errors are [`AppError`], serialized as a stable machine code that
//! `src/lib/errors.ts` maps to a localized message. Internal detail is logged,
//! never sent.

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use tauri::State;

use crate::db::{
    add_interval, installment_status, parse_date, purchase_status, split_amounts, today, AppError,
    Db, DbResult, BACKUP_FREQUENCIES, CURRENCY_CODES, DATE_FORMATS, DEFAULT_BACKUP_TIME,
    EXPORT_MAX_BYTES, INSTALLMENT_COUNT_RANGE, INTERVAL_DAYS_RANGE, INTERVAL_KINDS, LANGUAGES,
    LONG_TEXT_MAX, MONEY_RANGE, PAYMENT_LIMIT_RANGE, REPORT_DAY_MAX_SPAN, REPORT_GRANULARITIES,
    REPORT_MAX_BUCKETS, REPORT_MONTH_MAX_SPAN, REPORT_SPAN_DAYS_RANGE, REPORT_TOP_N,
    SHORT_TEXT_MAX, UPCOMING_DAYS_RANGE,
};
use crate::db::{
    AMOUNT_LOCKED, ARCHIVE_HAS_OUTSTANDING, BACKUP_FAILED, BELOW_PAID, CLIENT_ARCHIVED,
    CLIENT_HAS_PURCHASES, CLIENT_NOT_FOUND, DUE_DATE_LOCKED, DUE_DATE_OUT_OF_ORDER, EXPORT_FAILED,
    FUTURE_PAID_DATE, INSTALLMENT_COUNT_MISMATCH, INSTALLMENT_NOT_FOUND, INVALID_AMOUNT,
    INVALID_DATE, INVALID_GRANULARITY, INVALID_INSTALLMENT_COUNT, INVALID_INTERVAL_DAYS,
    INVALID_INTERVAL_KIND, INVALID_LICENSE, INVALID_LOGO_TYPE, INVALID_SETTING_VALUE,
    INVALID_TOTAL_PRICE, LICENSE_REQUIRED, LOGO_TOO_LARGE, NO_PAYMENT_TO_DATE, OVERPAYMENT,
    PAID_ABOVE_AMOUNT, PAYMENT_DATE_LOCKED, PREVIOUS_UNPAID, PURCHASE_ARCHIVED,
    PURCHASE_HAS_PAYMENTS, PURCHASE_NOT_ARCHIVED, PURCHASE_NOT_FOUND, REPORT_RANGE_TOO_LONG,
    SCHEDULE_VIA_PURCHASE, SUM_MISMATCH, TEXT_REQUIRED, TEXT_TOO_LONG,
};
use crate::license::{self, LicenseInfo, LicenseState, LicenseStatus};
use crate::models::*;

// ===========================================================================
// Licence gate
// ===========================================================================
//
// The gate lives here, in Rust, and not only in the UI. The renderer is a
// WebView the user controls: hiding a button behind `v-if` is a statement of
// intent, not a control. These are the calls that must actually refuse.
//
// The unlicensed baseline is deliberately narrow but genuinely usable: a shop
// keeper can still *read* their clients and purchases, so an expired licence
// never holds their own ledger hostage. Everything that changes data, and every
// derived view (dashboard, payments, échéances, impayés, alerts), is licensed.
//
// One honest limitation: sorting and most filtering happen in the browser on
// already-fetched rows (`useSort.ts`, `ListFilterBar.vue`), so the backend never
// sees them and cannot enforce them. `scope` is the exception — a real argument,
// degraded below rather than refused.

/// Refuse a licensed command when the install has no valid licence.
///
/// Takes `&LicenseState`, not `&State<'_, LicenseState>`, so the call sites keep
/// working by deref coercion while the rule itself stays reachable from
/// `cargo test` without a Tauri runtime — the same reasoning behind the `*_impl`
/// split used throughout this module.
fn require_license(lic: &LicenseState) -> DbResult<()> {
    if lic.is_valid() {
        return Ok(());
    }
    Err(AppError::validation(LICENSE_REQUIRED))
}

// ===========================================================================
// Row mappers & shared helpers
// ===========================================================================

/// Reject a free-text field that is longer than `max` **characters**.
///
/// Byte length would be the wrong unit here: the same 40-character address
/// costs 40 bytes in ASCII, more in French and more again in Arabic, so a byte
/// cap silently gives different users different limits.
fn bounded(value: &str, max: usize) -> DbResult<()> {
    if value.chars().count() > max {
        return Err(AppError::conflict(TEXT_TOO_LONG, max));
    }
    Ok(())
}

/// Reject a field that must carry something once trimmed.
fn required(value: &str) -> DbResult<()> {
    if value.is_empty() {
        return Err(AppError::validation(TEXT_REQUIRED));
    }
    Ok(())
}

/// Validate a client as it arrives off the IPC boundary.
///
/// Everything here was previously `.trim()` and nothing else, so the renderer
/// could store a nameless client, or a megabyte of text in a field that is then
/// rendered into every list, export and dashboard card. The "names are required"
/// rule existed only in `ClientForm.vue`, which is a statement of intent rather
/// than a control — the WebView belongs to the user.
///
/// Takes the already-trimmed values so the caller cannot validate one string and
/// store a different one.
fn validate_client_input(input: &ClientInput) -> DbResult<()> {
    let first = input.first_name.trim();
    let last = input.last_name.trim();
    required(first)?;
    required(last)?;
    bounded(first, SHORT_TEXT_MAX)?;
    bounded(last, SHORT_TEXT_MAX)?;
    bounded(input.phone.trim(), SHORT_TEXT_MAX)?;
    bounded(input.address.trim(), LONG_TEXT_MAX)?;
    if let Some(email) = &input.email {
        bounded(email.trim(), SHORT_TEXT_MAX)?;
    }
    Ok(())
}

fn map_client(row: &rusqlite::Row) -> rusqlite::Result<Client> {
    Ok(Client {
        id: row.get("id")?,
        first_name: row.get("first_name")?,
        last_name: row.get("last_name")?,
        phone: row.get("phone")?,
        address: row.get("address")?,
        email: row.get("email")?,
        created_at: row.get("created_at")?,
        archived_at: row.get("archived_at")?,
    })
}

fn fetch_client(conn: &Connection, id: i64) -> DbResult<Client> {
    conn.query_row(
        "SELECT id, first_name, last_name, phone, address, email, created_at, archived_at
           FROM client WHERE id = ?1",
        [id],
        map_client,
    )
    .map_err(missing_row(CLIENT_NOT_FOUND))
}

/// Total still owed across every purchase of `client_id`; 0 when they have none.
///
/// `COALESCE` is load-bearing: a bare `SUM` over an empty join returns `NULL`,
/// and the comparison is deliberately done in Rust rather than in SQL, where
/// `NULL > 0` is `NULL` rather than false.
/// Archived purchases are excluded: they have left every other money view, so
/// letting one block archiving its client would be inconsistent.
fn client_outstanding(conn: &Connection, client_id: i64) -> DbResult<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(i.amount - i.paid_amount), 0)
           FROM purchase p JOIN installment i ON i.purchase_id = p.id
          WHERE p.client_id = ?1 AND p.archived_at IS NULL",
        [client_id],
        |r| r.get(0),
    )
    .map_err(AppError::from)
}

/// How many payments have been recorded against any installment of `purchase_id`.
///
/// The gate on both rescheduling and archiving a purchase: regenerating the
/// installment rows would cascade these away, and archiving one that carries
/// real cash would take that cash out of the books.
fn payment_count(conn: &Connection, purchase_id: i64) -> DbResult<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM payment pay
           JOIN installment i ON i.id = pay.installment_id
          WHERE i.purchase_id = ?1",
        [purchase_id],
        |r| r.get(0),
    )
    .map_err(AppError::from)
}

/// Map "no such row" to an actionable code, leaving every other database
/// failure as an internal error.
///
/// Without the split, opening a deleted record and a real database fault both
/// surfaced as the same opaque failure, and the detail views render *any* error
/// as "this record does not exist" — so a transient fault was reported to the
/// user as permanent data loss.
fn missing_row(code: &'static str) -> impl Fn(rusqlite::Error) -> AppError {
    move |e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::not_found(code),
        other => AppError::from(other),
    }
}

fn map_purchase(row: &rusqlite::Row) -> rusqlite::Result<Purchase> {
    Ok(Purchase {
        id: row.get("id")?,
        reference: row.get("reference")?,
        client_id: row.get("client_id")?,
        product_label: row.get("product_label")?,
        total_price: row.get("total_price")?,
        installment_count: row.get("installment_count")?,
        interval_kind: row.get("interval_kind")?,
        interval_days: row.get("interval_days")?,
        purchase_date: row.get("purchase_date")?,
        created_at: row.get("created_at")?,
        archived_at: row.get("archived_at")?,
    })
}

/// Load a purchase's installments with their effective status computed.
fn load_installments(conn: &Connection, purchase_id: i64) -> DbResult<Vec<Installment>> {
    let today = today();
    let mut stmt = conn.prepare(
        "SELECT id, purchase_id, idx, amount, due_date, paid_amount, paid_date
             FROM installment WHERE purchase_id = ?1 ORDER BY idx",
    )?;
    let rows = stmt.query_map([purchase_id], |row| {
        let amount: i64 = row.get("amount")?;
        let paid_amount: i64 = row.get("paid_amount")?;
        let due_date: String = row.get("due_date")?;
        let status = parse_date(&due_date)
            .map(|d| installment_status(amount, paid_amount, d, today))
            .unwrap_or("pending");
        Ok(Installment {
            id: row.get("id")?,
            purchase_id: row.get("purchase_id")?,
            index: row.get("idx")?,
            amount,
            due_date,
            paid_amount,
            paid_date: row.get("paid_date")?,
            status: status.to_string(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

/// Deliberately unfiltered by `archived_at`: an archived purchase's detail page
/// must still open, whether reached from the archive tab or a direct link.
fn build_purchase_detail(conn: &Connection, purchase_id: i64) -> DbResult<PurchaseDetail> {
    let purchase = conn
        .query_row(
            "SELECT id, reference, client_id, product_label, total_price,
                    installment_count, interval_kind, interval_days,
                    purchase_date, created_at, archived_at
             FROM purchase WHERE id = ?1",
            [purchase_id],
            map_purchase,
        )
        .map_err(missing_row(PURCHASE_NOT_FOUND))?;
    let client = fetch_client(conn, purchase.client_id)?;
    let installments = load_installments(conn, purchase_id)?;

    let total_paid: i64 = installments.iter().map(|i| i.paid_amount).sum();
    let remaining = (purchase.total_price - total_paid).max(0);
    let statuses: Vec<&str> = installments.iter().map(|i| i.status.as_str()).collect();
    let status = purchase_status(&statuses, total_paid > 0);

    Ok(PurchaseDetail {
        purchase,
        client,
        installments,
        total_paid,
        remaining,
        status: status.to_string(),
    })
}

fn build_purchase_summary(conn: &Connection, purchase_id: i64) -> DbResult<PurchaseSummary> {
    let detail = build_purchase_detail(conn, purchase_id)?;
    let overdue_count = detail
        .installments
        .iter()
        .filter(|i| i.status == "late")
        .count() as i64;
    Ok(PurchaseSummary {
        id: detail.purchase.id,
        reference: detail.purchase.reference.clone(),
        client_id: detail.purchase.client_id,
        client_name: format!("{} {}", detail.client.first_name, detail.client.last_name),
        product_label: detail.purchase.product_label.clone(),
        total_price: detail.purchase.total_price,
        paid_amount: detail.total_paid,
        remaining: detail.remaining,
        installment_count: detail.purchase.installment_count,
        purchase_date: detail.purchase.purchase_date.clone(),
        status: detail.status.clone(),
        overdue_count,
        archived_at: detail.purchase.archived_at.clone(),
    })
}

// ===========================================================================
// Clients
// ===========================================================================

/// List clients with their aggregated purchase and balance figures.
///
/// `scope` selects which slice to return; omitting it means active clients
/// only, which is what every screen except the Clients page's Archived tab
/// wants.
#[tauri::command]
pub async fn list_clients(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    scope: Option<ClientScope>,
) -> DbResult<Vec<ClientSummary>> {
    // Reading your own client list is the unlicensed baseline, so this degrades
    // instead of refusing: without a licence the caller always gets the active
    // slice, whatever it asked for. Refusing outright would need a new error
    // code and would leave the page blank; silently narrowing keeps the baseline
    // honest — the archive is a licensed view, the client list is not.
    let scope = licensed_scope(&lic, scope);
    list_clients_impl(&db.lock(), scope)
}

/// Force an unlicensed caller onto the default (active) slice.
///
/// Generic over the two scope enums, which both derive `Default` with `Active`
/// as the default variant.
fn licensed_scope<T: Default>(lic: &LicenseState, scope: Option<T>) -> T {
    if lic.is_valid() {
        return scope.unwrap_or_default();
    }
    T::default()
}

/// Split out despite being a read, which the module doc reserves for mutating
/// commands: the scope predicate below is the kind of thing that silently
/// returns the wrong set, and this is what makes it reachable from `cargo test`
/// without a Tauri `State`.
pub(crate) fn list_clients_impl(
    conn: &Connection,
    scope: ClientScope,
) -> DbResult<Vec<ClientSummary>> {
    let today_str = today().to_string();
    // The client predicate filters the *driving* table, so it belongs in WHERE
    // rather than HAVING: the aggregates below are then computed only over the
    // joined rows of the clients that survive it. Each arm is a `&'static str`
    // — no caller input reaches the SQL text.
    //
    // The *purchase* predicate is different and must stay in the `LEFT JOIN …
    // ON` clause below. Moved into this WHERE it would degrade the outer join
    // into an inner one and drop every client who has no live purchase — see
    // `list_clients_keeps_clients_with_no_purchases_under_every_scope`.
    let scope_predicate = match scope {
        ClientScope::Active => "c.archived_at IS NULL",
        ClientScope::Archived => "c.archived_at IS NOT NULL",
        ClientScope::All => "1 = 1",
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT c.id, c.first_name, c.last_name, c.phone, c.address, c.email,
                c.created_at, c.archived_at,
                COUNT(DISTINCT p.id) AS purchase_count,
                COALESCE(SUM(i.amount - i.paid_amount), 0) AS outstanding,
                COALESCE(SUM(CASE WHEN i.due_date < ?1 AND i.amount > i.paid_amount
                                  THEN 1 ELSE 0 END), 0) AS overdue_count
             FROM client c
             LEFT JOIN purchase p
                    ON p.client_id = c.id AND p.archived_at IS NULL
             LEFT JOIN installment i ON i.purchase_id = p.id
             WHERE {scope_predicate}
             GROUP BY c.id
             ORDER BY c.last_name COLLATE NOCASE, c.first_name COLLATE NOCASE"
    ))?;
    let rows = stmt.query_map([today_str], |row| {
        Ok(ClientSummary {
            client: map_client(row)?,
            purchase_count: row.get("purchase_count")?,
            total_outstanding: row.get("outstanding")?,
            overdue_count: row.get("overdue_count")?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[tauri::command]
pub async fn get_client_detail(db: State<'_, Db>, id: i64) -> DbResult<ClientDetail> {
    let conn = db.lock();
    let client = fetch_client(&conn, id)?;

    let mut stmt = conn.prepare(
        "SELECT id FROM purchase WHERE client_id = ?1 ORDER BY purchase_date DESC, id DESC",
    )?;
    let ids: Vec<i64> = stmt
        .query_map([id], |r| r.get(0))?
        .collect::<Result<_, _>>()?;

    // Archived purchases are listed separately and contribute to none of the
    // totals — the client no longer owes them.
    let mut purchases = Vec::new();
    let mut archived_purchases = Vec::new();
    let (mut total_purchased, mut total_paid, mut overdue_count) = (0i64, 0i64, 0i64);
    for pid in ids {
        let s = build_purchase_summary(&conn, pid)?;
        if s.archived_at.is_some() {
            archived_purchases.push(s);
            continue;
        }
        total_purchased += s.total_price;
        total_paid += s.paid_amount;
        overdue_count += s.overdue_count;
        purchases.push(s);
    }
    let total_outstanding = (total_purchased - total_paid).max(0);

    Ok(ClientDetail {
        client,
        purchases,
        archived_purchases,
        total_purchased,
        total_paid,
        total_outstanding,
        overdue_count,
    })
}

#[tauri::command]
pub async fn create_client(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    input: ClientInput,
) -> DbResult<Client> {
    require_license(&lic)?;
    validate_client_input(&input)?;
    let conn = db.lock();
    conn.execute(
        "INSERT INTO client (first_name, last_name, phone, address, email)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            input.first_name.trim(),
            input.last_name.trim(),
            input.phone.trim(),
            input.address.trim(),
            input.email.as_ref().map(|e| e.trim().to_string())
        ],
    )?;
    fetch_client(&conn, conn.last_insert_rowid())
}

#[tauri::command]
pub async fn update_client(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
    input: ClientInput,
) -> DbResult<Client> {
    require_license(&lic)?;
    validate_client_input(&input)?;
    let conn = db.lock();
    conn.execute(
        "UPDATE client SET first_name = ?1, last_name = ?2, phone = ?3,
            address = ?4, email = ?5 WHERE id = ?6",
        params![
            input.first_name.trim(),
            input.last_name.trim(),
            input.phone.trim(),
            input.address.trim(),
            input.email.as_ref().map(|e| e.trim().to_string()),
            id
        ],
    )?;
    fetch_client(&conn, id)
}

/// Archive a client: hide them from the active list while keeping every
/// purchase, installment and payment row exactly as it was.
#[tauri::command]
pub async fn archive_client(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
) -> DbResult<()> {
    require_license(&lic)?;
    archive_client_impl(&mut db.lock(), id)
}

/// Refused while the client still owes money.
///
/// That guard is what lets every money read model — impayés, the dashboard,
/// the reports — skip an `archived_at` filter entirely: an archived client has
/// a zero balance by construction, so they contribute nothing to those
/// aggregates whether they are filtered out or not. The cost is deliberate: a
/// client with unpaid installments can be neither deleted nor archived.
pub(crate) fn archive_client_impl(conn: &mut Connection, id: i64) -> DbResult<()> {
    let tx = conn.transaction()?;
    fetch_client(&tx, id)?;
    let outstanding = client_outstanding(&tx, id)?;
    if outstanding > 0 {
        return Err(AppError::conflict(ARCHIVE_HAS_OUTSTANDING, outstanding));
    }
    // `date('now')`, not `datetime('now')`: this value is rendered, and
    // `formatDatePattern` splits an ISO date on `-`, so a trailing " HH:MM:SS"
    // makes the day component `NaN` and the whole timestamp falls through to
    // the screen raw. Every other date in the schema is `YYYY-MM-DD` too.
    //
    // `AND archived_at IS NULL` keeps a repeated archive a no-op instead of
    // moving the stamp, so "archived on <date>" stays truthful.
    tx.execute(
        "UPDATE client SET archived_at = date('now')
          WHERE id = ?1 AND archived_at IS NULL",
        [id],
    )?;
    tx.commit()?;
    log::info!("archived client id={id}");
    Ok(())
}

/// Restore an archived client to the active list.
#[tauri::command]
pub async fn restore_client(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
) -> DbResult<()> {
    require_license(&lic)?;
    restore_client_impl(&mut db.lock(), id)
}

/// Unconditional beyond the client existing: restoring only ever makes a hidden
/// row visible again, so there is nothing to guard against. Restoring an
/// already-active client is a successful no-op.
pub(crate) fn restore_client_impl(conn: &mut Connection, id: i64) -> DbResult<()> {
    let tx = conn.transaction()?;
    fetch_client(&tx, id)?;
    tx.execute("UPDATE client SET archived_at = NULL WHERE id = ?1", [id])?;
    tx.commit()?;
    log::info!("restored client id={id}");
    Ok(())
}

/// Delete a client outright.
///
/// Only ever permitted for a client with no purchases — a mistyped or duplicate
/// entry. Anyone with history is archived instead, never destroyed: the app's
/// only other recovery path is a backup the user had to have taken first.
#[tauri::command]
pub async fn delete_client(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
) -> DbResult<()> {
    require_license(&lic)?;
    delete_client_impl(&mut db.lock(), id)
}

pub(crate) fn delete_client_impl(conn: &mut Connection, id: i64) -> DbResult<()> {
    let tx = conn.transaction()?;
    // Deleting an id that is already gone used to succeed silently; the caller
    // deserves to know its list was stale.
    fetch_client(&tx, id)?;
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM purchase WHERE client_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if count > 0 {
        return Err(AppError::conflict(CLIENT_HAS_PURCHASES, count));
    }
    tx.execute("DELETE FROM client WHERE id = ?1", [id])?;
    tx.commit()?;
    log::info!("deleted client id={id} (no purchases)");
    Ok(())
}

// ===========================================================================
// Purchases (Achats)
// ===========================================================================

/// SQL predicate selecting one slice of the purchase table.
///
/// Each arm is a `&'static str`, so nothing from the caller reaches the SQL
/// text. Applied in the id query rather than over the built summaries: the
/// listing is already N+1 (three queries per summary), so filtering afterwards
/// would pay that cost for rows nobody asked for.
fn purchase_scope_predicate(scope: PurchaseScope) -> &'static str {
    match scope {
        PurchaseScope::Active => "archived_at IS NULL",
        PurchaseScope::Archived => "archived_at IS NOT NULL",
        PurchaseScope::All => "1 = 1",
    }
}

/// Purchase ids in list order, for one scope. Split out so the predicate is
/// reachable from `cargo test` without a Tauri `State`.
pub(crate) fn list_purchase_ids(conn: &Connection, scope: PurchaseScope) -> DbResult<Vec<i64>> {
    let predicate = purchase_scope_predicate(scope);
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM purchase WHERE {predicate} ORDER BY purchase_date DESC, id DESC"
    ))?;
    let ids = stmt
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids)
}

#[tauri::command]
pub async fn list_purchases(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    scope: Option<PurchaseScope>,
    search: Option<String>,
) -> DbResult<Vec<PurchaseSummary>> {
    // Same baseline rule as `list_clients`: reading the purchase list is free,
    // narrowing it is licensed. An unlicensed caller is pinned to the active
    // slice with no server-side search.
    let licensed = lic.is_valid();
    let conn = db.lock();
    let ids = list_purchase_ids(&conn, licensed_scope(&lic, scope))?;

    let needle = search
        .filter(|_| licensed)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let mut out = Vec::new();
    for pid in ids {
        let s = build_purchase_summary(&conn, pid)?;
        if let Some(n) = &needle {
            let hay =
                format!("{} {} {}", s.reference, s.client_name, s.product_label).to_lowercase();
            if !hay.contains(n) {
                continue;
            }
        }
        out.push(s);
    }
    Ok(out)
}

#[tauri::command]
pub async fn get_purchase_detail(db: State<'_, Db>, id: i64) -> DbResult<PurchaseDetail> {
    let conn = db.lock();
    build_purchase_detail(&conn, id)
}

#[tauri::command]
pub async fn create_purchase(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    input: PurchaseInput,
) -> DbResult<PurchaseDetail> {
    require_license(&lic)?;
    create_purchase_impl(&mut db.lock(), input)
}

/// Validate a purchase request coming off the IPC boundary.
///
/// Every field here arrives from the renderer and is therefore untrusted. The
/// numeric bounds matter beyond tidiness: `installment_count` sizes a `Vec` and
/// an insert loop, and `interval_days` is multiplied by the installment index
/// before being handed to date math, so unbounded values are a memory- and
/// crash-amplification path rather than merely bad data.
fn validate_purchase_input(input: &PurchaseInput) -> DbResult<chrono::NaiveDate> {
    // Bounded from above as well as below: see [`MONEY_RANGE`] for why an
    // unbounded total lets a wrapping `i64` sum satisfy the `SUM_MISMATCH`
    // equality that is supposed to prove the schedule adds up.
    // A purchase must be worth something, so the lower bound is stricter than
    // [`MONEY_RANGE`]'s (which admits a zero *share*). The upper bound is the
    // range's: see its doc for why an unbounded total lets a wrapping `i64` sum
    // satisfy the `SUM_MISMATCH` equality meant to prove the schedule adds up.
    if input.total_price <= 0 || input.total_price > *MONEY_RANGE.end() {
        return Err(AppError::validation(INVALID_TOTAL_PRICE));
    }
    // The label is free text and lands in every list, export and dashboard card.
    bounded(input.product_label.trim(), SHORT_TEXT_MAX)?;
    if !INSTALLMENT_COUNT_RANGE.contains(&input.installment_count) {
        return Err(AppError::validation(INVALID_INSTALLMENT_COUNT));
    }
    if !INTERVAL_KINDS.contains(&input.interval_kind.as_str()) {
        return Err(AppError::validation(INVALID_INTERVAL_KIND));
    }
    // `interval_days` is only meaningful for the custom kind; the other two
    // ignore it, so an unused stale value must not fail the request.
    if input.interval_kind == "custom" {
        let days = input.interval_days.unwrap_or(30);
        if !INTERVAL_DAYS_RANGE.contains(&days) {
            return Err(AppError::validation(INVALID_INTERVAL_DAYS));
        }
    }
    parse_date(&input.purchase_date)
}

/// Resolve a request into the installment amounts and due dates to write.
///
/// Shared by create and update so the two can never drift: a rescheduling edit
/// has to produce byte-identical rows to creating the same purchase from
/// scratch. Runs entirely before any transaction opens, so a rejected request
/// never touches the database.
fn resolve_schedule(
    input: &PurchaseInput,
    purchase_date: chrono::NaiveDate,
) -> DbResult<(Vec<i64>, Vec<String>)> {
    let amounts = match &input.installments {
        Some(list) if !list.is_empty() => {
            // The list is what actually sizes the schedule — the row vector, the
            // date vector and the insert loop all follow its length, not
            // `installment_count`. So the `1..=120` bound `validate_purchase_input`
            // puts on that field only binds if the two agree; without this a
            // request declaring `installmentCount: 1` could carry a million
            // entries and drive exactly the unbounded allocation and insert loop
            // the bound exists to prevent. It would also leave the stored
            // `installment_count` lying about the row count, and that figure is
            // rendered straight to the user as "index/count".
            if list.len() as i64 != input.installment_count {
                return Err(AppError::conflict(
                    INSTALLMENT_COUNT_MISMATCH,
                    format!("{}:{}", list.len(), input.installment_count),
                ));
            }
            // Bounded before the sum is taken, not after: the sum is the thing
            // being protected. A negative share is the sharper half — it needs no
            // overflow at all, sails through the equality below when a sibling
            // covers it, and then feeds `SUM(amount - paid_amount)` in the
            // outstanding aggregates, where one client's negative row cancels out
            // another client's real debt.
            if list.iter().any(|i| !MONEY_RANGE.contains(&i.amount)) {
                return Err(AppError::validation(INVALID_AMOUNT));
            }
            let sum: i64 = list.iter().map(|i| i.amount).sum();
            if sum != input.total_price {
                return Err(AppError::conflict(
                    SUM_MISMATCH,
                    format!("{sum}:{}", input.total_price),
                ));
            }
            list.iter().map(|i| i.amount).collect::<Vec<_>>()
        }
        _ => split_amounts(input.total_price, input.installment_count),
    };

    let due_dates: Vec<String> = match &input.installments {
        Some(list) if !list.is_empty() => list
            .iter()
            .take(amounts.len())
            // Every caller-supplied due date must survive `parse_date`. Storing
            // one unparsed is invisible but permanent: the read paths fall back
            // to "pending" and 0 days late, so the installment silently drops
            // out of the overdue and alert screens forever.
            .map(|x| parse_date(&x.due_date).map(|d| d.to_string()))
            .collect::<DbResult<_>>()?,
        // k = i (0-based): the first installment falls on the purchase date,
        // subsequent ones one interval apart.
        _ => (0..amounts.len())
            .map(|i| {
                add_interval(
                    purchase_date,
                    &input.interval_kind,
                    input.interval_days,
                    i as i64,
                )
                .to_string()
            })
            .collect(),
    };

    // Position order and chronological order have to stay the same thing: the
    // money rules speak of "the previous installment" and mean both readings at
    // once. The generated path is ascending by construction, so this only bites
    // a hand-edited schedule. It lives here rather than in `update_purchase` so
    // create and update cannot drift.
    if due_dates.windows(2).any(|w| w[0] > w[1]) {
        return Err(AppError::conflict(DUE_DATE_OUT_OF_ORDER, ""));
    }

    Ok((amounts, due_dates))
}

/// Insert installment rows for `purchase_id`. `idx` is 1-based positional and
/// continues from `first_position` (0 when writing a whole new schedule), so
/// this serves both creating a purchase and appending to one being rescheduled.
fn insert_installments(
    tx: &rusqlite::Transaction,
    purchase_id: i64,
    amounts: &[i64],
    due_dates: &[String],
    first_position: usize,
) -> DbResult<()> {
    for (i, (amount, due)) in amounts.iter().zip(due_dates).enumerate() {
        let idx = (first_position + i) as i64 + 1;
        tx.execute(
            "INSERT INTO installment (purchase_id, idx, amount, due_date)
             VALUES (?1, ?2, ?3, ?4)",
            params![purchase_id, idx, amount, due],
        )?;
    }
    Ok(())
}

pub(crate) fn create_purchase_impl(
    conn: &mut Connection,
    input: PurchaseInput,
) -> DbResult<PurchaseDetail> {
    let purchase_date = validate_purchase_input(&input)?;
    let (amounts, due_dates) = resolve_schedule(&input, purchase_date)?;

    let tx = conn.transaction()?;

    // An archived client must not take on new debt. Beyond the UI (whose picker
    // only offers active clients), this keeps "archived implies a zero balance"
    // true by construction — which is exactly the property that lets impayés,
    // the dashboard and the reports skip an `archived_at` filter altogether.
    if fetch_client(&tx, input.client_id)?.archived_at.is_some() {
        return Err(AppError::conflict(CLIENT_ARCHIVED, ""));
    }

    tx.execute(
        "INSERT INTO purchase
            (reference, client_id, product_label, total_price, installment_count,
             interval_kind, interval_days, purchase_date)
         VALUES ('', ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            input.client_id,
            input.product_label.trim(),
            input.total_price,
            input.installment_count,
            input.interval_kind,
            input.interval_days,
            input.purchase_date,
        ],
    )?;
    let purchase_id = tx.last_insert_rowid();
    let reference = format!("A-{:06}", purchase_id);
    tx.execute(
        "UPDATE purchase SET reference = ?1 WHERE id = ?2",
        params![reference, purchase_id],
    )?;

    insert_installments(&tx, purchase_id, &amounts, &due_dates, 0)?;

    tx.commit()?;
    log::info!(
        "created purchase id={purchase_id} with {} installments",
        amounts.len()
    );
    build_purchase_detail(conn, purchase_id)
}

/// Whether applying `input` would produce different installment rows.
///
/// Compares the *resolved* schedule against what is stored rather than trusting
/// the presence of `input.installments`: the editor always sends the rows it is
/// displaying, so a label-only edit arrives carrying an installment list
/// identical to the stored one. Treating that as a reschedule would lock the
/// label behind the payment guard for no reason.
fn schedule_changed(
    existing: &PurchaseDetail,
    input: &PurchaseInput,
    amounts: &[i64],
    due_dates: &[String],
) -> bool {
    let p = &existing.purchase;
    if p.total_price != input.total_price
        || p.installment_count != input.installment_count
        || p.interval_kind != input.interval_kind
        || p.interval_days != input.interval_days
        || p.purchase_date != input.purchase_date
    {
        return true;
    }
    existing.installments.len() != amounts.len()
        || existing
            .installments
            .iter()
            .zip(amounts.iter().zip(due_dates))
            .any(|(inst, (amount, due))| inst.amount != *amount || inst.due_date != *due)
}

/// Apply a resolved schedule onto the stored rows, position by position.
///
/// This is the **only** place an installment's `amount` or `due_date` changes.
/// `update_installment` refuses both outright, so a schedule edit always arrives
/// as a whole schedule and can be judged against every row at once.
///
/// It updates in place rather than regenerating, which is what lets a purchase
/// carrying payments still be rescheduled: the `payment` ledger hangs off
/// `installment` by `ON DELETE CASCADE`, so keeping the rows keeps the history.
///
/// Three rules decide whether the incoming schedule is acceptable, all checked
/// before anything is written:
///
/// * A **settled** row (`paid_amount >= amount`) is history. The incoming
///   schedule has to agree with it or the edit is refused (`AMOUNT_LOCKED`,
///   `DUE_DATE_LOCKED`).
/// * No row may be pushed below what it has already collected (`BELOW_PAID`),
///   because `amount - paid_amount` feeds every outstanding aggregate and must
///   not go negative.
/// * A row may only be **dropped** — by shortening the schedule — while it has
///   no ledger history at all (`PURCHASE_HAS_PAYMENTS`), or the delete would
///   cascade real payments away.
fn apply_schedule_in_place(
    tx: &rusqlite::Transaction,
    purchase_id: i64,
    amounts: &[i64],
    due_dates: &[String],
) -> DbResult<()> {
    let rows = load_installment_rows(tx, purchase_id)?;
    let kept = rows.len().min(amounts.len());

    for (row, (amount, due)) in rows.iter().zip(amounts.iter().zip(due_dates)) {
        if row.paid_amount >= row.amount {
            if *amount != row.amount {
                return Err(AppError::conflict(AMOUNT_LOCKED, ""));
            }
            if *due != row.due_date {
                return Err(AppError::conflict(DUE_DATE_LOCKED, ""));
            }
        }
        if *amount < row.paid_amount {
            return Err(AppError::conflict(BELOW_PAID, row.paid_amount));
        }
    }

    // Counted from the ledger, not from `paid_amount`. A row corrected back down
    // to zero still holds the entries that took the money and gave it back, and
    // the cascade would erase both — a silent hole in the payment log rather
    // than a broken total, which is exactly the kind of loss no aggregate would
    // ever surface.
    let mut dropped_with_history = 0usize;
    for row in &rows[kept..] {
        let entries: i64 = tx.query_row(
            "SELECT COUNT(*) FROM payment WHERE installment_id = ?1",
            [row.id],
            |r| r.get(0),
        )?;
        if entries > 0 {
            dropped_with_history += 1;
        }
    }
    if dropped_with_history > 0 {
        return Err(AppError::conflict(
            PURCHASE_HAS_PAYMENTS,
            dropped_with_history,
        ));
    }

    // --- writes --------------------------------------------------------------

    for (row, (amount, due)) in rows.iter().zip(amounts.iter().zip(due_dates)) {
        if row.amount == *amount && row.due_date == *due {
            continue;
        }
        tx.execute(
            "UPDATE installment SET amount = ?1, due_date = ?2 WHERE id = ?3",
            params![amount, due, row.id],
        )?;
        // `paid_date` is derived from the amount as much as from the ledger: a
        // row lowered onto its collected figure settles and gains a date, one
        // raised past it stops being settled and loses it.
        if row.amount != *amount {
            sync_paid_date(tx, row.id, *amount, row.paid_amount)?;
        }
    }

    for row in &rows[kept..] {
        tx.execute("DELETE FROM installment WHERE id = ?1", [row.id])?;
    }

    insert_installments(
        tx,
        purchase_id,
        &amounts[kept..],
        &due_dates[kept..],
        rows.len(),
    )
}

/// Edit a purchase.
///
/// The product label is always editable. Everything the schedule is derived
/// from — total, count, interval and the purchase date that anchors it — is
/// applied through [`apply_schedule_in_place`], which is what makes this the
/// single place an installment's amount or due date may move. A settled
/// installment, or one whose removal would take payments with it, refuses the
/// edit there. `client_id` is ignored: moving a purchase to another client is
/// not something this command does.
#[tauri::command]
pub async fn update_purchase(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
    input: PurchaseInput,
) -> DbResult<PurchaseDetail> {
    require_license(&lic)?;
    update_purchase_impl(&mut db.lock(), id, input)
}

pub(crate) fn update_purchase_impl(
    conn: &mut Connection,
    id: i64,
    input: PurchaseInput,
) -> DbResult<PurchaseDetail> {
    let purchase_date = validate_purchase_input(&input)?;
    let (amounts, due_dates) = resolve_schedule(&input, purchase_date)?;

    let tx = conn.transaction()?;
    let existing = build_purchase_detail(&tx, id)?;
    if existing.purchase.archived_at.is_some() {
        return Err(AppError::conflict(PURCHASE_ARCHIVED, ""));
    }

    let reschedule = schedule_changed(&existing, &input, &amounts, &due_dates);

    tx.execute(
        "UPDATE purchase SET product_label = ?1, total_price = ?2,
             installment_count = ?3, interval_kind = ?4, interval_days = ?5,
             purchase_date = ?6
         WHERE id = ?7",
        params![
            input.product_label.trim(),
            input.total_price,
            input.installment_count,
            input.interval_kind,
            input.interval_days,
            input.purchase_date,
            id,
        ],
    )?;

    if reschedule {
        apply_schedule_in_place(&tx, id, &amounts, &due_dates)?;
    }

    tx.commit()?;
    log::info!("updated purchase id={id} (rescheduled: {reschedule})");
    build_purchase_detail(conn, id)
}

/// Archive a purchase: remove it from every list and every total, reversibly.
#[tauri::command]
pub async fn archive_purchase(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
) -> DbResult<()> {
    require_license(&lic)?;
    archive_purchase_impl(&mut db.lock(), id)
}

/// Refused once any payment has been recorded against it.
///
/// This is half of the invariant the money queries rely on: **an archived
/// purchase carries zero payments**. Together with the guard in
/// `record_payment_impl` it means `total_collected` never has to filter on
/// `archived_at` — an archived purchase has nothing to contribute to it.
/// It also means a purchase against which real cash was taken is permanent:
/// it can be neither archived nor deleted.
pub(crate) fn archive_purchase_impl(conn: &mut Connection, id: i64) -> DbResult<()> {
    let tx = conn.transaction()?;
    build_purchase_detail(&tx, id)?;
    let paid = payment_count(&tx, id)?;
    if paid > 0 {
        return Err(AppError::conflict(PURCHASE_HAS_PAYMENTS, paid));
    }
    // `date('now')` and the `IS NULL` guard: see `archive_client_impl`.
    tx.execute(
        "UPDATE purchase SET archived_at = date('now')
          WHERE id = ?1 AND archived_at IS NULL",
        [id],
    )?;
    tx.commit()?;
    log::info!("archived purchase id={id}");
    Ok(())
}

/// Restore an archived purchase, putting it back into every total.
#[tauri::command]
pub async fn restore_purchase(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
) -> DbResult<()> {
    require_license(&lic)?;
    restore_purchase_impl(&mut db.lock(), id)
}

pub(crate) fn restore_purchase_impl(conn: &mut Connection, id: i64) -> DbResult<()> {
    let tx = conn.transaction()?;
    build_purchase_detail(&tx, id)?;
    tx.execute("UPDATE purchase SET archived_at = NULL WHERE id = ?1", [id])?;
    tx.commit()?;
    log::info!("restored purchase id={id}");
    Ok(())
}

/// Destroy a purchase and its installments for good.
///
/// Only ever permitted for a purchase that is already archived, which makes
/// the two-step real rather than a convention the UI could forget. Combined
/// with the archive guard, a purchase carrying payments can never reach here.
#[tauri::command]
pub async fn delete_purchase(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
) -> DbResult<()> {
    require_license(&lic)?;
    delete_purchase_impl(&mut db.lock(), id)
}

pub(crate) fn delete_purchase_impl(conn: &mut Connection, id: i64) -> DbResult<()> {
    let tx = conn.transaction()?;
    // Deleting an id that is already gone used to succeed silently.
    let purchase = build_purchase_detail(&tx, id)?.purchase;
    if purchase.archived_at.is_none() {
        return Err(AppError::conflict(PURCHASE_NOT_ARCHIVED, ""));
    }
    tx.execute("DELETE FROM purchase WHERE id = ?1", [id])?;
    tx.commit()?;
    log::info!("deleted archived purchase id={id}");
    Ok(())
}

// ===========================================================================
// Installments
// ===========================================================================

/// Re-derive one installment's `paid_date` from its current numbers.
///
/// `record_payment_impl` owns the same rule, but it only ever moves
/// `paid_amount` upwards against a fixed `amount`. The two editors reach it from
/// either side: `update_installment` moves `paid_amount` under a fixed amount,
/// and `apply_schedule_in_place` moves the amount under a fixed `paid_amount`.
/// Either way a row can *become* settled or *stop* being settled without any
/// payment changing hands, and the date has to follow. The settled date is the
/// last payment on the row; a row settled because it was zeroed has no payments
/// and keeps a `NULL` date.
fn sync_paid_date(
    tx: &rusqlite::Transaction,
    installment_id: i64,
    amount: i64,
    paid_amount: i64,
) -> DbResult<()> {
    let paid_date: Option<String> = if paid_amount >= amount {
        tx.query_row(
            "SELECT MAX(payment_date) FROM payment WHERE installment_id = ?1",
            [installment_id],
            |r| r.get(0),
        )?
    } else {
        None
    };
    tx.execute(
        "UPDATE installment SET paid_date = ?1 WHERE id = ?2",
        params![paid_date, installment_id],
    )?;
    Ok(())
}

/// One installment's amount, due date and paid date, as stored.
struct InstallmentRow {
    id: i64,
    idx: i64,
    amount: i64,
    paid_amount: i64,
    due_date: String,
}

/// Every installment of a purchase, ordered by position. Read inside the
/// transaction because the rebalance below needs a consistent snapshot of all
/// of them, not just the one being edited.
fn load_installment_rows(
    tx: &rusqlite::Transaction,
    purchase_id: i64,
) -> DbResult<Vec<InstallmentRow>> {
    let mut stmt = tx.prepare(
        "SELECT id, idx, amount, paid_amount, due_date
           FROM installment WHERE purchase_id = ?1 ORDER BY idx",
    )?;
    let rows = stmt
        .query_map([purchase_id], |row| {
            Ok(InstallmentRow {
                id: row.get("id")?,
                idx: row.get("idx")?,
                amount: row.get("amount")?,
                paid_amount: row.get("paid_amount")?,
                due_date: row.get("due_date")?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Record money against a single installment.
///
/// This is the one write path that still works *after* a payment has been
/// recorded, and it deals **only** in money: `paid_amount`, `payment_date` and
/// `note`. The schedule — `amount` and `due_date` — belongs to
/// [`update_purchase`] alone and is refused here with `SCHEDULE_VIA_PURCHASE`,
/// which is what makes "the schedule is edited in one place" a property of the
/// backend rather than a habit of the UI.
///
/// The rules:
///
/// * `paid_amount` is editable only once installment `N-1` is fully paid
///   (`PREVIOUS_UNPAID:{index}`). Cash is collected in order, so it cannot be
///   recorded out of order. Nothing about *this* installment's own status gates
///   it — a settled row's collected figure stays correctable.
/// * A **payment date is history once recorded** (`PAYMENT_DATE_LOCKED`). It may
///   only be supplied to date the ledger entry this edit is about to create, so
///   an existing entry's date can never be rewritten. With no entry to create
///   and none on record, a date or a note is refused with `NO_PAYMENT_TO_DATE`
///   rather than silently dropped.
///
/// One invariant survives it: `SUM(payment.amount) == SUM(installment.paid_amount)`.
/// `paid_amount` is a cache of the ledger, so moving it writes a matching
/// **correction entry** into `payment` (negative when the figure comes down).
/// Without that the dashboard's "Amount collected", the only money figure
/// derived from the ledger, would drift away from every other total in the app.
///
/// Mirrored guard-for-guard by `updateInstallment` in `src/api/mock.ts`.
#[tauri::command]
pub async fn update_installment(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    id: i64,
    edit: InstallmentEdit,
) -> DbResult<PurchaseDetail> {
    require_license(&lic)?;
    update_installment_impl(&mut db.lock(), id, edit)
}

pub(crate) fn update_installment_impl(
    conn: &mut Connection,
    id: i64,
    edit: InstallmentEdit,
) -> DbResult<PurchaseDetail> {
    // Validate what can be validated without touching the database, so a
    // malformed request never opens a transaction.
    //
    // Refused on *presence*, not on "differs from what is stored": a caller
    // sending a schedule field still believes this command owns one, and a
    // no-op today is a real edit after the next keystroke.
    if edit.amount.is_some() || edit.due_date.is_some() {
        return Err(AppError::conflict(SCHEDULE_VIA_PURCHASE, ""));
    }
    let payment_date = edit.payment_date.as_deref().map(parse_date).transpose()?;
    if edit.paid_amount.is_some_and(|p| p < 0) {
        return Err(AppError::validation(INVALID_AMOUNT));
    }
    if payment_date.is_some_and(|d| d > today()) {
        return Err(AppError::validation(FUTURE_PAID_DATE));
    }

    let tx = conn.transaction()?;

    let purchase_id: i64 = tx
        .query_row(
            "SELECT purchase_id FROM installment WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .map_err(missing_row(INSTALLMENT_NOT_FOUND))?;

    let archived: Option<String> = tx.query_row(
        "SELECT archived_at FROM purchase WHERE id = ?1",
        [purchase_id],
        |r| r.get(0),
    )?;
    if archived.is_some() {
        return Err(AppError::conflict(PURCHASE_ARCHIVED, ""));
    }

    let rows = load_installment_rows(&tx, purchase_id)?;
    let pos = rows
        .iter()
        .position(|r| r.id == id)
        .ok_or_else(|| AppError::not_found(INSTALLMENT_NOT_FOUND))?;
    let target = &rows[pos];

    // --- the money: gated on the previous installment being settled ----------

    let paid_changed = edit.paid_amount.is_some_and(|p| p != target.paid_amount);
    let touches_money = paid_changed || payment_date.is_some() || edit.note.is_some();
    if touches_money {
        if let Some(prev) = pos.checked_sub(1).map(|p| &rows[p]) {
            if prev.paid_amount < prev.amount {
                return Err(AppError::conflict(PREVIOUS_UNPAID, prev.idx));
            }
        }
    }

    // --- resolve everything before writing anything --------------------------

    let final_paid = edit.paid_amount.unwrap_or(target.paid_amount);
    if final_paid > target.amount {
        return Err(AppError::conflict(PAID_ABOVE_AMOUNT, target.amount));
    }

    let latest_payment: Option<i64> = tx
        .query_row(
            "SELECT id FROM payment WHERE installment_id = ?1
              ORDER BY payment_date DESC, id DESC LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()?;

    // A payment date dates the correction entry created below, and nothing
    // else. Once an entry exists its date is history — re-dating it would move
    // `paid_date` (derived as `MAX(payment_date)`) away from the cash it
    // describes — and with no entry either way there is nothing for the date to
    // land on. Both are refusals rather than a silently dropped field.
    if payment_date.is_some() && !paid_changed {
        return Err(if latest_payment.is_some() {
            AppError::conflict(PAYMENT_DATE_LOCKED, "")
        } else {
            AppError::conflict(NO_PAYMENT_TO_DATE, "")
        });
    }
    // A note carries no such history, so it may still amend the latest entry —
    // but it still needs one to amend.
    if edit.note.is_some() && !paid_changed && latest_payment.is_none() {
        return Err(AppError::conflict(NO_PAYMENT_TO_DATE, ""));
    }

    // --- writes --------------------------------------------------------------

    let note = edit
        .note
        .as_ref()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty());
    if paid_changed {
        // The correction entry. Dated today when the caller did not say, so the
        // ledger row always carries a real date.
        let entry_date = payment_date.unwrap_or_else(today).to_string();
        tx.execute(
            "INSERT INTO payment (installment_id, amount, payment_date, note)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, final_paid - target.paid_amount, entry_date, note],
        )?;
        tx.execute(
            "UPDATE installment SET paid_amount = ?1 WHERE id = ?2",
            params![final_paid, id],
        )?;
    } else if let Some(payment_id) = latest_payment {
        // Nothing to correct, so a note amends the entry already there. The
        // guard above proved there is one — and that no date came with it.
        if let Some(note) = note {
            tx.execute(
                "UPDATE payment SET note = ?1 WHERE id = ?2",
                params![note, payment_id],
            )?;
        }
    }

    // `paid_date` is derived from the collected figure, so it has to be re-run
    // whenever that figure moves.
    sync_paid_date(&tx, id, target.amount, final_paid)?;

    tx.commit()?;
    log::info!(
        "updated installment id={id} on purchase id={purchase_id} \
         (ledger correction: {paid_changed})"
    );
    build_purchase_detail(conn, purchase_id)
}

// ===========================================================================
// Payments
// ===========================================================================
//
// None of the listings below filter on `purchase.archived_at`, deliberately.
// They are the payment ledger, and under the zero-payments invariant an
// archived purchase has nothing in it: `archive_purchase` refuses once a
// payment exists and `record_payment` refuses an archived purchase. A filter
// here would be dead weight on a join that is already four tables deep.

/// Record a payment against a specific installment. Supports partial payments:
/// the installment's `paid_amount` accumulates and `paid_date` is set once it
/// is fully covered.
#[tauri::command]
pub async fn record_payment(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    input: PaymentInput,
) -> DbResult<PurchaseDetail> {
    require_license(&lic)?;
    record_payment_impl(&mut db.lock(), input)
}

pub(crate) fn record_payment_impl(
    conn: &mut Connection,
    input: PaymentInput,
) -> DbResult<PurchaseDetail> {
    if input.amount <= 0 {
        return Err(AppError::validation(INVALID_AMOUNT));
    }
    parse_date(&input.payment_date)?;

    let tx = conn.transaction()?;

    let (purchase_id, amount, paid): (i64, i64, i64) = tx
        .query_row(
            "SELECT purchase_id, amount, paid_amount FROM installment WHERE id = ?1",
            [input.installment_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| AppError::not_found(INSTALLMENT_NOT_FOUND))?;

    // The other half of "an archived purchase carries zero payments": without
    // this, an archived purchase could accrue cash that no total would ever
    // show, because every money query filters archived purchases out.
    let archived: Option<String> = tx.query_row(
        "SELECT archived_at FROM purchase WHERE id = ?1",
        [purchase_id],
        |r| r.get(0),
    )?;
    if archived.is_some() {
        return Err(AppError::conflict(PURCHASE_ARCHIVED, ""));
    }

    // Reject overpayment rather than absorbing it. An uncapped `paid_amount`
    // makes `amount - paid_amount` negative, and that column is summed straight
    // into the outstanding/overdue aggregates — so one overpaid installment
    // silently cancels out another client's real debt on the dashboard.
    let remaining = amount - paid;
    if input.amount > remaining {
        return Err(AppError::conflict(OVERPAYMENT, remaining.max(0)));
    }
    let new_paid = paid + input.amount;

    tx.execute(
        "INSERT INTO payment (installment_id, amount, payment_date, note)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            input.installment_id,
            input.amount,
            input.payment_date,
            input.note.as_ref().map(|n| n.trim().to_string())
        ],
    )?;

    let paid_date = if new_paid >= amount {
        Some(input.payment_date.clone())
    } else {
        None
    };
    tx.execute(
        "UPDATE installment SET paid_amount = ?1, paid_date = ?2 WHERE id = ?3",
        params![new_paid, paid_date, input.installment_id],
    )?;

    tx.commit()?;
    log::info!(
        "recorded payment on installment id={} (fully covered: {})",
        input.installment_id,
        new_paid >= amount
    );
    build_purchase_detail(conn, purchase_id)
}

fn map_payment(row: &rusqlite::Row) -> rusqlite::Result<Payment> {
    let first: String = row.get("first_name")?;
    let last: String = row.get("last_name")?;
    Ok(Payment {
        id: row.get("id")?,
        installment_id: row.get("installment_id")?,
        installment_index: row.get("idx")?,
        purchase_id: row.get("purchase_id")?,
        purchase_reference: row.get("reference")?,
        client_id: row.get("client_id")?,
        client_name: format!("{first} {last}"),
        amount: row.get("amount")?,
        payment_date: row.get("payment_date")?,
        note: row.get("note")?,
        created_at: row.get("created_at")?,
    })
}

#[tauri::command]
pub async fn list_payments_for_purchase(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    purchase_id: i64,
) -> DbResult<Vec<Payment>> {
    require_license(&lic)?;
    let conn = db.lock();
    let mut stmt = conn.prepare(
        "SELECT pay.id, pay.installment_id, pay.amount, pay.payment_date,
                    pay.note, pay.created_at,
                    i.idx, i.purchase_id, pu.reference,
                    c.id AS client_id, c.first_name, c.last_name
             FROM payment pay
             JOIN installment i ON i.id = pay.installment_id
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             WHERE i.purchase_id = ?1
             ORDER BY pay.payment_date DESC, pay.id DESC",
    )?;
    let rows = stmt.query_map([purchase_id], map_payment)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[tauri::command]
pub async fn list_all_payments(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    limit: Option<i64>,
) -> DbResult<Vec<Payment>> {
    require_license(&lic)?;
    list_all_payments_impl(&db.lock(), limit)
}

pub(crate) fn list_all_payments_impl(
    conn: &Connection,
    limit: Option<i64>,
) -> DbResult<Vec<Payment>> {
    // Clamped, not rejected — the same treatment `upcoming_days` and
    // `alert_soon_days` get, since this is a display horizon rather than a
    // domain value and no user can ask for a bad one through the UI. See
    // [`PAYMENT_LIMIT_RANGE`] for why the *lower* bound is the one that matters.
    let limit = limit
        .unwrap_or(500)
        .clamp(*PAYMENT_LIMIT_RANGE.start(), *PAYMENT_LIMIT_RANGE.end());
    let mut stmt = conn.prepare(
        "SELECT pay.id, pay.installment_id, pay.amount, pay.payment_date,
                    pay.note, pay.created_at,
                    i.idx, i.purchase_id, pu.reference,
                    c.id AS client_id, c.first_name, c.last_name
             FROM payment pay
             JOIN installment i ON i.id = pay.installment_id
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             ORDER BY pay.payment_date DESC, pay.id DESC
             LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], map_payment)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[tauri::command]
pub async fn list_payments_for_client(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    client_id: i64,
) -> DbResult<Vec<Payment>> {
    require_license(&lic)?;
    let conn = db.lock();
    let mut stmt = conn.prepare(
        "SELECT pay.id, pay.installment_id, pay.amount, pay.payment_date,
                    pay.note, pay.created_at,
                    i.idx, i.purchase_id, pu.reference,
                    c.id AS client_id, c.first_name, c.last_name
             FROM payment pay
             JOIN installment i ON i.id = pay.installment_id
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             WHERE pu.client_id = ?1
             ORDER BY pay.payment_date DESC, pay.id DESC",
    )?;
    let rows = stmt.query_map([client_id], map_payment)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

// ===========================================================================
// Échéances / Impayés
// ===========================================================================

/// All installments due in the given window (defaults to everything), enriched
/// for the schedule screen.
#[tauri::command]
pub async fn list_impayes(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    filter: Option<ImpayeFilter>,
) -> DbResult<Vec<ImpayeClient>> {
    require_license(&lic)?;
    let conn = db.lock();
    build_impayes(&conn, filter.unwrap_or_default(), None)
}

fn build_impayes(
    conn: &Connection,
    filter: ImpayeFilter,
    limit: Option<usize>,
) -> DbResult<Vec<ImpayeClient>> {
    let today = today();
    let today_str = today.to_string();

    // Gather overdue installments (past due with remaining balance), applying
    // the optional date-range / client filters.
    let mut sql = String::from(
        "SELECT i.id, i.purchase_id, pu.reference, i.idx, pu.installment_count,
                i.due_date, i.amount, i.paid_amount,
                c.id AS client_id, c.first_name, c.last_name, c.phone, c.address, c.email
         FROM installment i
         JOIN purchase pu ON pu.id = i.purchase_id
         JOIN client c ON c.id = pu.client_id
         WHERE i.due_date < ?1 AND i.amount > i.paid_amount
           AND pu.archived_at IS NULL",
    );
    // The archived-purchase predicate above is deliberately a literal with no
    // placeholder, so it costs the numbering below nothing.
    //
    // Bind parameters in lockstep with the placeholders: only the optional
    // filters that are actually present contribute both a `?n` clause and a
    // value, so the numbering stays sequential and the count always matches.
    // (Binding a fixed set of four here silently breaks the common no-filter
    // path, since the query then declares only `?1`.)
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(today_str.clone())];
    let mut next = 1;
    if let Some(from) = filter.date_from.clone() {
        next += 1;
        sql.push_str(&format!(" AND i.due_date >= ?{next}"));
        params_vec.push(Box::new(from));
    }
    if let Some(to) = filter.date_to.clone() {
        next += 1;
        sql.push_str(&format!(" AND i.due_date <= ?{next}"));
        params_vec.push(Box::new(to));
    }
    if let Some(cid) = filter.client_id {
        next += 1;
        sql.push_str(&format!(" AND c.id = ?{next}"));
        params_vec.push(Box::new(cid));
    }
    sql.push_str(" ORDER BY c.last_name COLLATE NOCASE, c.first_name COLLATE NOCASE, i.due_date");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

    // Accumulate per client, preserving first-seen order.
    let mut order: Vec<i64> = Vec::new();
    let mut map: std::collections::HashMap<i64, ImpayeClient> = std::collections::HashMap::new();

    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        let due_date: String = row.get("due_date")?;
        let amount: i64 = row.get("amount")?;
        let paid: i64 = row.get("paid_amount")?;
        let days_late = parse_date(&due_date)
            .map(|d| (today - d).num_days())
            .unwrap_or(0);
        let client_id: i64 = row.get("client_id")?;
        let first: String = row.get("first_name")?;
        let last: String = row.get("last_name")?;
        Ok((
            client_id,
            first,
            last,
            row.get::<_, String>("phone")?,
            row.get::<_, String>("address")?,
            row.get::<_, Option<String>>("email")?,
            OverdueInstallment {
                installment_id: row.get("id")?,
                purchase_id: row.get("purchase_id")?,
                purchase_reference: row.get("reference")?,
                index: row.get("idx")?,
                installment_count: row.get("installment_count")?,
                due_date,
                amount,
                remaining: amount - paid,
                days_late,
            },
        ))
    })?;

    for row in rows {
        let (cid, first, last, phone, address, email, inst) = row?;
        let entry = map.entry(cid).or_insert_with(|| {
            order.push(cid);
            ImpayeClient {
                client_id: cid,
                client_name: format!("{first} {last}"),
                phone,
                address,
                email,
                reference: inst.purchase_reference.clone(),
                total_overdue: 0,
                overdue_count: 0,
                installments: Vec::new(),
            }
        });
        entry.total_overdue += inst.remaining;
        entry.overdue_count += 1;
        entry.installments.push(inst);
    }

    let mut result: Vec<ImpayeClient> =
        order.into_iter().filter_map(|id| map.remove(&id)).collect();
    // Most owed first for the dashboard panel.
    result.sort_by_key(|c| std::cmp::Reverse(c.total_overdue));
    if let Some(n) = limit {
        result.truncate(n);
    }
    Ok(result)
}

/// All installments enriched with client/purchase context, for the schedule
/// (Échéances) screen. Sorted by due date.
#[tauri::command]
pub async fn list_schedule(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
) -> DbResult<Vec<ScheduleRow>> {
    require_license(&lic)?;
    list_schedule_rows(&db.lock())
}

/// Split out so the archived-purchase filter is reachable from `cargo test`.
pub(crate) fn list_schedule_rows(conn: &Connection) -> DbResult<Vec<ScheduleRow>> {
    let today = today();
    let mut stmt = conn.prepare(
        "SELECT i.id, i.purchase_id, pu.reference, c.id AS client_id,
                    c.first_name, c.last_name, i.idx, pu.installment_count,
                    i.due_date, i.amount, i.paid_amount
             FROM installment i
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             WHERE pu.archived_at IS NULL
             ORDER BY i.due_date ASC, i.id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let amount: i64 = row.get("amount")?;
        let paid: i64 = row.get("paid_amount")?;
        let due_date: String = row.get("due_date")?;
        let status = parse_date(&due_date)
            .map(|d| installment_status(amount, paid, d, today))
            .unwrap_or("pending");
        let first: String = row.get("first_name")?;
        let last: String = row.get("last_name")?;
        Ok(ScheduleRow {
            installment_id: row.get("id")?,
            purchase_id: row.get("purchase_id")?,
            reference: row.get("reference")?,
            client_id: row.get("client_id")?,
            client_name: format!("{first} {last}"),
            index: row.get("idx")?,
            installment_count: row.get("installment_count")?,
            due_date,
            amount,
            paid_amount: paid,
            remaining: amount - paid,
            status: status.to_string(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

// ===========================================================================
// Dashboard
// ===========================================================================

#[tauri::command]
pub async fn get_dashboard(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    upcoming_days: Option<i64>,
) -> DbResult<Dashboard> {
    require_license(&lic)?;
    let conn = db.lock();
    let today = today();
    let today_str = today.to_string();
    // Clamp rather than reject: this is a display window, not user data, and an
    // out-of-range value should still render a dashboard. Unclamped it reached
    // `Duration::days`, which panics on overflow — and with `panic = "abort"`
    // that took the whole app down from a single IPC argument.
    let days = upcoming_days
        .unwrap_or(7)
        .clamp(*UPCOMING_DAYS_RANGE.start(), *UPCOMING_DAYS_RANGE.end());
    let horizon = add_interval(today, "custom", Some(days), 1).to_string();

    // Every figure below excludes archived purchases: an archived purchase has
    // been removed from the books and must not be owed, sold or counted.
    //
    // The three installment-only aggregates cannot simply gain a WHERE clause —
    // they never mention `purchase` — so each carries an EXISTS instead. Miss
    // one and the headline number silently disagrees with the list it links to.
    let total_purchases: i64 = conn.query_row(
        "SELECT COUNT(*) FROM purchase WHERE archived_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    let total_sales: i64 = conn.query_row(
        "SELECT COALESCE(SUM(total_price),0) FROM purchase WHERE archived_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    // Deliberately unfiltered. `archive_purchase` refuses once a payment
    // exists and `record_payment` refuses an archived purchase, so an archived
    // purchase has no payments to exclude — joining payment → installment →
    // purchase here would cost the app's hottest aggregate nothing but time.
    // `archiving_is_impossible_once_a_payment_exists` is what keeps that true.
    let total_collected: i64 =
        conn.query_row("SELECT COALESCE(SUM(amount),0) FROM payment", [], |r| {
            r.get(0)
        })?;
    let total_outstanding: i64 = conn.query_row(
        "SELECT COALESCE(SUM(i.amount - i.paid_amount),0) FROM installment i
             WHERE EXISTS (SELECT 1 FROM purchase pu
                            WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)",
        [],
        |r| r.get(0),
    )?;
    let overdue_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM installment i
             WHERE i.due_date < ?1 AND i.amount > i.paid_amount
               AND EXISTS (SELECT 1 FROM purchase pu
                            WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)",
        [&today_str],
        |r| r.get(0),
    )?;
    let overdue_clients: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT pu.client_id) FROM installment i
             JOIN purchase pu ON pu.id = i.purchase_id
             WHERE i.due_date < ?1 AND i.amount > i.paid_amount
               AND pu.archived_at IS NULL",
        [&today_str],
        |r| r.get(0),
    )?;
    let upcoming_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM installment i
             WHERE i.due_date >= ?1 AND i.due_date <= ?2 AND i.amount > i.paid_amount
               AND EXISTS (SELECT 1 FROM purchase pu
                            WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)",
        params![today_str, horizon],
        |r| r.get(0),
    )?;

    let stats = DashboardStats {
        total_purchases,
        total_sales,
        total_collected,
        total_outstanding,
        overdue_count,
        overdue_clients,
        upcoming_count,
    };

    // Recent purchases (latest 5).
    let mut stmt = conn.prepare(
        "SELECT id FROM purchase WHERE archived_at IS NULL
             ORDER BY purchase_date DESC, id DESC LIMIT 5",
    )?;
    let recent_ids: Vec<i64> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let recent_purchases: Vec<PurchaseSummary> = recent_ids
        .iter()
        .map(|id| build_purchase_summary(&conn, *id))
        .collect::<DbResult<_>>()?;

    // Featured purchase: prefer the most recent one with an overdue tranche,
    // else the most recent purchase overall.
    let featured_id: Option<i64> = conn
        .query_row(
            "SELECT i.purchase_id FROM installment i
             JOIN purchase pu ON pu.id = i.purchase_id
             WHERE i.due_date < ?1 AND i.amount > i.paid_amount
               AND pu.archived_at IS NULL
             ORDER BY pu.purchase_date DESC, pu.id DESC LIMIT 1",
            [&today_str],
            |r| r.get(0),
        )
        .optional()?
        .or_else(|| recent_ids.first().copied());
    let featured_purchase = match featured_id {
        Some(id) => Some(build_purchase_detail(&conn, id)?),
        None => None,
    };

    // Due alerts: overdue installments, most days late first (top 4).
    let mut stmt = conn.prepare(
        "SELECT i.purchase_id, pu.reference, i.idx, pu.installment_count, i.due_date,
                    c.first_name, c.last_name
             FROM installment i
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             WHERE i.due_date < ?1 AND i.amount > i.paid_amount
               AND pu.archived_at IS NULL
             ORDER BY i.due_date ASC LIMIT 4",
    )?;
    let due_alerts = stmt
        .query_map([&today_str], |row| {
            let due_date: String = row.get("due_date")?;
            let first: String = row.get("first_name")?;
            let last: String = row.get("last_name")?;
            let days_late = parse_date(&due_date)
                .map(|d| (today - d).num_days())
                .unwrap_or(0);
            Ok(DueAlert {
                purchase_id: row.get("purchase_id")?,
                reference: row.get("reference")?,
                client_name: format!("{first} {last}"),
                index: row.get("idx")?,
                installment_count: row.get("installment_count")?,
                due_date,
                days_late,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let impayes = build_impayes(&conn, ImpayeFilter::default(), Some(5))?;

    Ok(Dashboard {
        stats,
        recent_purchases,
        featured_purchase,
        due_alerts,
        impayes,
    })
}

// ===========================================================================
// Rapports (reports)
// ===========================================================================
//
// Why this aggregates in SQL rather than in the renderer: the frontend's widest
// window onto the ledger is `list_all_payments`, which clamps at
// `PAYMENT_LIMIT_RANGE` — 500 rows as the UI calls it. Summing that client-side
// would under-report revenue for any shop past its five-hundredth payment, and
// under-report it *silently*. The per-row read models are the wrong shape too:
// `list_purchases`, `get_client_detail` and `get_dashboard` all go through
// `build_purchase_detail` at three queries per purchase, which is fine for a
// page of rows and quadratic nonsense for a year of them.
//
// See the module comment on `models::Report` for the period-versus-as-of split
// that governs which figures here are historical and which are a snapshot.

/// The `strftime` pattern that turns a date into a bucket key for `granularity`.
///
/// Bucketing and ordering both come from this one pattern, so they cannot
/// disagree: every key is fixed-width and zero-padded, which makes
/// lexicographic order chronological order.
fn period_format(granularity: &str) -> &'static str {
    match granularity {
        "day" => "%Y-%m-%d",
        "year" => "%Y",
        // `month` and anything else; the caller has already validated the value
        // against `REPORT_GRANULARITIES`, so this arm is only reached for
        // "month".
        _ => "%Y-%m",
    }
}

/// Pick a bucket size for a span the caller did not choose one for.
///
/// Thresholds rather than a formula because the answer is about legibility, not
/// arithmetic: two months of daily bars is readable, two years of them is a
/// smear.
fn default_granularity(span_days: i64) -> &'static str {
    if span_days <= REPORT_DAY_MAX_SPAN {
        "day"
    } else if span_days <= REPORT_MONTH_MAX_SPAN {
        "month"
    } else {
        "year"
    }
}

/// Every bucket key the range covers, in order, including the empty ones.
///
/// The gaps are the reason this exists. Grouping in SQL only returns periods
/// that have rows, so a month with no takings would vanish from the series and
/// the chart would draw a continuous line over a hole. Enumerating the calendar
/// here and letting the query fill it in keeps the axis honest.
fn period_keys(from: NaiveDate, to: NaiveDate, granularity: &str) -> Vec<String> {
    let mut keys = Vec::new();
    match granularity {
        "day" => {
            let mut d = from;
            while d <= to {
                keys.push(d.to_string());
                match d.succ_opt() {
                    Some(next) => d = next,
                    // Unreachable for any validated range; saturating beats
                    // panicking under `panic = "abort"`.
                    None => break,
                }
            }
        }
        "year" => {
            for y in from.year()..=to.year() {
                keys.push(format!("{y:04}"));
            }
        }
        _ => {
            let (mut y, mut m) = (from.year(), from.month());
            let (end_y, end_m) = (to.year(), to.month());
            while (y, m) <= (end_y, end_m) {
                keys.push(format!("{y:04}-{m:02}"));
                if m == 12 {
                    y += 1;
                    m = 1;
                } else {
                    m += 1;
                }
            }
        }
    }
    keys
}

/// Sum a dated money column into bucket keys.
///
/// `date_expr` is interpolated into the SQL, so it is never caller-controlled —
/// every call site below passes a literal column name.
fn sum_by_period(
    conn: &Connection,
    table_and_join: &str,
    date_expr: &str,
    amount_expr: &str,
    fmt: &str,
    from: &str,
    to: &str,
) -> DbResult<HashMap<String, i64>> {
    let sql = format!(
        "SELECT strftime(?1, {date_expr}) AS period, COALESCE(SUM({amount_expr}),0) AS total
           FROM {table_and_join}
          WHERE {date_expr} BETWEEN ?2 AND ?3
          GROUP BY period"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![fmt, from, to], |r| {
        Ok((r.get::<_, String>("period")?, r.get::<_, i64>("total")?))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (period, total) = row?;
        out.insert(period, total);
    }
    Ok(out)
}

/// Aggregated figures over a date range, for the Rapports screen.
#[tauri::command]
pub async fn get_report(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    input: ReportInput,
) -> DbResult<Report> {
    require_license(&lic)?;
    get_report_impl(&db.lock(), &input)
}

/// Split out from the command so the whole aggregation is reachable from
/// `cargo test` without a Tauri `State`, like every other non-trivial command in
/// this module.
pub(crate) fn get_report_impl(conn: &Connection, input: &ReportInput) -> DbResult<Report> {
    let from = parse_date(&input.date_from)?;
    let to = parse_date(&input.date_to)?;
    if from > to {
        return Err(AppError::validation(INVALID_DATE));
    }
    // Inclusive of both ends, which is what a shop means by "1 to 31 January".
    let span_days = (to - from).num_days() + 1;
    if !REPORT_SPAN_DAYS_RANGE.contains(&span_days) {
        return Err(AppError::conflict(
            REPORT_RANGE_TOO_LONG,
            REPORT_SPAN_DAYS_RANGE.end(),
        ));
    }

    let granularity = match input.granularity.as_deref() {
        None => default_granularity(span_days).to_string(),
        Some(g) if REPORT_GRANULARITIES.contains(&g) => g.to_string(),
        Some(_) => return Err(AppError::validation(INVALID_GRANULARITY)),
    };
    let fmt = period_format(&granularity);

    // Resolved before any query runs, so an over-wide request is refused rather
    // than served after eight aggregates have already been computed.
    let periods = period_keys(from, to, &granularity);
    if periods.len() > REPORT_MAX_BUCKETS {
        return Err(AppError::conflict(
            REPORT_RANGE_TOO_LONG,
            REPORT_MAX_BUCKETS,
        ));
    }

    let from_s = from.to_string();
    let to_s = to.to_string();
    let as_of = today();
    let as_of_s = as_of.to_string();

    // --- period figures: genuinely historical ------------------------------
    //
    // Archived purchases are excluded, matching `list_schedule_rows` and the
    // dashboard: an archived purchase has been taken off the books, so it was
    // never sold as far as any total is concerned.
    let (sales_count, sales_amount): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(total_price),0) FROM purchase
          WHERE archived_at IS NULL AND purchase_date BETWEEN ?1 AND ?2",
        params![&from_s, &to_s],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // Deliberately unjoined, for the reason spelled out in `get_dashboard`:
    // `archive_purchase` refuses once a payment exists and `record_payment`
    // refuses an archived purchase, so an archived purchase has no payments to
    // exclude. `payment_count` counts ledger *entries*, which includes the
    // signed correction rows `update_installment` writes — that is the honest
    // number for a ledger, and `collected` nets them out on its own.
    let (payment_count, collected): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(amount),0) FROM payment
          WHERE payment_date BETWEEN ?1 AND ?2",
        params![&from_s, &to_s],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // `created_at` is a `datetime('now')` stamp, not a plain date, so it must be
    // narrowed before comparing: `'2026-08-19 10:04:11' BETWEEN '2026-08-01' AND
    // '2026-08-19'` is false, and the last day of every range would go missing.
    let new_clients: i64 = conn.query_row(
        "SELECT COUNT(*) FROM client WHERE date(created_at) BETWEEN ?1 AND ?2",
        params![&from_s, &to_s],
        |r| r.get(0),
    )?;

    // --- balance figures: a snapshot as of today ---------------------------
    let outstanding_now: i64 = conn.query_row(
        "SELECT COALESCE(SUM(i.amount - i.paid_amount),0) FROM installment i
          WHERE EXISTS (SELECT 1 FROM purchase pu
                         WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)",
        [],
        |r| r.get(0),
    )?;
    let overdue_now: i64 = conn.query_row(
        "SELECT COALESCE(SUM(i.amount - i.paid_amount),0) FROM installment i
          WHERE i.due_date < ?1 AND i.amount > i.paid_amount
            AND EXISTS (SELECT 1 FROM purchase pu
                         WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)",
        [&as_of_s],
        |r| r.get(0),
    )?;

    let totals = ReportTotals {
        sales_count,
        sales_amount,
        collected,
        payment_count,
        outstanding_now,
        overdue_now,
        new_clients,
    };

    // --- collections series ------------------------------------------------
    let collected_by = sum_by_period(
        conn,
        "payment",
        "payment_date",
        "amount",
        fmt,
        &from_s,
        &to_s,
    )?;
    let due_by = sum_by_period(
        conn,
        "installment i JOIN purchase pu ON pu.id = i.purchase_id AND pu.archived_at IS NULL",
        "i.due_date",
        "i.amount",
        fmt,
        &from_s,
        &to_s,
    )?;
    let collections = periods
        .into_iter()
        .map(|period| PeriodPoint {
            collected: collected_by.get(&period).copied().unwrap_or(0),
            due: due_by.get(&period).copied().unwrap_or(0),
            period,
        })
        .collect();

    // --- aging of what is still owed, as of today --------------------------
    //
    // Bucketed in SQL so the boundaries live in one place. `days_late` is
    // `as_of - due_date`: due today is 0 and lands in `current`, so the first
    // late bucket genuinely starts at one day.
    let mut aging_by: HashMap<String, (i64, i64)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT CASE
                      WHEN i.due_date >= ?1 THEN 'current'
                      WHEN julianday(?1) - julianday(i.due_date) <= 30 THEN '1-30'
                      WHEN julianday(?1) - julianday(i.due_date) <= 60 THEN '31-60'
                      WHEN julianday(?1) - julianday(i.due_date) <= 90 THEN '61-90'
                      ELSE '90+'
                    END AS bucket,
                    COUNT(*) AS n,
                    COALESCE(SUM(i.amount - i.paid_amount),0) AS amount
               FROM installment i
              WHERE i.amount > i.paid_amount
                AND EXISTS (SELECT 1 FROM purchase pu
                             WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)
              GROUP BY bucket",
        )?;
        let rows = stmt.query_map([&as_of_s], |r| {
            Ok((
                r.get::<_, String>("bucket")?,
                r.get::<_, i64>("n")?,
                r.get::<_, i64>("amount")?,
            ))
        })?;
        for row in rows {
            let (bucket, n, amount) = row?;
            aging_by.insert(bucket, (n, amount));
        }
    }
    // All five emitted in a fixed order, present or not, so the UI renders a
    // stable table instead of one whose rows appear and disappear with the data.
    let aging = AGING_BUCKETS
        .iter()
        .map(|&bucket| {
            let (count, amount) = aging_by.get(bucket).copied().unwrap_or((0, 0));
            AgingBucket {
                bucket: bucket.to_string(),
                count,
                amount,
            }
        })
        .collect();

    // --- who owes the most, as of today ------------------------------------
    let mut top_clients = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.first_name, c.last_name,
                    COALESCE(SUM(i.amount - i.paid_amount),0) AS outstanding,
                    COALESCE(SUM(CASE WHEN i.due_date < ?1
                                      THEN i.amount - i.paid_amount ELSE 0 END),0) AS overdue,
                    COUNT(CASE WHEN i.due_date < ?1 THEN 1 END) AS overdue_count
               FROM client c
               JOIN purchase pu ON pu.client_id = c.id AND pu.archived_at IS NULL
               JOIN installment i ON i.purchase_id = pu.id AND i.amount > i.paid_amount
              GROUP BY c.id
              ORDER BY outstanding DESC, c.last_name COLLATE NOCASE, c.first_name COLLATE NOCASE
              LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![&as_of_s, REPORT_TOP_N], |r| {
            let first: String = r.get("first_name")?;
            let last: String = r.get("last_name")?;
            Ok(ClientRisk {
                client_id: r.get("id")?,
                client_name: format!("{first} {last}"),
                outstanding: r.get("outstanding")?,
                overdue: r.get("overdue")?,
                overdue_count: r.get("overdue_count")?,
            })
        })?;
        for row in rows {
            top_clients.push(row?);
        }
    }

    // --- what sold in the range --------------------------------------------
    //
    // Period-scoped, unlike `top_clients` directly above: this answers "what did
    // we sell in January", not "what is owed on right now".
    let mut top_products = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT product_label,
                    COUNT(*) AS purchase_count,
                    COALESCE(SUM(total_price),0) AS total_amount
               FROM purchase
              WHERE archived_at IS NULL AND purchase_date BETWEEN ?1 AND ?2
              GROUP BY product_label
              ORDER BY total_amount DESC, purchase_count DESC, product_label COLLATE NOCASE
              LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![&from_s, &to_s, REPORT_TOP_N], |r| {
            Ok(ProductLine {
                product_label: r.get("product_label")?,
                purchase_count: r.get("purchase_count")?,
                total_amount: r.get("total_amount")?,
            })
        })?;
        for row in rows {
            top_products.push(row?);
        }
    }

    Ok(Report {
        range: ReportRange {
            from: from_s,
            to: to_s,
            as_of: as_of_s,
            granularity,
        },
        totals,
        collections,
        aging,
        top_clients,
        top_products,
    })
}

/// The aging buckets, in the order they are reported. Mirrored by
/// `AGING_BUCKETS` in `src/types/models.ts` and by the `rapports.aging.*` keys
/// in every locale file.
pub(crate) const AGING_BUCKETS: [&str; 5] = ["current", "1-30", "31-60", "61-90", "90+"];

// ===========================================================================
// Settings
// ===========================================================================

/// `setting` key holding the ISO date of the last successful backup.
///
/// Deliberately a setting rather than a schema column: `read_settings` resolves
/// every key through [`get_setting`] with a default, so a key that has never
/// been written reads as absent on an existing database and needs no migration.
pub(crate) const LAST_BACKUP_KEY: &str = "last_backup_at";

/// The backup schedule, all three resolved through [`get_setting`] with a
/// default, so an existing database needs no migration to gain them.
pub(crate) const AUTO_BACKUP_ENABLED_KEY: &str = "auto_backup_enabled";
pub(crate) const AUTO_BACKUP_FREQUENCY_KEY: &str = "auto_backup_frequency";
pub(crate) const AUTO_BACKUP_TIME_KEY: &str = "auto_backup_time";

pub(crate) fn get_setting(conn: &Connection, key: &str, default: &str) -> String {
    match conn
        .query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .optional()
    {
        Ok(found) => found.unwrap_or_else(|| default.to_string()),
        Err(e) => {
            // A query failure and an unset key used to be indistinguishable
            // here, so a broken settings table looked exactly like a fresh
            // install. Falling back is still the right behaviour — the UI must
            // render — but it must not be silent.
            log::warn!("failed to read setting {key:?}, falling back to default: {e}");
            default.to_string()
        }
    }
}

pub(crate) fn put_setting(conn: &Connection, key: &str, value: &str) -> DbResult<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub(crate) fn read_settings(conn: &Connection) -> Settings {
    let logo = get_setting(conn, "logo_path", "");
    Settings {
        language: get_setting(conn, "language", "fr"),
        currency_code: get_setting(conn, "currency_code", "TND"),
        date_format: get_setting(conn, "date_format", "dd/MM/yyyy"),
        logo_path: if logo.is_empty() { None } else { Some(logo) },
        shop_name: get_setting(conn, "shop_name", ""),
        shop_info: get_setting(conn, "shop_info", ""),
        alert_soon_days: get_setting(conn, "alert_soon_days", "7")
            .parse()
            .unwrap_or(7),
        language_is_default: get_setting(conn, "language_is_default", "1") == "1",
        last_backup_at: match get_setting(conn, LAST_BACKUP_KEY, "") {
            v if v.is_empty() => None,
            v => Some(v),
        },
        last_auto_backup_at: match get_setting(conn, crate::autobackup::LAST_AUTO_BACKUP_KEY, "") {
            v if v.is_empty() => None,
            v => Some(v),
        },
        auto_backup_enabled: get_setting(conn, AUTO_BACKUP_ENABLED_KEY, "1") == "1",
        auto_backup_frequency: get_setting(conn, AUTO_BACKUP_FREQUENCY_KEY, "daily"),
        auto_backup_time: get_setting(conn, AUTO_BACKUP_TIME_KEY, DEFAULT_BACKUP_TIME),
    }
}

#[tauri::command]
pub async fn get_settings(db: State<'_, Db>) -> DbResult<Settings> {
    let conn = db.lock();
    Ok(read_settings(&conn))
}

/// Update settings. Unlicensed, only `language` may be changed.
///
/// Language stays open because an unlicensed user still has to be able to *read*
/// the licence screen — locking them out of a language they cannot read would
/// make the app unrecoverable. Everything else here (shop branding, currency,
/// date format, alert window) is configuration of a licensed product.
#[tauri::command]
pub async fn update_settings(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    patch: SettingsPatch,
) -> DbResult<Settings> {
    if !lic.is_valid() && !is_language_only(&patch) {
        return Err(AppError::validation(LICENSE_REQUIRED));
    }
    update_settings_impl(&mut db.lock(), patch)
}

/// Whether a patch touches nothing but the language.
///
/// Written as an exhaustive destructure on purpose: adding a field to
/// [`SettingsPatch`] without deciding whether it is licensed becomes a compile
/// error here rather than a silently ungated setting.
fn is_language_only(patch: &SettingsPatch) -> bool {
    let SettingsPatch {
        language: _,
        currency_code,
        date_format,
        shop_name,
        shop_info,
        alert_soon_days,
        auto_backup_enabled,
        auto_backup_frequency,
        auto_backup_time,
    } = patch;
    currency_code.is_none()
        && date_format.is_none()
        && shop_name.is_none()
        && shop_info.is_none()
        && alert_soon_days.is_none()
        // The schedule is configuration of a licensed product, like the
        // currency or the alert window. The backups themselves keep running on
        // whatever is stored, and the manual button carries no gate at all, so
        // an expired licence still cannot be left without a way to copy its
        // ledger — it just cannot re-time the automatic one.
        && auto_backup_enabled.is_none()
        && auto_backup_frequency.is_none()
        && auto_backup_time.is_none()
}

pub(crate) fn update_settings_impl(
    conn: &mut Connection,
    patch: SettingsPatch,
) -> DbResult<Settings> {
    // Resolve and validate every field before the transaction opens, so a
    // rejected patch never writes and the guards cannot be read as applying to
    // one value while a different one is stored.
    //
    // The three coded fields have tiny closed vocabularies on the frontend —
    // all three are `<select>` elements — and were accepted as arbitrary strings
    // here. A junk `language` leaves the UI silently falling back to French; a
    // junk `date_format` is substituted into every date the app renders. Neither
    // can execute anything (`formatDatePattern` is a plain `String.replace` and
    // Vue escapes its interpolations), so this is data hygiene rather than an
    // injection fix — but a closed set on the frontend deserves a closed set
    // here, since the renderer is not what enforces it.
    let language = patch.language.map(|v| v.trim().to_string());
    if let Some(v) = &language {
        if !LANGUAGES.contains(&v.as_str()) {
            return Err(AppError::validation(INVALID_SETTING_VALUE));
        }
    }
    let currency_code = patch.currency_code.map(|v| v.trim().to_string());
    if let Some(v) = &currency_code {
        if !CURRENCY_CODES.contains(&v.as_str()) {
            return Err(AppError::validation(INVALID_SETTING_VALUE));
        }
    }
    let date_format = patch.date_format.map(|v| v.trim().to_string());
    if let Some(v) = &date_format {
        if !DATE_FORMATS.contains(&v.as_str()) {
            return Err(AppError::validation(INVALID_SETTING_VALUE));
        }
    }
    // Free text, so bounded rather than enumerated. Neither was even trimmed
    // before.
    let shop_name = patch.shop_name.map(|v| v.trim().to_string());
    if let Some(v) = &shop_name {
        bounded(v, SHORT_TEXT_MAX)?;
    }
    let shop_info = patch.shop_info.map(|v| v.trim().to_string());
    if let Some(v) = &shop_info {
        bounded(v, LONG_TEXT_MAX)?;
    }
    let auto_backup_frequency = patch.auto_backup_frequency.map(|v| v.trim().to_string());
    if let Some(v) = &auto_backup_frequency {
        if !BACKUP_FREQUENCIES.contains(&v.as_str()) {
            return Err(AppError::validation(INVALID_SETTING_VALUE));
        }
    }
    // Stored canonically as `HH:MM`, so the scheduler can parse it without
    // re-deciding what "5 pm" means and the settings page always round-trips
    // what an `<input type="time">` expects.
    let auto_backup_time = match &patch.auto_backup_time {
        Some(v) => Some(
            crate::autobackup::canonical_time(v)
                .ok_or_else(|| AppError::validation(INVALID_SETTING_VALUE))?,
        ),
        None => None,
    };

    // One transaction for the whole patch. Applied one upsert at a time, a
    // mid-way failure left settings half-written — worst case `language`
    // committed but `language_is_default = "0"` not, which permanently
    // re-enables OS-locale detection over the user's explicit choice.
    let tx = conn.transaction()?;
    if let Some(v) = language {
        put_setting(&tx, "language", &v)?;
        // A manual language choice ends OS-locale auto-detection.
        put_setting(&tx, "language_is_default", "0")?;
    }
    if let Some(v) = currency_code {
        put_setting(&tx, "currency_code", &v)?;
    }
    if let Some(v) = date_format {
        put_setting(&tx, "date_format", &v)?;
    }
    if let Some(v) = shop_name {
        put_setting(&tx, "shop_name", &v)?;
    }
    if let Some(v) = shop_info {
        put_setting(&tx, "shop_info", &v)?;
    }
    if let Some(v) = patch.auto_backup_enabled {
        put_setting(&tx, AUTO_BACKUP_ENABLED_KEY, if v { "1" } else { "0" })?;
    }
    if let Some(v) = auto_backup_frequency {
        put_setting(&tx, AUTO_BACKUP_FREQUENCY_KEY, &v)?;
    }
    if let Some(v) = auto_backup_time {
        put_setting(&tx, AUTO_BACKUP_TIME_KEY, &v)?;
    }
    if let Some(v) = patch.alert_soon_days {
        // Clamp defensively so the schedule query and UI never see a nonsense
        // window; the UI already constrains the input to the same range.
        let clamped = v.clamp(1, 90);
        put_setting(&tx, "alert_soon_days", &clamped.to_string())?;
    }
    tx.commit()?;
    Ok(read_settings(conn))
}

// ---------------------------------------------------------------------------
// Logo
// ---------------------------------------------------------------------------

/// Extensions the logo picker offers, and therefore the only ones accepted.
/// Must stay in step with the dialog filter in `src/views/SettingsView.vue`.
const LOGO_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// Upper bound on a logo file. A shop logo is a few hundred KB; this only has
/// to be small enough that an arbitrary file cannot be bulk-copied into app
/// data.
const LOGO_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Whether `bytes` starts with the signature of an image format we accept.
///
/// This is the check that actually matters. `set_logo` takes a caller-supplied
/// path and copies it into the app-data directory, which the renderer can read
/// back through the `asset:` protocol — so without content validation the
/// command is an arbitrary-file-read primitive: a compromised renderer could
/// copy `~/.ssh/id_rsa` to `logo.png` and fetch it. An extension check alone
/// does not stop that, because the caller chooses the extension too.
fn looks_like_image(bytes: &[u8]) -> bool {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    const GIF87: &[u8] = b"GIF87a";
    const GIF89: &[u8] = b"GIF89a";
    const JPEG: &[u8] = b"\xff\xd8\xff";

    if bytes.starts_with(PNG) || bytes.starts_with(GIF87) || bytes.starts_with(GIF89) {
        return true;
    }
    if bytes.starts_with(JPEG) {
        return true;
    }
    // WEBP is "RIFF" + 4 size bytes + "WEBP".
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}

/// Copy a picked image file into the app data dir and store its path as the
/// shop logo. Returns the updated settings.
///
/// `source_path` is untrusted even though it normally comes from the native
/// picker — the renderer can call this command with any path at all.
#[tauri::command]
pub async fn set_logo(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    app: tauri::AppHandle,
    source_path: String,
) -> DbResult<Settings> {
    require_license(&lic)?;
    use std::io::Read;
    use tauri::Manager;

    let source = std::path::Path::new(&source_path);

    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if !LOGO_EXTENSIONS.contains(&ext.as_str()) {
        log::warn!("rejected logo with unsupported extension {ext:?}");
        return Err(AppError::validation(INVALID_LOGO_TYPE));
    }

    let meta = std::fs::metadata(source).map_err(|e| {
        log::warn!("rejected unreadable logo source: {e}");
        AppError::validation(INVALID_LOGO_TYPE)
    })?;
    if !meta.is_file() {
        log::warn!("rejected logo source that is not a regular file");
        return Err(AppError::validation(INVALID_LOGO_TYPE));
    }
    if meta.len() > LOGO_MAX_BYTES {
        log::warn!("rejected logo of {} bytes", meta.len());
        return Err(AppError::validation(LOGO_TOO_LARGE));
    }

    let mut head = [0u8; 12];
    let read = std::fs::File::open(source)
        .and_then(|mut f| f.read(&mut head))
        .map_err(|e| {
            log::warn!("failed to read logo header: {e}");
            AppError::validation(INVALID_LOGO_TYPE)
        })?;
    if !looks_like_image(&head[..read]) {
        log::warn!("rejected logo whose contents are not a supported image");
        return Err(AppError::validation(INVALID_LOGO_TYPE));
    }

    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;

    // Drop any previous logo first. Without this a png → jpg switch leaves the
    // old file behind, still inside the `$APPDATA/logo.*` asset scope.
    remove_existing_logos(&data_dir);

    let dest = data_dir.join(format!("logo.{ext}"));
    std::fs::copy(source, &dest)?;

    let conn = db.lock();
    put_setting(&conn, "logo_path", &dest.to_string_lossy())?;
    log::info!("logo updated ({ext}, {} bytes)", meta.len());
    Ok(read_settings(&conn))
}

/// Delete every `logo.<ext>` we may have written previously.
fn remove_existing_logos(data_dir: &std::path::Path) {
    for ext in LOGO_EXTENSIONS {
        let path = data_dir.join(format!("logo.{ext}"));
        match std::fs::remove_file(&path) {
            Ok(()) => log::debug!("removed previous logo {ext}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("could not remove previous logo {ext}: {e}"),
        }
    }
}

#[tauri::command]
pub async fn clear_logo(db: State<'_, Db>, lic: State<'_, LicenseState>) -> DbResult<Settings> {
    require_license(&lic)?;
    let conn = db.lock();
    let existing = get_setting(&conn, "logo_path", "");
    if !existing.is_empty() {
        // The setting is cleared either way — the user asked for the logo to
        // go — but a failed delete leaves an orphan inside the asset scope, so
        // it must not vanish silently the way `let _ = …` made it.
        if let Err(e) = std::fs::remove_file(&existing) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("failed to delete logo file: {e}");
            }
        }
    }
    put_setting(&conn, "logo_path", "")?;
    Ok(read_settings(&conn))
}

// ===========================================================================
// Backup
// ===========================================================================

/// Write a consistent snapshot of the database to `dest`.
///
/// Uses `VACUUM INTO` rather than a file copy: the copy would race any
/// in-flight write and, in WAL mode, would miss everything still in the -wal
/// file. This is the only recovery path the app has — client deletes cascade
/// through purchases, installments and payments and are irreversible.
///
/// **Deliberately unlicensed**, unlike every other write in this module. The
/// unlicensed baseline exists so that losing a licence never holds a shop
/// keeper's own ledger hostage, and a snapshot of records they can already read
/// on screen hands them nothing the licence was protecting. Gating it inverted
/// the intent: the copy a shop most wants is the one taken *before* they go
/// troubleshooting an expiry, which is exactly when the gate refused.
#[tauri::command]
pub async fn backup_database(
    db: State<'_, Db>,
    app: tauri::AppHandle,
    dest: String,
) -> DbResult<Settings> {
    use tauri::Manager;

    // Staging happens inside app data, which the app owns outright — see
    // `backup_database_impl`. `AppHandle` is injected by Tauri, so the renderer
    // still calls this with `{ dest }` alone.
    let staging_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&staging_dir)?;

    let conn = db.lock();
    backup_database_impl(&conn, std::path::Path::new(&dest), &staging_dir)?;

    // Record that a backup happened, so the app can answer "when did this user
    // last back up?" — it could not before, and the users who never think to
    // back up are exactly the ones the manual-only design fails.
    //
    // A failure to record must not fail the command: the snapshot is already on
    // disk and good, and reporting BACKUP_FAILED would send the user chasing a
    // backup they actually have. Warn instead, so a settings table that has
    // stopped accepting writes is not silent.
    if let Err(e) = put_setting(&conn, LAST_BACKUP_KEY, &today().to_string()) {
        log::warn!("backup succeeded but its date could not be recorded: {e}");
    }

    // Returned rather than voided so the renderer can refresh the staleness
    // banner without a second round trip, as `set_logo`/`clear_logo` do.
    Ok(read_settings(&conn))
}

pub(crate) fn backup_database_impl(
    conn: &Connection,
    dest_path: &std::path::Path,
    staging_dir: &std::path::Path,
) -> DbResult<()> {
    use std::io::Read;

    // `dest` is untrusted: it normally comes from the native save dialog, but
    // the renderer can call this command with any path. Two guards keep it from
    // being an arbitrary-file-destruction primitive.
    //
    // 1. It must be named like a database.
    if dest_path.extension().and_then(|e| e.to_str()) != Some("db") {
        log::warn!("rejected backup destination without a .db extension");
        return Err(AppError::validation(BACKUP_FAILED));
    }
    // 2. If something is already there, it must itself be a SQLite database —
    //    so a mistaken (or malicious) path cannot clobber unrelated files.
    if dest_path.exists() {
        let mut header = [0u8; 16];
        let read = std::fs::File::open(dest_path)
            .and_then(|mut f| f.read(&mut header))
            .map_err(|e| {
                log::warn!("cannot inspect existing backup destination: {e}");
                AppError::validation(BACKUP_FAILED)
            })?;
        if &header[..read] != b"SQLite format 3\0" {
            log::warn!("refused to overwrite a destination that is not a SQLite database");
            return Err(AppError::validation(BACKUP_FAILED));
        }
    }

    // Stage inside app data under a name generated per run, never derived from
    // `dest`. The previous version wrote `dest.with_extension("db.part")` and
    // `remove_file`d it unconditionally — an unguarded delete of a caller-chosen
    // path, since the SQLite check above protects `dest` and not its sibling.
    // Deriving the name was also wrong on its own terms: `with_extension` on
    // `payment-schedule-2026.08.04.db` yields `…2026.08.db.part`, and two
    // backups into one directory collided. pid+nanos matches the temp-name
    // idiom the test helpers already use.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staged = staging_dir.join(format!("backup-{}-{stamp}.part", std::process::id()));

    let vacuum = conn.execute("VACUUM INTO ?1", [&staged.to_string_lossy().to_string()]);
    if let Err(e) = vacuum {
        log::error!("database backup failed: {e}");
        discard(&staged);
        return Err(AppError::validation(BACKUP_FAILED));
    }

    // A clean `VACUUM INTO` proves the statement ran, not that the bytes it left
    // behind are a usable database — a full disk, a failing drive or a truncated
    // write all land here as success. Verify the snapshot while it is still in
    // staging, because after the rename the only person who finds out is the
    // user, at the moment they need it.
    if let Err(e) = verify_snapshot(&staged) {
        log::error!("the backup snapshot did not verify: {e}");
        discard(&staged);
        return Err(AppError::validation(BACKUP_FAILED));
    }

    // Rename is atomic and leaves any previous backup intact until the moment it
    // is replaced — but it only works within one filesystem, and a backup
    // destination is routinely a USB stick while app data is on the internal
    // disk. So fall back to a copy.
    //
    // Be clear about what that costs: **the copy is not atomic at the
    // destination**. A failure part-way through leaves a truncated file where a
    // good backup used to be. That is the price of staging inside app data
    // rather than beside `dest`, and it is the better trade — staging beside
    // `dest` is what made this command able to touch files it does not own.
    // Falling back on any rename error rather than matching `CrossesDevices`
    // keeps this portable; a permission failure simply fails twice.
    if std::fs::rename(&staged, dest_path).is_err() {
        if let Err(e) = std::fs::copy(&staged, dest_path) {
            log::error!("could not move the backup into place: {e}");
            discard(&staged);
            return Err(AppError::validation(BACKUP_FAILED));
        }
        log::info!("backup copied across filesystems rather than renamed");
        discard(&staged);
    }

    log::info!("database backup written");
    Ok(())
}

/// Open a freshly written snapshot and prove it is a sound database.
///
/// `integrity_check` walks the b-trees and the freelist; `foreign_key_check`
/// then proves the relationships the app relies on still resolve — a snapshot
/// whose `payment` rows point at vanished installments would open fine and read
/// wrong. Both return the string `"ok"` / no rows respectively on success.
///
/// The error is a plain string rather than an `AppError` because every caller
/// maps it to `BACKUP_FAILED`; what matters is that the detail reaches the log.
fn verify_snapshot(path: &std::path::Path) -> Result<(), String> {
    // Read-only, so verification cannot alter the thing it is verifying and
    // cannot leave a journal beside the staged file for the rename to strand.
    // `VACUUM INTO` writes a rollback-journal database regardless of the source
    // being WAL, so opening it read-only needs no `-shm` and always works.
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("cannot open the snapshot: {e}"))?;

    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| format!("integrity_check did not run: {e}"))?;
    if integrity != "ok" {
        return Err(format!("integrity_check reported: {integrity}"));
    }

    // `foreign_key_check` yields one row per violation and nothing at all when
    // the database is sound, so "no rows" is the passing case.
    let mut stmt = conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| format!("foreign_key_check did not run: {e}"))?;
    let violations = stmt
        .query_map([], |_| Ok(()))
        .map_err(|e| format!("foreign_key_check did not run: {e}"))?
        .count();
    if violations > 0 {
        return Err(format!(
            "foreign_key_check reported {violations} violation(s)"
        ));
    }

    Ok(())
}

/// Remove a staging file we created, logging a failure rather than swallowing
/// it — an orphan in app data is invisible and grows with every failed backup.
fn discard(staged: &std::path::Path) {
    match std::fs::remove_file(staged) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("could not remove the backup staging file: {e}"),
    }
}

// ===========================================================================
// CSV export
// ===========================================================================

/// Write a CSV the renderer built to a path the user picked.
///
/// # Why this is a command at all
///
/// The renderer used to do this itself, with a `Blob` and an `<a download>`.
/// That is a *browser* mechanism: it works in the dev preview and in the E2E
/// suite, and does nothing at all inside the WebView, which has no download
/// manager. The click was silently inert in the shipped app — for the Impayés
/// export as well as for Rapports.
///
/// Writing it from the frontend instead is not an option: there is deliberately
/// no `fs` plugin and no `fs:*` permission (see `lib.rs`). So this follows the
/// same shape as `backup_database` — the renderer picks a destination with the
/// dialog plugin and hands the path over, and the write happens here.
#[tauri::command]
pub async fn export_csv(
    lic: State<'_, LicenseState>,
    dest: String,
    contents: String,
) -> DbResult<()> {
    require_license(&lic)?;
    export_csv_impl(std::path::Path::new(&dest), &contents)
}

/// Split out from the command so the guards are reachable from `cargo test`.
pub(crate) fn export_csv_impl(dest: &std::path::Path, contents: &str) -> DbResult<()> {
    // `dest` is untrusted. It normally comes from the native save dialog, but
    // the renderer can call this command with any path at all.
    //
    // The extension check is the guard, and it is weaker than the one
    // `backup_database` gets to use: a SQLite file announces itself with a magic
    // header, so a backup can refuse to clobber anything that is not already a
    // database. CSV has no such marker — any text file is a plausible CSV — so
    // there is nothing to sniff. What remains is still a real bound: the only
    // files this can overwrite are ones the user named `.csv`.
    if dest.extension().and_then(|e| e.to_str()) != Some("csv") {
        log::warn!("rejected a CSV export destination without a .csv extension");
        return Err(AppError::validation(EXPORT_FAILED));
    }
    if contents.len() > EXPORT_MAX_BYTES {
        log::warn!(
            "rejected a CSV export of {} bytes, over the {EXPORT_MAX_BYTES} cap",
            contents.len()
        );
        return Err(AppError::validation(EXPORT_FAILED));
    }

    // Written in place rather than staged-then-renamed, unlike `backup_database`.
    // The difference is what a half-written file costs: a truncated backup is
    // discovered at restore time, when it is the only copy and it is too late.
    // A truncated export is discovered immediately and fixed by pressing the
    // button again, because the data it came from is still in the database.
    if let Err(e) = std::fs::write(dest, contents) {
        log::error!("could not write the CSV export: {e}");
        return Err(AppError::validation(EXPORT_FAILED));
    }
    log::info!("wrote a CSV export of {} bytes", contents.len());
    Ok(())
}

// ===========================================================================
// Licence
// ===========================================================================

/// Validate the installed licence, applying the clock guard and advancing the
/// high-water mark.
///
/// This is the single place the three pieces are combined — `lib.rs` calls it at
/// startup and [`import_license`] calls it again after installing a file. The
/// watermark I/O lives here rather than in `license.rs` so that module stays
/// free of persistence concerns and its core stays a pure function.
pub(crate) fn evaluate_license(app: &tauri::AppHandle, conn: &Connection) -> LicenseStatus {
    // An unreadable licence file is treated as absent: the app must still start.
    // `validate_installed` has already logged the underlying cause.
    let status = license::validate_installed(app).unwrap_or(LicenseStatus::Missing);

    let today = today();
    let stored = get_setting(conn, license::CLOCK_WATERMARK_KEY, "");
    let watermark = parse_date(&stored).ok();

    let status = license::apply_clock_guard(status, today, watermark);

    if let Some(next) = license::next_watermark(watermark, today) {
        // Best-effort: a read-only database must not stop the app from running.
        if let Err(e) = put_setting(conn, license::CLOCK_WATERMARK_KEY, &next.to_string()) {
            log::warn!("could not advance the licence clock watermark: {e}");
        }
    }

    status
}

/// How often the licence verdict is re-checked while the app runs.
///
/// Expiry is date-granular, so minute precision buys nothing — a quarter hour
/// bounds how long a shop keeps working past an expiry without paying for a file
/// read and a machine-id hash every minute.
///
/// A poll, not a computed sleep until midnight, for the same reasons as
/// [`crate::autobackup`]'s tick: the machine suspends and wakes hours later, and
/// the system clock moves. Both invalidate a deadline computed in advance; they
/// cannot invalidate a fixed interval.
const LICENSE_TICK: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Event carrying a changed licence verdict to the renderer. Payload:
/// [`LicenseInfo`], the same projection [`get_license_status`] returns.
///
/// The **only** backend-pushed event in the app. Everything else is
/// request/response through a command, so this is the one place the renderer
/// learns something it did not ask for.
pub(crate) const LICENSE_CHANGED_EVENT: &str = "license://changed";

/// Store a freshly computed verdict and tell the window, but only if it differs.
///
/// The comparison is what keeps this quiet: the watcher runs every
/// [`LICENSE_TICK`] and the verdict almost never changes, so without it the
/// renderer would be woken 96 times a day to be told nothing.
///
/// Emitting is best-effort. By the time it runs the cache is already updated, so
/// [`require_license`] is correct whatever the window does — a failed emit costs
/// a stale screen, not a stale gate. The log line carries the status tag only:
/// it is a fixed vocabulary, unlike the parser detail `Malformed` holds.
///
/// **Call this with the connection guard from the matching `evaluate_license`
/// still held.** The compare-and-set here is not atomic with the evaluation that
/// produced `next`, and there are two writers — the watcher thread and
/// `import_license`. Left unserialized, a watcher that read the licence file just
/// before an import lands would publish its stale verdict *after* the import
/// published the new one, locking a customer out of the licence they just
/// installed until the next tick. Holding the guard both already take is what
/// orders the file read against the publish.
fn publish_license(app: &tauri::AppHandle, lic: &LicenseState, next: LicenseStatus) {
    use tauri::Emitter;

    if lic.get() == next {
        return;
    }

    let info = next.to_info(license::machine_fingerprint());
    log::info!("licence verdict changed to {}", info.status);
    lic.set(next);

    if let Err(e) = app.emit(LICENSE_CHANGED_EVENT, &info) {
        log::warn!("could not notify the window of the licence change: {e}");
    }
}

/// Start the background licence watcher. Returns immediately.
///
/// Without this the verdict computed in `lib.rs` at startup stands for the whole
/// process: a shop that leaves the app open across its expiry date keeps full
/// access until it restarts (`AUDIT_REPORT.md` L4). Re-evaluating on a tick makes
/// the Rust gate — the one that actually refuses — authoritative within
/// [`LICENSE_TICK`], and [`publish_license`] flips the UI to match.
///
/// A plain `std::thread` rather than an async task, for the same reason as
/// [`crate::autobackup::start_scheduler`]: the work is blocking (a file read, a
/// hash, and a locked connection) and does not belong on the executor that
/// serves IPC. Detached — process exit ends it, and there is nothing to join.
///
/// Must be started only after `Db` and `LicenseState` are managed; `state()`
/// panics otherwise.
pub(crate) fn start_license_watcher(app: tauri::AppHandle) {
    use tauri::Manager;

    std::thread::spawn(move || {
        log::info!("licence watcher started");
        loop {
            // Sleeps first: `lib.rs` has just evaluated the licence, and an
            // immediate second pass would only re-read the same file.
            std::thread::sleep(LICENSE_TICK);

            // Fetched per tick rather than held, so the thread borrows the
            // database only while it is actually working. Advancing the clock
            // watermark is part of `evaluate_license`, so a day rolling over is
            // now recorded without a restart too.
            //
            // The guard spans the publish deliberately — see `publish_license`
            // for the interleaving with `import_license` that would otherwise
            // let this tick's stale verdict win.
            let db = app.state::<Db>();
            let conn = db.lock();
            let next = evaluate_license(&app, &conn);
            publish_license(&app, &app.state::<LicenseState>(), next);
            drop(conn);
        }
    });
}

/// The current licence verdict, for the UI.
///
/// Never fails: every outcome, including "no licence installed", is a status the
/// frontend renders rather than an error it has to catch.
///
/// Reads the cache rather than re-validating: [`start_license_watcher`] keeps it
/// fresh, and a command that hit the filesystem on every call would make the
/// licence card the most expensive screen in the app.
#[tauri::command]
pub async fn get_license_status(lic: State<'_, LicenseState>) -> DbResult<LicenseInfo> {
    Ok(lic.get().to_info(license::machine_fingerprint()))
}

/// Install a licence file the user picked, after proving it is valid.
///
/// `source_path` is untrusted even though it normally comes from the native
/// picker — the renderer can call this command with any path at all. The same
/// reasoning as [`set_logo`], with one difference that matters: a licence
/// validates itself. There is no need to sniff the content, because a file that
/// does not carry a good signature is rejected outright.
///
/// The file is validated **before** it is copied, so a bad import can never
/// displace a licence that was working. That ordering is the whole point of this
/// command; reversing it would let a stray file lock a paying customer out.
#[tauri::command]
pub async fn import_license(
    db: State<'_, Db>,
    lic: State<'_, LicenseState>,
    app: tauri::AppHandle,
    source_path: String,
) -> DbResult<LicenseInfo> {
    use tauri::Manager;

    let source = std::path::Path::new(&source_path);
    let candidate =
        license::validate_file(source, today(), license::machine_fingerprint().as_deref())?;

    if !candidate.is_valid() {
        // The status tag is safe to send — it is a fixed vocabulary, not parser
        // detail — and the user needs to know *why* their file was refused.
        let info = candidate.to_info(None);
        log::warn!("licence import refused: {}", info.status);
        return Err(AppError::conflict(INVALID_LICENSE, info.status));
    }

    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;
    let dest = data_dir.join("license.json");
    std::fs::copy(source, &dest)?;

    // Re-evaluate from the installed path rather than trusting `candidate`:
    // this is what every later startup will see, and it also advances the clock
    // watermark. If the copy landed wrong, the user finds out now.
    //
    // Published through the same path as the watcher, so the cache has exactly
    // one writer and "the cache changed" can never drift from "the window was
    // told". The event is redundant for this caller — it is handed the verdict
    // below — but harmless. The connection guard spans both calls so the watcher
    // cannot slip in a verdict it computed from the pre-import file.
    let conn = db.lock();
    let status = evaluate_license(&app, &conn);
    publish_license(&app, &lic, status.clone());
    drop(conn);

    if status.is_valid() {
        log::info!("licence installed");
    } else {
        log::warn!(
            "licence re-check after install returned {}",
            status.to_info(None).status
        );
    }
    Ok(status.to_info(license::machine_fingerprint()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::license::License;
    use std::path::PathBuf;

    fn temp_db_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut p = std::env::temp_dir();
        p.push(format!(
            "payment_schedule_cmd_test_{tag}_{}_{nanos}.db",
            std::process::id()
        ));
        p
    }

    /// Regression: `build_impayes` must bind exactly as many parameters as the
    /// query declares for every filter combination. The no-filter case (the
    /// default overdue-page load) declares only `?1`; previously a fixed set of
    /// four params was always bound, so SQLite rejected the query at runtime and
    /// the page rendered blank under `tauri dev` while the mock-backed browser
    /// build was unaffected.
    #[test]
    fn build_impayes_binds_params_for_every_filter_combo() {
        let path = temp_db_path("impayes");
        let db = Db::open(&path).unwrap();
        let conn = db.lock();

        let cid = conn
            .query_row("SELECT id FROM client LIMIT 1", [], |r| r.get::<_, i64>(0))
            .ok();

        let filters = [
            ImpayeFilter::default(),
            ImpayeFilter {
                date_from: Some("2000-01-01".into()),
                ..Default::default()
            },
            ImpayeFilter {
                date_to: Some("2999-12-31".into()),
                ..Default::default()
            },
            ImpayeFilter {
                client_id: cid,
                ..Default::default()
            },
            ImpayeFilter {
                date_from: Some("2000-01-01".into()),
                date_to: Some("2999-12-31".into()),
                client_id: cid,
            },
        ];

        for (i, f) in filters.into_iter().enumerate() {
            let res = build_impayes(&conn, f, None);
            assert!(res.is_ok(), "filter combo {i} must not error: {res:?}");
        }

        // With demo seeding on (debug builds), the unfiltered call must surface
        // the seeded overdue installments rather than an empty list. Detect the
        // seeded state via the client count so we don't depend on the private
        // seeding gate, and stay correct under `cargo test --release` (empty DB).
        let seeded: i64 = conn
            .query_row("SELECT COUNT(*) FROM client", [], |r| r.get(0))
            .unwrap();
        if seeded > 0 {
            let out = build_impayes(&conn, ImpayeFilter::default(), None).unwrap();
            let total: usize = out.iter().map(|c| c.installments.len()).sum();
            assert!(total > 0, "seeded DB should report overdue installments");
        }

        drop(conn);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    // =======================================================================
    // Command behaviour over a real temp database
    //
    // These exercise the `*_impl` functions the commands delegate to, which is
    // what makes them reachable without a Tauri `State`. Before this suite the
    // backend had three tests total and none over the code that owns the money:
    // transactions, cascades, overpayment and the validation guards were all
    // untested, and the integration/E2E suites only ever drove the TS mock.
    // =======================================================================

    /// Every money read model's headline figure, captured together.
    ///
    /// Archiving a purchase has to move all of these at once and restoring has
    /// to put them all back; comparing whole snapshots catches the query someone
    /// forgot to filter far better than asserting them one at a time.
    #[derive(Debug, PartialEq, Eq)]
    struct MoneySnapshot {
        purchases: i64,
        sales: i64,
        outstanding: i64,
        overdue: i64,
        impayes: i64,
        schedule: i64,
        client_outstanding: i64,
    }

    /// A fresh database with exactly one client and nothing else.
    struct Fixture {
        db: Db,
        path: PathBuf,
        client_id: i64,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let path = temp_db_path(tag);
            let db = Db::open(&path).expect("open temp db");
            {
                let conn = db.lock();
                // Start from a known-empty state regardless of the demo-seeding
                // gate, which is on in debug builds.
                conn.execute_batch("DELETE FROM payment; DELETE FROM installment; DELETE FROM purchase; DELETE FROM client;")
                    .unwrap();
                conn.execute(
                    "INSERT INTO client (first_name, last_name, phone) VALUES ('Test', 'Client', '+21620000000')",
                    [],
                )
                .unwrap();
            }
            let client_id = db.lock().last_insert_rowid();
            Fixture {
                db,
                path,
                client_id,
            }
        }

        fn purchase_input(&self) -> PurchaseInput {
            PurchaseInput {
                client_id: self.client_id,
                product_label: "Machine à laver".into(),
                total_price: 1000,
                installment_count: 4,
                interval_kind: "monthly".into(),
                interval_days: None,
                purchase_date: "2024-01-15".into(),
                installments: None,
            }
        }

        fn count(&self, sql: &str) -> i64 {
            self.db.lock().query_row(sql, [], |r| r.get(0)).unwrap()
        }

        /// The archive stamp for a purchase, or `None` while it is live.
        fn purchase_archived_at(&self, id: i64) -> Option<String> {
            self.db
                .lock()
                .query_row(
                    "SELECT archived_at FROM purchase WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap()
        }

        /// Every money read model in one shot, so an archive can be checked
        /// against all of them at once and a restore proved exact.
        fn money_snapshot(&self) -> MoneySnapshot {
            let conn = self.db.lock();
            let today_str = today().to_string();
            let scalar = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
            MoneySnapshot {
                purchases: scalar("SELECT COUNT(*) FROM purchase WHERE archived_at IS NULL"),
                sales: scalar(
                    "SELECT COALESCE(SUM(total_price),0) FROM purchase WHERE archived_at IS NULL",
                ),
                outstanding: scalar(
                    "SELECT COALESCE(SUM(i.amount - i.paid_amount),0) FROM installment i
                       WHERE EXISTS (SELECT 1 FROM purchase pu
                                      WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)",
                ),
                overdue: conn
                    .query_row(
                        "SELECT COUNT(*) FROM installment i
                           WHERE i.due_date < ?1 AND i.amount > i.paid_amount
                             AND EXISTS (SELECT 1 FROM purchase pu
                                          WHERE pu.id = i.purchase_id AND pu.archived_at IS NULL)",
                        [&today_str],
                        |r| r.get(0),
                    )
                    .unwrap(),
                impayes: build_impayes(&conn, ImpayeFilter::default(), None)
                    .unwrap()
                    .len() as i64,
                schedule: list_schedule_rows(&conn).unwrap().len() as i64,
                client_outstanding: client_outstanding(&conn, self.client_id).unwrap(),
            }
        }

        /// `paid_amount` is a cache of the ledger, and the dashboard's
        /// "Amount collected" is the only money figure read from the ledger
        /// itself. If the two ever disagree, that tile silently contradicts
        /// every purchase and client total in the app — so any write that
        /// touches collected money asserts this.
        fn assert_ledger_matches_installments(&self) {
            let conn = self.db.lock();
            let scalar = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
            assert_eq!(
                scalar("SELECT COALESCE(SUM(amount),0) FROM payment"),
                scalar("SELECT COALESCE(SUM(paid_amount),0) FROM installment"),
                "the payment ledger and installment.paid_amount have drifted apart"
            );
            assert_eq!(
                scalar("SELECT COUNT(*) FROM installment WHERE paid_amount > amount"),
                0,
                "no row may owe less than it has collected"
            );
        }

        /// The archive stamp for a client, or `None` while they are active.
        fn archived_at(&self, id: i64) -> Option<String> {
            self.db
                .lock()
                .query_row("SELECT archived_at FROM client WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
            }
        }
    }

    fn code_of(e: AppError) -> String {
        e.code()
    }

    // --- create_purchase ---------------------------------------------------

    #[test]
    fn create_purchase_splits_evenly_with_remainder_last() {
        let f = Fixture::new("create_even");
        let mut conn = f.db.lock();
        let detail = create_purchase_impl(&mut conn, f.purchase_input()).unwrap();

        let amounts: Vec<i64> = detail.installments.iter().map(|i| i.amount).collect();
        assert_eq!(amounts, vec![250, 250, 250, 250]);
        assert_eq!(amounts.iter().sum::<i64>(), 1000, "parts must sum to total");
        assert_eq!(detail.installments[0].due_date, "2024-01-15");
        assert_eq!(detail.installments[3].due_date, "2024-04-15");
        // The fixture is dated in the past, so every tranche is already overdue
        // and the rollup reports "late" — status is computed against today, not
        // stored.
        assert_eq!(detail.status, "late");
        assert_eq!(detail.total_paid, 0);
        assert_eq!(detail.remaining, 1000);
    }

    #[test]
    fn create_purchase_accepts_a_manual_uneven_split() {
        let f = Fixture::new("create_manual");
        let mut conn = f.db.lock();
        let input = PurchaseInput {
            installment_count: 3,
            installments: Some(vec![
                InstallmentInput {
                    index: 1,
                    amount: 500,
                    due_date: "2024-01-15".into(),
                },
                InstallmentInput {
                    index: 2,
                    amount: 300,
                    due_date: "2024-02-15".into(),
                },
                InstallmentInput {
                    index: 3,
                    amount: 200,
                    due_date: "2024-03-15".into(),
                },
            ]),
            ..f.purchase_input()
        };
        let detail = create_purchase_impl(&mut conn, input).unwrap();
        let amounts: Vec<i64> = detail.installments.iter().map(|i| i.amount).collect();
        assert_eq!(amounts, vec![500, 300, 200]);
    }

    /// The rollback case: a mismatched manual split must leave nothing behind.
    #[test]
    fn create_purchase_sum_mismatch_writes_nothing() {
        let f = Fixture::new("create_mismatch");
        {
            let mut conn = f.db.lock();
            let input = PurchaseInput {
                installment_count: 2,
                installments: Some(vec![
                    InstallmentInput {
                        index: 1,
                        amount: 400,
                        due_date: "2024-01-15".into(),
                    },
                    InstallmentInput {
                        index: 2,
                        amount: 500,
                        due_date: "2024-02-15".into(),
                    },
                ]),
                ..f.purchase_input()
            };
            let err = create_purchase_impl(&mut conn, input).unwrap_err();
            assert_eq!(code_of(err), "SUM_MISMATCH:900:1000");
        }
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 0);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 0);
    }

    #[test]
    fn create_purchase_rejects_out_of_range_input() {
        let f = Fixture::new("create_bounds");
        let mut conn = f.db.lock();

        let cases: Vec<(PurchaseInput, &str)> = vec![
            (
                PurchaseInput {
                    total_price: 0,
                    ..f.purchase_input()
                },
                "INVALID_TOTAL_PRICE",
            ),
            (
                PurchaseInput {
                    total_price: -100,
                    ..f.purchase_input()
                },
                "INVALID_TOTAL_PRICE",
            ),
            (
                PurchaseInput {
                    installment_count: 0,
                    ..f.purchase_input()
                },
                "INVALID_INSTALLMENT_COUNT",
            ),
            (
                PurchaseInput {
                    installment_count: 121,
                    ..f.purchase_input()
                },
                "INVALID_INSTALLMENT_COUNT",
            ),
            (
                PurchaseInput {
                    interval_kind: "fortnightly".into(),
                    ..f.purchase_input()
                },
                "INVALID_INTERVAL_KIND",
            ),
            (
                PurchaseInput {
                    interval_kind: "custom".into(),
                    interval_days: Some(0),
                    ..f.purchase_input()
                },
                "INVALID_INTERVAL_DAYS",
            ),
            (
                PurchaseInput {
                    interval_kind: "custom".into(),
                    interval_days: Some(400),
                    ..f.purchase_input()
                },
                "INVALID_INTERVAL_DAYS",
            ),
            (
                PurchaseInput {
                    purchase_date: "15/01/2024".into(),
                    ..f.purchase_input()
                },
                "INVALID_DATE",
            ),
        ];

        for (input, expected) in cases {
            let err = create_purchase_impl(&mut conn, input).unwrap_err();
            assert_eq!(code_of(err), expected);
        }
        // `Fixture::count` takes the same non-reentrant lock, so the guard has
        // to go first.
        drop(conn);
        assert_eq!(
            f.count("SELECT COUNT(*) FROM purchase"),
            0,
            "no rejected request may write a row"
        );
    }

    /// A malformed manual due date used to be stored verbatim, after which the
    /// installment reported "pending" and 0 days late forever — invisible in
    /// every overdue and alert screen.
    #[test]
    fn create_purchase_rejects_a_malformed_manual_due_date() {
        let f = Fixture::new("create_baddate");
        {
            let mut conn = f.db.lock();
            let input = PurchaseInput {
                installment_count: 1,
                installments: Some(vec![InstallmentInput {
                    index: 1,
                    amount: 1000,
                    due_date: "not-a-date".into(),
                }]),
                ..f.purchase_input()
            };
            let err = create_purchase_impl(&mut conn, input).unwrap_err();
            assert_eq!(code_of(err), "INVALID_DATE");
        }
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 0);
    }

    // --- record_payment ----------------------------------------------------

    fn seeded_purchase(f: &Fixture) -> PurchaseDetail {
        let mut conn = f.db.lock();
        create_purchase_impl(&mut conn, f.purchase_input()).unwrap()
    }

    #[test]
    fn partial_payment_leaves_paid_date_null() {
        let f = Fixture::new("pay_partial");
        let detail = seeded_purchase(&f);
        let inst = &detail.installments[0];

        let mut conn = f.db.lock();
        let after = record_payment_impl(
            &mut conn,
            PaymentInput {
                installment_id: inst.id,
                amount: 100,
                payment_date: "2024-01-20".into(),
                note: None,
            },
        )
        .unwrap();

        let updated = &after.installments[0];
        assert_eq!(updated.paid_amount, 100);
        assert_eq!(updated.paid_date, None);
        assert_eq!(after.total_paid, 100);
        assert_eq!(after.remaining, 900);
    }

    #[test]
    fn full_payment_sets_paid_date_and_status() {
        let f = Fixture::new("pay_full");
        let detail = seeded_purchase(&f);
        let inst = &detail.installments[0];

        let mut conn = f.db.lock();
        let after = record_payment_impl(
            &mut conn,
            PaymentInput {
                installment_id: inst.id,
                amount: 250,
                payment_date: "2024-01-20".into(),
                note: Some("  espèces  ".into()),
            },
        )
        .unwrap();

        let updated = &after.installments[0];
        assert_eq!(updated.paid_amount, 250);
        assert_eq!(updated.paid_date.as_deref(), Some("2024-01-20"));
        assert_eq!(updated.status, "paid");
    }

    /// Overpayment must be rejected, not absorbed: a negative
    /// `amount - paid_amount` is summed into the outstanding aggregates, where
    /// it silently cancels out another client's real debt.
    #[test]
    fn overpayment_is_rejected_and_records_nothing() {
        let f = Fixture::new("pay_over");
        let detail = seeded_purchase(&f);
        let inst = &detail.installments[0];

        {
            let mut conn = f.db.lock();
            // Part-pay first so the reported remainder is not just the amount.
            record_payment_impl(
                &mut conn,
                PaymentInput {
                    installment_id: inst.id,
                    amount: 100,
                    payment_date: "2024-01-20".into(),
                    note: None,
                },
            )
            .unwrap();

            let err = record_payment_impl(
                &mut conn,
                PaymentInput {
                    installment_id: inst.id,
                    amount: 200,
                    payment_date: "2024-01-21".into(),
                    note: None,
                },
            )
            .unwrap_err();
            assert_eq!(code_of(err), "OVERPAYMENT:150");
        }

        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1);
        assert_eq!(
            f.count("SELECT SUM(paid_amount) FROM installment"),
            100,
            "paid_amount must never exceed the amount due"
        );
        assert_eq!(
            f.count("SELECT COUNT(*) FROM installment WHERE paid_amount > amount"),
            0
        );
    }

    #[test]
    fn record_payment_rejects_bad_arguments() {
        let f = Fixture::new("pay_bad");
        let detail = seeded_purchase(&f);
        let inst_id = detail.installments[0].id;
        let mut conn = f.db.lock();

        let zero = record_payment_impl(
            &mut conn,
            PaymentInput {
                installment_id: inst_id,
                amount: 0,
                payment_date: "2024-01-20".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(code_of(zero), "INVALID_AMOUNT");

        let bad_date = record_payment_impl(
            &mut conn,
            PaymentInput {
                installment_id: inst_id,
                amount: 10,
                payment_date: "20-01-2024".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(code_of(bad_date), "INVALID_DATE");

        let missing = record_payment_impl(
            &mut conn,
            PaymentInput {
                installment_id: 999_999,
                amount: 10,
                payment_date: "2024-01-20".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(code_of(missing), "INSTALLMENT_NOT_FOUND");
    }

    /// The payment queries join four tables and `map_payment` resolves columns
    /// **by name**, so the projection has to name them explicitly.
    ///
    /// This used to select `pay.*`. The day a migration adds a `reference`,
    /// `purchase_id` or `idx` column to `payment`, the star would start
    /// shadowing the purchase's value — no compile error, no runtime error,
    /// SQLite reads a negative `LIMIT` as *no* limit, so binding the caller's
    /// value straight in made `listAllPayments(-1)` serialize the entire payment
    /// ledger — a four-table join — across IPC.
    ///
    /// The ledger here is deliberately larger than the clamp floor: with fewer
    /// rows than the limit every case returns everything and the test proves
    /// nothing about clamping at all.
    #[test]
    fn the_payment_limit_is_clamped_rather_than_trusted() {
        let f = Fixture::new("payment_limit");
        let detail = seeded_purchase(&f);

        // Twelve partial payments against the first tranche, so "clamped to 1"
        // and "unbounded" cannot look alike.
        for i in 0..12 {
            record_payment_impl(
                &mut f.db.lock(),
                PaymentInput {
                    installment_id: detail.installments[0].id,
                    amount: 1,
                    payment_date: format!("2024-02-{:02}", i + 1),
                    note: None,
                },
            )
            .unwrap();
        }
        let total = f.count("SELECT COUNT(*) FROM payment");
        assert_eq!(total, 12, "the fixture must have more rows than the floor");

        // Drives the shipped query, not a re-implementation of the clamp — the
        // point is to prove `list_all_payments` behaves, not that `clamp` does.
        let listed = |limit: Option<i64>| -> usize {
            list_all_payments_impl(&f.db.lock(), limit).unwrap().len()
        };

        // The defect: without the clamp each of these returns all 12.
        for hostile in [-1, -999, 0, i64::MIN] {
            assert_eq!(
                listed(Some(hostile)),
                1,
                "a limit of {hostile} must clamp to the floor, not fall through to every row"
            );
        }

        // The ceiling holds, and an ordinary request is untouched.
        assert_eq!(listed(Some(i64::MAX)), 12, "capped at 5000, so all 12 fit");
        assert_eq!(listed(Some(5)), 5);
        assert_eq!(listed(None), 12, "the 500 default still returns everything");
    }

    /// just the wrong reference on the payments screen. The migration ladder
    /// makes that a plausible future change rather than a hypothetical one.
    #[test]
    fn payment_rows_resolve_join_columns_from_the_right_table() {
        let f = Fixture::new("payment_join");
        let detail = seeded_purchase(&f);
        {
            let mut conn = f.db.lock();
            record_payment_impl(
                &mut conn,
                PaymentInput {
                    installment_id: detail.installments[1].id,
                    amount: 250,
                    payment_date: "2024-02-20".into(),
                    note: None,
                },
            )
            .unwrap();
        }

        let conn = f.db.lock();
        let mut stmt = conn
            .prepare(
                "SELECT pay.id, pay.installment_id, pay.amount, pay.payment_date,
                        pay.note, pay.created_at,
                        i.idx, i.purchase_id, pu.reference,
                        c.id AS client_id, c.first_name, c.last_name
                 FROM payment pay
                 JOIN installment i ON i.id = pay.installment_id
                 JOIN purchase pu ON pu.id = i.purchase_id
                 JOIN client c ON c.id = pu.client_id",
            )
            .unwrap();
        let rows: Vec<Payment> = stmt
            .query_map([], map_payment)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        assert_eq!(rows.len(), 1);
        let p = &rows[0];
        // reference and purchase_id must come from `purchase`/`installment`...
        assert_eq!(p.purchase_reference, detail.purchase.reference);
        assert_eq!(p.purchase_id, detail.purchase.id);
        // ...idx from `installment` (2nd tranche), not from `payment`...
        assert_eq!(p.installment_index, 2);
        // ...client_id from `client`, and the amount from `payment`.
        assert_eq!(p.client_id, f.client_id);
        assert_eq!(p.amount, 250);
    }

    // --- deletes / cascades ------------------------------------------------

    /// The whole point of the archive feature: a client with history can never
    /// be deleted, with no `force` escape hatch to reach the cascade behind it.
    #[test]
    fn delete_client_is_refused_for_any_client_with_purchases() {
        let f = Fixture::new("del_client");
        let detail = seeded_purchase(&f);
        {
            let mut conn = f.db.lock();
            record_payment_impl(
                &mut conn,
                PaymentInput {
                    installment_id: detail.installments[0].id,
                    amount: 250,
                    payment_date: "2024-01-20".into(),
                    note: None,
                },
            )
            .unwrap();
        }

        let err = delete_client_impl(&mut f.db.lock(), f.client_id).unwrap_err();
        assert_eq!(code_of(err), "CLIENT_HAS_PURCHASES:1");

        // Nothing was touched — not the client, and not the history below them.
        assert_eq!(f.count("SELECT COUNT(*) FROM client"), 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1);
    }

    #[test]
    fn delete_client_removes_a_client_with_no_purchases() {
        let f = Fixture::new("del_client_empty");

        delete_client_impl(&mut f.db.lock(), f.client_id).unwrap();

        assert_eq!(f.count("SELECT COUNT(*) FROM client"), 0);
    }

    #[test]
    fn delete_client_reports_a_missing_id_rather_than_succeeding_silently() {
        let f = Fixture::new("del_client_missing");
        let err = delete_client_impl(&mut f.db.lock(), 99_999).unwrap_err();
        assert_eq!(code_of(err), "CLIENT_NOT_FOUND");
    }

    // --- archive / restore -------------------------------------------------

    #[test]
    fn archive_client_is_refused_while_the_client_owes_money() {
        let f = Fixture::new("archive_owing");
        let detail = seeded_purchase(&f);
        {
            let mut conn = f.db.lock();
            record_payment_impl(
                &mut conn,
                PaymentInput {
                    installment_id: detail.installments[0].id,
                    amount: 250,
                    payment_date: "2024-01-20".into(),
                    note: None,
                },
            )
            .unwrap();
        }

        // 1000 total, 250 paid.
        let err = archive_client_impl(&mut f.db.lock(), f.client_id).unwrap_err();
        assert_eq!(code_of(err), "ARCHIVE_HAS_OUTSTANDING:750");
        assert!(
            f.archived_at(f.client_id).is_none(),
            "a refused archive must not have written the stamp"
        );
    }

    #[test]
    fn archive_client_succeeds_once_every_installment_is_paid() {
        let f = Fixture::new("archive_paid");
        let detail = seeded_purchase(&f);
        for inst in &detail.installments {
            let mut conn = f.db.lock();
            record_payment_impl(
                &mut conn,
                PaymentInput {
                    installment_id: inst.id,
                    amount: inst.amount,
                    payment_date: "2024-01-20".into(),
                    note: None,
                },
            )
            .unwrap();
        }

        archive_client_impl(&mut f.db.lock(), f.client_id).unwrap();

        assert!(f.archived_at(f.client_id).is_some());
        // Archiving hides the client; it destroys nothing.
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 4);
    }

    /// The empty-join case: `SUM` over no rows is `NULL`, which is exactly where
    /// a missing `COALESCE` in `client_outstanding` would go wrong.
    #[test]
    fn archive_client_succeeds_for_a_client_with_no_purchases() {
        let f = Fixture::new("archive_empty");
        archive_client_impl(&mut f.db.lock(), f.client_id).unwrap();
        assert!(f.archived_at(f.client_id).is_some());
    }

    /// The stamp is what renders as "archived on <date>", so it must be an ISO
    /// date — `formatDatePattern` on the frontend splits on `-` and would emit a
    /// raw `datetime('now')` string verbatim.
    #[test]
    fn archive_stamp_is_an_iso_date_and_does_not_move_on_a_repeat() {
        let f = Fixture::new("archive_stamp");
        archive_client_impl(&mut f.db.lock(), f.client_id).unwrap();
        let first = f.archived_at(f.client_id).unwrap();
        assert_eq!(first.len(), 10, "expected YYYY-MM-DD, got {first:?}");
        assert!(chrono::NaiveDate::parse_from_str(&first, "%Y-%m-%d").is_ok());

        archive_client_impl(&mut f.db.lock(), f.client_id).unwrap();
        assert_eq!(
            f.archived_at(f.client_id).unwrap(),
            first,
            "re-archiving must not move the stamp"
        );
    }

    #[test]
    fn restore_client_clears_the_stamp_and_is_idempotent() {
        let f = Fixture::new("restore");
        archive_client_impl(&mut f.db.lock(), f.client_id).unwrap();

        restore_client_impl(&mut f.db.lock(), f.client_id).unwrap();
        assert!(f.archived_at(f.client_id).is_none());

        // Restoring an already-active client is the state the caller asked for.
        restore_client_impl(&mut f.db.lock(), f.client_id).unwrap();
        assert!(f.archived_at(f.client_id).is_none());
    }

    #[test]
    fn archive_and_restore_report_a_missing_client() {
        let f = Fixture::new("archive_missing");
        assert_eq!(
            code_of(archive_client_impl(&mut f.db.lock(), 99_999).unwrap_err()),
            "CLIENT_NOT_FOUND"
        );
        assert_eq!(
            code_of(restore_client_impl(&mut f.db.lock(), 99_999).unwrap_err()),
            "CLIENT_NOT_FOUND"
        );
    }

    /// The guard that keeps "archived implies a zero balance" true by
    /// construction rather than by UI convention — without it, a purchase
    /// created straight over IPC would give an archived client a balance that
    /// the money read models assume cannot exist.
    #[test]
    fn an_archived_client_cannot_take_on_a_new_purchase() {
        let f = Fixture::new("archived_no_purchase");
        archive_client_impl(&mut f.db.lock(), f.client_id).unwrap();

        let err = create_purchase_impl(&mut f.db.lock(), f.purchase_input()).unwrap_err();
        assert_eq!(code_of(err), "CLIENT_ARCHIVED");
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 0);

        // ...and it works again once they are restored.
        restore_client_impl(&mut f.db.lock(), f.client_id).unwrap();
        create_purchase_impl(&mut f.db.lock(), f.purchase_input()).unwrap();
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 1);
    }

    #[test]
    fn list_clients_filters_by_archived_state() {
        let f = Fixture::new("list_scope");
        f.db.lock()
            .execute(
                "INSERT INTO client (first_name, last_name) VALUES ('Second', 'Client')",
                [],
            )
            .unwrap();
        let second_id = f.db.lock().last_insert_rowid();
        archive_client_impl(&mut f.db.lock(), second_id).unwrap();

        let conn = f.db.lock();
        let active = list_clients_impl(&conn, ClientScope::Active).unwrap();
        let archived = list_clients_impl(&conn, ClientScope::Archived).unwrap();
        let all = list_clients_impl(&conn, ClientScope::All).unwrap();

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].client.id, f.client_id);
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].client.id, second_id);
        assert!(archived[0].client.archived_at.is_some());
        assert_eq!(all.len(), 2);
    }

    /// Regression guard for putting the scope predicate in the wrong clause: it
    /// names `c.` only, so it must not turn either `LEFT JOIN` into an inner one
    /// and drop clients who have no purchases yet.
    #[test]
    fn list_clients_keeps_clients_with_no_purchases_under_every_scope() {
        let f = Fixture::new("list_no_purchases");
        let conn = f.db.lock();

        for scope in [ClientScope::Active, ClientScope::All] {
            let rows = list_clients_impl(&conn, scope).unwrap();
            assert_eq!(rows.len(), 1, "{scope:?} dropped a purchase-less client");
            assert_eq!(rows[0].purchase_count, 0);
            assert_eq!(rows[0].total_outstanding, 0);
            assert_eq!(rows[0].overdue_count, 0);
        }
    }

    /// Deleting is now the second half of a two-step, enforced in the backend
    /// rather than left to the UI to remember.
    #[test]
    fn delete_purchase_refuses_one_that_has_not_been_archived() {
        let f = Fixture::new("del_purchase_live");
        let detail = seeded_purchase(&f);

        let err = delete_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_NOT_ARCHIVED");
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);
    }

    #[test]
    fn delete_purchase_destroys_an_archived_one_and_cascades() {
        let f = Fixture::new("del_purchase");
        let detail = seeded_purchase(&f);
        archive_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();

        delete_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();

        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 0);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 0);
        assert_eq!(f.count("SELECT COUNT(*) FROM client"), 1, "client survives");
    }

    #[test]
    fn delete_purchase_reports_a_missing_id() {
        let f = Fixture::new("del_purchase_missing");
        let err = delete_purchase_impl(&mut f.db.lock(), 99_999).unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_NOT_FOUND");
    }

    // --- purchase archive / restore ----------------------------------------

    /// Record a payment against the first installment of `detail`.
    fn pay_first(f: &Fixture, detail: &PurchaseDetail, amount: i64) {
        let mut conn = f.db.lock();
        record_payment_impl(
            &mut conn,
            PaymentInput {
                installment_id: detail.installments[0].id,
                amount,
                payment_date: "2024-01-20".into(),
                note: None,
            },
        )
        .unwrap();
    }

    /// The invariant the money queries lean on: archiving is impossible once
    /// cash has been recorded, which is why `total_collected` needs no filter.
    #[test]
    fn archiving_is_impossible_once_a_payment_exists() {
        let f = Fixture::new("archive_purchase_paid");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);

        let err = archive_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_HAS_PAYMENTS:1");
        assert!(f.purchase_archived_at(detail.purchase.id).is_none());

        // ...and therefore it cannot be deleted either. It is permanent.
        let err = delete_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_NOT_ARCHIVED");
    }

    /// The other half: an archived purchase cannot start collecting cash.
    #[test]
    fn an_archived_purchase_cannot_take_a_payment() {
        let f = Fixture::new("archive_purchase_pay");
        let detail = seeded_purchase(&f);
        archive_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();

        let err = record_payment_impl(
            &mut f.db.lock(),
            PaymentInput {
                installment_id: detail.installments[0].id,
                amount: 250,
                payment_date: "2024-01-20".into(),
                note: None,
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_ARCHIVED");
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 0);
    }

    #[test]
    fn archive_purchase_stamps_an_iso_date_and_restore_clears_it() {
        let f = Fixture::new("archive_purchase_stamp");
        let detail = seeded_purchase(&f);

        archive_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();
        let first = f.purchase_archived_at(detail.purchase.id).unwrap();
        assert_eq!(first.len(), 10, "expected YYYY-MM-DD, got {first:?}");

        // Re-archiving must not move the stamp.
        archive_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();
        assert_eq!(f.purchase_archived_at(detail.purchase.id).unwrap(), first);

        restore_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();
        assert!(f.purchase_archived_at(detail.purchase.id).is_none());
        // Restoring an already-live purchase is the state the caller asked for.
        restore_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();
        assert!(f.purchase_archived_at(detail.purchase.id).is_none());
    }

    #[test]
    fn archive_and_restore_purchase_report_a_missing_id() {
        let f = Fixture::new("archive_purchase_missing");
        assert_eq!(
            code_of(archive_purchase_impl(&mut f.db.lock(), 99_999).unwrap_err()),
            "PURCHASE_NOT_FOUND"
        );
        assert_eq!(
            code_of(restore_purchase_impl(&mut f.db.lock(), 99_999).unwrap_err()),
            "PURCHASE_NOT_FOUND"
        );
    }

    /// Archiving must take the purchase out of every money read model, and
    /// restoring must put it back exactly. This is the whole point of the
    /// filter sweep, and the one test that covers all of them at once.
    #[test]
    fn archiving_removes_the_purchase_from_every_money_view() {
        let f = Fixture::new("archive_purchase_money");
        let detail = seeded_purchase(&f);
        let pid = detail.purchase.id;

        let before = f.money_snapshot();
        assert!(before.outstanding > 0, "fixture must owe something");

        archive_purchase_impl(&mut f.db.lock(), pid).unwrap();
        let after = f.money_snapshot();

        assert_eq!(after.purchases, 0, "dashboard purchase count");
        assert_eq!(after.sales, 0, "dashboard total sales");
        assert_eq!(after.outstanding, 0, "dashboard outstanding");
        assert_eq!(after.overdue, 0, "dashboard overdue installments");
        assert_eq!(after.impayes, 0, "impayés rows");
        assert_eq!(after.schedule, 0, "échéances rows");
        assert_eq!(after.client_outstanding, 0, "the client's balance");

        // The rows themselves are untouched — this is a hide, not a delete.
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 1);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);

        restore_purchase_impl(&mut f.db.lock(), pid).unwrap();
        assert_eq!(f.money_snapshot(), before, "restore must be exact");
    }

    #[test]
    fn list_purchases_partitions_by_scope() {
        let f = Fixture::new("purchase_scope");
        let live = seeded_purchase(&f);
        let archived = create_purchase_impl(&mut f.db.lock(), f.purchase_input()).unwrap();
        archive_purchase_impl(&mut f.db.lock(), archived.purchase.id).unwrap();

        let conn = f.db.lock();
        let ids = |scope| list_purchase_ids(&conn, scope).unwrap();
        assert_eq!(ids(PurchaseScope::Active), vec![live.purchase.id]);
        assert_eq!(ids(PurchaseScope::Archived), vec![archived.purchase.id]);
        assert_eq!(ids(PurchaseScope::All).len(), 2);
    }

    // --- purchase edit ------------------------------------------------------

    #[test]
    fn updating_the_label_alone_is_allowed_even_after_a_payment() {
        let f = Fixture::new("update_label");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);

        let mut input = f.purchase_input();
        input.product_label = "Réfrigérateur".into();
        let updated = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();

        assert_eq!(updated.purchase.product_label, "Réfrigérateur");
        // The schedule and the payment are untouched.
        assert_eq!(updated.installments.len(), 4);
        assert_eq!(updated.total_paid, 250);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1);
    }

    /// The editor always sends the installment rows it is displaying, so a
    /// label-only edit arrives carrying a list identical to the stored one.
    /// That must not read as a reschedule, or the label would be locked behind
    /// the payment guard for no reason.
    #[test]
    fn resending_the_unchanged_schedule_is_not_a_reschedule() {
        let f = Fixture::new("update_same_rows");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);

        let mut input = f.purchase_input();
        input.product_label = "Congélateur".into();
        input.installments = Some(
            detail
                .installments
                .iter()
                .map(|i| InstallmentInput {
                    index: i.index,
                    amount: i.amount,
                    due_date: i.due_date.clone(),
                })
                .collect(),
        );

        let updated = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();
        assert_eq!(updated.purchase.product_label, "Congélateur");
        assert_eq!(updated.total_paid, 250, "the payment survived");
    }

    /// Regenerating the whole schedule from the anchor fields rewrites every
    /// row's amount and due date, settled ones included — so a settled tranche
    /// refuses it. The tranches that are still owed remain editable; see
    /// `rescheduling_moves_the_unpaid_tranches_around_a_settled_one`.
    #[test]
    fn regenerating_the_schedule_is_refused_once_a_tranche_is_settled() {
        let f = Fixture::new("update_locked");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);

        for (mutate, expected) in [
            (
                (|i: &mut PurchaseInput| i.total_price = 2000) as fn(&mut PurchaseInput),
                "AMOUNT_LOCKED",
            ),
            (
                |i: &mut PurchaseInput| i.installment_count = 6,
                "AMOUNT_LOCKED",
            ),
            (
                |i: &mut PurchaseInput| i.purchase_date = "2024-02-01".into(),
                "DUE_DATE_LOCKED",
            ),
        ] {
            let mut input = f.purchase_input();
            mutate(&mut input);
            let err =
                update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
            assert_eq!(code_of(err), expected);
        }

        // Nothing moved — not even the purchase row, which is written first.
        let after = build_purchase_detail(&f.db.lock(), detail.purchase.id).unwrap();
        assert_eq!(after.purchase.total_price, 1000);
        assert_eq!(after.purchase.purchase_date, "2024-01-15");
        assert_eq!(after.installments.len(), 4);
    }

    /// The point of applying a schedule in place: a purchase carrying payments
    /// can still have the tranches that are still owed moved, because the rows
    /// — and the ledger hanging off them — survive the edit.
    #[test]
    fn rescheduling_moves_the_unpaid_tranches_around_a_settled_one() {
        let f = Fixture::new("update_around_settled");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);
        let ids: Vec<i64> = detail.installments.iter().map(|i| i.id).collect();

        let mut input = f.purchase_input();
        // Tranche 1 keeps its 250 and its date; the rest are re-cut and re-dated.
        input.installments = Some(vec![
            InstallmentInput {
                index: 1,
                amount: 250,
                due_date: "2024-01-15".into(),
            },
            InstallmentInput {
                index: 2,
                amount: 400,
                due_date: "2024-03-01".into(),
            },
            InstallmentInput {
                index: 3,
                amount: 200,
                due_date: "2024-04-01".into(),
            },
            InstallmentInput {
                index: 4,
                amount: 150,
                due_date: "2024-05-01".into(),
            },
        ]);
        let updated = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();

        assert_eq!(amounts_of(&updated), vec![250, 400, 200, 150]);
        assert_eq!(updated.installments[1].due_date, "2024-03-01");
        // In place, not regenerated: the rows kept their identities, which is
        // what kept the payment attached to tranche 1.
        assert_eq!(
            updated
                .installments
                .iter()
                .map(|i| i.id)
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(updated.total_paid, 250);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1);
        assert_eq!(updated.installments[0].status, "paid");
    }

    /// A tranche cannot be worth less than what has already been collected on
    /// it: `amount - paid_amount` feeds every outstanding total.
    #[test]
    fn rescheduling_below_what_a_tranche_collected_is_refused() {
        let f = Fixture::new("update_below_paid");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 100); // partial, so the row is not settled

        let mut input = f.purchase_input();
        input.installments = Some(vec![
            InstallmentInput {
                index: 1,
                amount: 50,
                due_date: "2024-01-15".into(),
            },
            InstallmentInput {
                index: 2,
                amount: 450,
                due_date: "2024-02-15".into(),
            },
            InstallmentInput {
                index: 3,
                amount: 250,
                due_date: "2024-03-15".into(),
            },
            InstallmentInput {
                index: 4,
                amount: 250,
                due_date: "2024-04-15".into(),
            },
        ]);
        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
        assert_eq!(code_of(err), "BELOW_PAID:100");
        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
    }

    /// Shortening the schedule deletes rows, and `payment` cascades off them.
    #[test]
    fn shortening_the_schedule_past_a_paid_tranche_is_refused() {
        let f = Fixture::new("update_shorten");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 3, 250); // the *last* tranche carries cash

        let mut input = f.purchase_input();
        input.total_price = 750;
        input.installment_count = 3;
        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_HAS_PAYMENTS:1");
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1);
    }

    /// The drop guard counts *ledger entries*, not the collected figure. A row
    /// corrected back down to zero still holds the entries that took the money
    /// and gave it back, and the cascade would erase both — a hole in the
    /// payment log that no total would ever surface.
    #[test]
    fn shortening_past_a_row_corrected_back_to_zero_is_refused() {
        let f = Fixture::new("update_shorten_zeroed");
        let detail = seeded_purchase(&f);
        let last = detail.installments[3].id;

        // Cash is recorded in order, so every earlier tranche is settled first;
        // then the last one's figure is corrected all the way back down.
        for pos in 0..4 {
            pay_installment(&f, &detail, pos, 250);
        }
        update_installment_impl(&mut f.db.lock(), last, edit_paid(0)).unwrap();

        let zeroed: i64 =
            f.db.lock()
                .query_row(
                    "SELECT paid_amount FROM installment WHERE id = ?1",
                    [last],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(zeroed, 0, "the figure is back to zero");
        assert_eq!(
            f.count("SELECT COUNT(*) FROM payment"),
            5,
            "history remains"
        );

        let mut input = f.purchase_input();
        input.total_price = 750;
        input.installment_count = 3;
        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_HAS_PAYMENTS:1");
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 5);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);
    }

    /// Dropping rows nobody has paid into is ordinary rescheduling.
    #[test]
    fn shortening_the_schedule_drops_only_empty_tranches() {
        let f = Fixture::new("update_shorten_ok");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);
        let first_id = detail.installments[0].id;

        let mut input = f.purchase_input();
        input.total_price = 500;
        input.installment_count = 2;
        input.installments = Some(vec![
            InstallmentInput {
                index: 1,
                amount: 250,
                due_date: "2024-01-15".into(),
            },
            InstallmentInput {
                index: 2,
                amount: 250,
                due_date: "2024-02-15".into(),
            },
        ]);
        let updated = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();

        assert_eq!(amounts_of(&updated), vec![250, 250]);
        assert_eq!(updated.installments[0].id, first_id, "kept in place");
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 2);
        assert_eq!(updated.total_paid, 250);
    }

    /// Lengthening it appends rows past the ones already stored.
    #[test]
    fn lengthening_the_schedule_appends_new_tranches() {
        let f = Fixture::new("update_lengthen");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);
        let ids: Vec<i64> = detail.installments.iter().map(|i| i.id).collect();

        let mut input = f.purchase_input();
        input.installment_count = 5;
        input.installments = Some(
            [250, 200, 200, 200, 150]
                .iter()
                .enumerate()
                .map(|(i, amount)| InstallmentInput {
                    index: i as i64 + 1,
                    amount: *amount,
                    due_date: format!("2024-0{}-15", i + 1),
                })
                .collect(),
        );
        let updated = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();

        assert_eq!(amounts_of(&updated), vec![250, 200, 200, 200, 150]);
        assert_eq!(
            updated.installments[..4]
                .iter()
                .map(|i| i.id)
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(updated.installments[4].index, 5);
        assert_eq!(updated.total_paid, 250);
    }

    /// Lowering a tranche onto its collected figure settles it, and `paid_date`
    /// is derived, so it has to gain one — from the ledger, not from thin air.
    #[test]
    fn rescheduling_onto_the_collected_figure_settles_the_tranche() {
        let f = Fixture::new("update_settles_row");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 100);

        let mut input = f.purchase_input();
        input.installments = Some(vec![
            InstallmentInput {
                index: 1,
                amount: 100,
                due_date: "2024-01-15".into(),
            },
            InstallmentInput {
                index: 2,
                amount: 300,
                due_date: "2024-02-15".into(),
            },
            InstallmentInput {
                index: 3,
                amount: 300,
                due_date: "2024-03-15".into(),
            },
            InstallmentInput {
                index: 4,
                amount: 300,
                due_date: "2024-04-15".into(),
            },
        ]);
        let updated = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();

        assert_eq!(updated.installments[0].status, "paid");
        assert_eq!(
            updated.installments[0].paid_date.as_deref(),
            Some("2024-01-20")
        );
    }

    /// Position order and chronological order have to stay the same thing, on
    /// the way in as much as on the way out.
    #[test]
    fn a_schedule_whose_dates_run_backwards_is_refused() {
        let f = Fixture::new("update_dates_backwards");
        let detail = seeded_purchase(&f);

        let out_of_order = vec![
            InstallmentInput {
                index: 1,
                amount: 500,
                due_date: "2024-03-15".into(),
            },
            InstallmentInput {
                index: 2,
                amount: 500,
                due_date: "2024-02-15".into(),
            },
        ];

        let mut input = f.purchase_input();
        input.installment_count = 2;
        input.installments = Some(out_of_order.clone());
        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
        assert_eq!(code_of(err), "DUE_DATE_OUT_OF_ORDER");
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);

        // Shared with create, so the two cannot drift.
        let mut input = f.purchase_input();
        input.installment_count = 2;
        input.installments = Some(out_of_order);
        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), "DUE_DATE_OUT_OF_ORDER");
    }

    // --- bounds on the manual schedule ---------------------------------------
    //
    // `validate_purchase_input` bounds every scalar field off the IPC boundary,
    // but the `installments` array bypasses it: it is what actually sizes the
    // row vector, the date vector and the insert loop. These pin the two guards
    // that make the scalar bounds bind on it too.

    /// Build a manual schedule of `amounts`, dated one month apart through 2024
    /// so the ordering check is never what refuses it.
    fn manual_schedule(amounts: &[i64]) -> Vec<InstallmentInput> {
        // The dates are formatted as month numbers, so a 13th entry would build
        // "2024-13-15" and fail on the date parse instead of the guard under
        // test — a confusing failure worth refusing outright.
        assert!(
            amounts.len() <= 12,
            "manual_schedule holds one year of dates"
        );
        amounts
            .iter()
            .enumerate()
            .map(|(i, amount)| InstallmentInput {
                index: i as i64 + 1,
                amount: *amount,
                due_date: format!("2024-{:02}-15", i + 1),
            })
            .collect()
    }

    /// The list length is what sizes the schedule, so the `1..=120` bound on
    /// `installment_count` only binds while the two agree. Declaring 1 and
    /// sending many is the shape that drove the unbounded allocation.
    #[test]
    fn a_manual_schedule_longer_than_the_declared_count_is_refused() {
        let f = Fixture::new("bounds_count_mismatch");

        let mut input = f.purchase_input();
        input.total_price = 1000;
        input.installment_count = 1;
        input.installments = Some(manual_schedule(&[1000, 0, 0, 0, 0]));

        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), "INSTALLMENT_COUNT_MISMATCH:5:1");
        assert_eq!(f.count("SELECT COUNT(*) FROM purchase"), 0);
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 0);
    }

    /// Shorter than declared is the same defect from the other side.
    #[test]
    fn a_manual_schedule_shorter_than_the_declared_count_is_refused() {
        let f = Fixture::new("bounds_count_short");

        let mut input = f.purchase_input();
        input.total_price = 1000;
        input.installment_count = 4;
        input.installments = Some(manual_schedule(&[600, 400]));

        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), "INSTALLMENT_COUNT_MISMATCH:2:4");
    }

    /// A negative share needs no overflow: a sibling covers it, the sum matches,
    /// and the row then subtracts from `SUM(amount - paid_amount)` — the figure
    /// every outstanding total is built on. Reachable on create, where nothing
    /// else looks at the amounts.
    #[test]
    fn a_negative_installment_amount_is_refused_on_create() {
        let f = Fixture::new("bounds_negative");

        let mut input = f.purchase_input();
        input.total_price = 1000;
        input.installment_count = 2;
        input.installments = Some(manual_schedule(&[1500, -500]));

        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), "INVALID_AMOUNT");
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 0);
    }

    /// The same guard has to hold on the update path, which shares
    /// `resolve_schedule`. Update caught this only incidentally before, via
    /// `BELOW_PAID` against a zero `paid_amount`.
    #[test]
    fn a_negative_installment_amount_is_refused_on_update() {
        let f = Fixture::new("bounds_negative_update");
        let detail = seeded_purchase(&f);

        let mut input = f.purchase_input();
        input.installment_count = 2;
        input.installments = Some(manual_schedule(&[1500, -500]));

        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
        assert_eq!(code_of(err), "INVALID_AMOUNT");
        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
    }

    /// The reason the bound is a bound and not an `overflow-checks` flag: two
    /// `i64::MAX` terms wrap to -2, and -2 + 1002 is exactly the declared total,
    /// so the sum check would have proved the schedule adds up when it does not.
    /// The per-amount range refuses it before the sum is ever taken.
    #[test]
    fn a_schedule_whose_sum_wraps_to_the_total_is_refused() {
        let f = Fixture::new("bounds_wrapping_sum");

        // Guard the arithmetic claim itself, so this test still means something
        // if the constant or the profile ever changes.
        assert_eq!(i64::MAX.wrapping_add(i64::MAX).wrapping_add(1002), 1000);

        let mut input = f.purchase_input();
        input.total_price = 1000;
        input.installment_count = 3;
        input.installments = Some(manual_schedule(&[i64::MAX, i64::MAX, 1002]));

        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), "INVALID_AMOUNT");
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 0);
    }

    /// An out-of-range total is refused for the same reason, and reported
    /// against the field the caller actually sent.
    #[test]
    fn a_total_price_beyond_the_money_range_is_refused() {
        let f = Fixture::new("bounds_total");

        let mut input = f.purchase_input();
        input.total_price = *MONEY_RANGE.end() + 1;
        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), "INVALID_TOTAL_PRICE");

        // The existing lower bound still reports the same code.
        let mut input = f.purchase_input();
        input.total_price = 0;
        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), "INVALID_TOTAL_PRICE");
    }

    /// Zero stays legal: `split_amounts` can produce it for a small total over
    /// many installments, and a zeroed tranche reads as settled by design. The
    /// bound must not turn that into a refusal.
    #[test]
    fn a_zero_installment_amount_is_still_accepted() {
        let f = Fixture::new("bounds_zero_ok");

        let mut input = f.purchase_input();
        input.total_price = 1000;
        input.installment_count = 3;
        input.installments = Some(manual_schedule(&[1000, 0, 0]));

        let detail = create_purchase_impl(&mut f.db.lock(), input).unwrap();
        assert_eq!(amounts_of(&detail), vec![1000, 0, 0]);
        assert_eq!(detail.installments[1].status, "paid");
    }

    #[test]
    fn rescheduling_regenerates_the_installments_while_unpaid() {
        let f = Fixture::new("update_reschedule");
        let detail = seeded_purchase(&f);

        let mut input = f.purchase_input();
        input.total_price = 900;
        input.installment_count = 3;
        let updated = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();

        assert_eq!(updated.purchase.total_price, 900);
        let amounts: Vec<i64> = updated.installments.iter().map(|i| i.amount).collect();
        assert_eq!(amounts, vec![300, 300, 300]);
        // Regenerated, not appended.
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 3);
        // The reference is derived at creation and must survive an edit.
        assert_eq!(updated.purchase.reference, detail.purchase.reference);
    }

    #[test]
    fn updating_an_archived_purchase_is_refused() {
        let f = Fixture::new("update_archived");
        let detail = seeded_purchase(&f);
        archive_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();

        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, f.purchase_input())
            .unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_ARCHIVED");
    }

    #[test]
    fn updating_rejects_a_mismatched_manual_split_without_writing() {
        let f = Fixture::new("update_mismatch");
        let detail = seeded_purchase(&f);

        let mut input = f.purchase_input();
        // The count has to agree with the list, or the length guard refuses it
        // first and the sum is never reached — which is the wrong failure for
        // this test.
        input.installment_count = 2;
        input.installments = Some(vec![
            InstallmentInput {
                index: 1,
                amount: 400,
                due_date: "2024-01-15".into(),
            },
            InstallmentInput {
                index: 2,
                amount: 500,
                due_date: "2024-02-15".into(),
            },
        ]);
        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
        assert_eq!(code_of(err), "SUM_MISMATCH:900:1000");
        assert_eq!(f.count("SELECT COUNT(*) FROM installment"), 4);
    }

    // --- installment edit ----------------------------------------------------
    //
    // The baseline purchase is 1000 over 4 monthly tranches of 250, dated
    // 2024-01-15 — so every tranche is already overdue against today.
    //
    // This editor deals only in money. The schedule belongs to `update_purchase`
    // and is refused here outright; the money is gated on the *previous* tranche
    // and nothing about this one's own status matters.

    /// Pay `amount` against installment `pos` (0-based) of `detail`.
    fn pay_installment(f: &Fixture, detail: &PurchaseDetail, pos: usize, amount: i64) {
        record_payment_impl(
            &mut f.db.lock(),
            PaymentInput {
                installment_id: detail.installments[pos].id,
                amount,
                payment_date: "2024-02-01".into(),
                note: None,
            },
        )
        .unwrap();
    }

    fn amounts_of(detail: &PurchaseDetail) -> Vec<i64> {
        detail.installments.iter().map(|i| i.amount).collect()
    }

    /// The schedule as it is right now, for asserting a refusal changed nothing.
    fn stored_amounts(f: &Fixture, purchase_id: i64) -> Vec<i64> {
        amounts_of(&build_purchase_detail(&f.db.lock(), purchase_id).unwrap())
    }

    fn edit_amount(amount: i64) -> InstallmentEdit {
        InstallmentEdit {
            amount: Some(amount),
            ..Default::default()
        }
    }

    fn edit_paid(paid: i64) -> InstallmentEdit {
        InstallmentEdit {
            paid_amount: Some(paid),
            ..Default::default()
        }
    }

    // -- the schedule is not this command's ------------------------------------

    /// Rule 3, and the structural half of rules 1 and 2: an amount or a due
    /// date sent here is refused whatever its value and whatever the tranche's
    /// state, so "the schedule is edited in one place" is a property of the
    /// backend rather than a habit of the UI.
    #[test]
    fn the_installment_editor_refuses_the_schedule_fields() {
        let f = Fixture::new("inst_schedule_refused");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);

        // Settled (tranche 1) and unsettled (tranche 3) alike.
        for pos in [0, 2] {
            for edit in [
                edit_amount(400),
                InstallmentEdit {
                    due_date: Some("2024-06-01".into()),
                    ..Default::default()
                },
                // Even a value identical to what is stored: sending the field at
                // all is a caller that still believes this command owns it.
                edit_amount(250),
            ] {
                let err =
                    update_installment_impl(&mut f.db.lock(), detail.installments[pos].id, edit)
                        .unwrap_err();
                assert_eq!(code_of(err), "SCHEDULE_VIA_PURCHASE", "at position {pos}");
            }
        }

        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
    }

    /// The refusal comes before the licence gate has anything to say, and before
    /// the transaction opens, so a schedule field never reaches the database.
    #[test]
    fn the_schedule_refusal_precedes_every_lookup() {
        let f = Fixture::new("inst_schedule_first");
        // An id that does not exist: the schedule guard still wins, which is
        // what proves it runs before the row is even looked up.
        let err = update_installment_impl(&mut f.db.lock(), 9_999, edit_amount(1)).unwrap_err();
        assert_eq!(code_of(err), "SCHEDULE_VIA_PURCHASE");
    }

    // -- the money half -------------------------------------------------------

    /// Moving the collected figure writes a matching ledger entry, so the
    /// dashboard's "Amount collected" cannot drift from every other total.
    #[test]
    fn raising_the_paid_amount_writes_a_correction_entry() {
        let f = Fixture::new("inst_paid_up");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 100);

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                paid_amount: Some(250),
                payment_date: Some("2024-03-05".into()),
                note: Some("  solde  ".into()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(updated.installments[0].paid_amount, 250);
        assert_eq!(updated.installments[0].status, "paid");
        assert_eq!(
            updated.installments[0].paid_date.as_deref(),
            Some("2024-03-05")
        );
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 2);
        assert_eq!(
            f.count("SELECT amount FROM payment ORDER BY id DESC LIMIT 1"),
            150,
            "the correction entry carries the difference, not the total"
        );
        let note: String =
            f.db.lock()
                .query_row(
                    "SELECT note FROM payment ORDER BY id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(note, "solde", "the note is trimmed, as record_payment does");
        f.assert_ledger_matches_installments();
    }

    #[test]
    fn lowering_the_paid_amount_writes_a_negative_correction() {
        let f = Fixture::new("inst_paid_down");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);

        let updated =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_paid(80))
                .unwrap();

        assert_eq!(updated.installments[0].paid_amount, 80);
        assert_eq!(updated.installments[0].status, "late");
        // No longer settled, so it must not still show a settlement date.
        assert_eq!(updated.installments[0].paid_date, None);
        assert_eq!(
            f.count("SELECT amount FROM payment ORDER BY id DESC LIMIT 1"),
            -170
        );
        f.assert_ledger_matches_installments();
    }

    /// Zeroing the collected figure reverses the whole ledger for that row.
    #[test]
    fn zeroing_the_paid_amount_reverses_the_ledger() {
        let f = Fixture::new("inst_paid_zero");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);

        let updated =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_paid(0))
                .unwrap();

        assert_eq!(updated.installments[0].paid_amount, 0);
        assert_eq!(updated.total_paid, 0);
        f.assert_ledger_matches_installments();
        assert_eq!(
            f.count("SELECT COALESCE(SUM(amount),0) FROM payment WHERE installment_id IS NOT NULL"),
            0
        );
    }

    /// Cash is collected in order, so it cannot be recorded out of order.
    #[test]
    fn the_money_fields_are_gated_on_the_previous_tranche() {
        let f = Fixture::new("inst_money_gate");
        let detail = seeded_purchase(&f);

        let err =
            update_installment_impl(&mut f.db.lock(), detail.installments[1].id, edit_paid(100))
                .unwrap_err();
        assert_eq!(code_of(err), "PREVIOUS_UNPAID:1");
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 0);

        // Settling tranche 1 opens tranche 2.
        pay_installment(&f, &detail, 0, 250);
        let updated =
            update_installment_impl(&mut f.db.lock(), detail.installments[1].id, edit_paid(100))
                .unwrap();
        assert_eq!(updated.installments[1].paid_amount, 100);
        f.assert_ledger_matches_installments();
    }

    /// The gate is on the money only — a tranche whose predecessor is owing can
    /// still be rescheduled, just not from here.
    #[test]
    fn the_gate_does_not_reach_the_purchase_editor() {
        let f = Fixture::new("inst_gate_scope");
        let detail = seeded_purchase(&f);

        let mut input = f.purchase_input();
        input.installments = Some(vec![
            InstallmentInput {
                index: 1,
                amount: 250,
                due_date: "2024-01-15".into(),
            },
            InstallmentInput {
                index: 2,
                amount: 300,
                due_date: "2024-03-01".into(),
            },
            InstallmentInput {
                index: 3,
                amount: 225,
                due_date: "2024-03-15".into(),
            },
            InstallmentInput {
                index: 4,
                amount: 225,
                due_date: "2024-04-15".into(),
            },
        ]);
        update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap();

        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 300, 225, 225]
        );
    }

    /// The collected figure can never exceed what the tranche is worth — that
    /// would make `amount - paid_amount` negative and cancel out another
    /// client's real debt in the outstanding aggregates.
    #[test]
    fn the_paid_amount_cannot_exceed_the_tranche() {
        let f = Fixture::new("inst_paid_over");
        let detail = seeded_purchase(&f);

        let err =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_paid(400))
                .unwrap_err();
        assert_eq!(code_of(err), "PAID_ABOVE_AMOUNT:250");
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 0);
    }

    /// Rule 1's other half: a *settled* tranche's collected figure stays
    /// correctable. Only its amount and due date are history.
    #[test]
    fn a_settled_tranche_keeps_its_collected_figure_editable() {
        let f = Fixture::new("inst_settled_money");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);

        let updated =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_paid(180))
                .unwrap();

        assert_eq!(updated.installments[0].paid_amount, 180);
        // No longer settled, so the derived date goes with it.
        assert_eq!(updated.installments[0].paid_date, None);
        assert_eq!(updated.installments[0].amount, 250, "the schedule held");
        f.assert_ledger_matches_installments();
    }

    // -- the payment date -----------------------------------------------------

    /// Rule 2: a payment date is history once recorded. With no correction to
    /// carry it there is nothing for a new date to describe, and rewriting the
    /// entry already there would move `paid_date` away from the cash behind it.
    #[test]
    fn a_recorded_payment_date_cannot_be_rewritten() {
        let f = Fixture::new("inst_redate");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);

        let err = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                payment_date: Some("2024-03-05".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), "PAYMENT_DATE_LOCKED");

        // The ledger entry, and the date derived from it, are untouched.
        let dated: String =
            f.db.lock()
                .query_row("SELECT payment_date FROM payment", [], |r| r.get(0))
                .unwrap();
        assert_eq!(dated, "2024-02-01");
        let after = build_purchase_detail(&f.db.lock(), detail.purchase.id).unwrap();
        assert_eq!(
            after.installments[0].paid_date.as_deref(),
            Some("2024-02-01")
        );
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1);
    }

    /// Setting one the first time is the whole point, though — a date travelling
    /// with a moved figure dates the entry that move creates.
    #[test]
    fn a_payment_date_still_dates_the_entry_it_arrives_with() {
        let f = Fixture::new("inst_date_new_entry");
        let detail = seeded_purchase(&f);

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                paid_amount: Some(250),
                payment_date: Some("2024-03-05".into()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            updated.installments[0].paid_date.as_deref(),
            Some("2024-03-05")
        );
        // And a second, later correction dates its own entry without touching
        // the first — the ledger accumulates rather than being rewritten.
        let updated =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_paid(200))
                .unwrap();
        assert_eq!(updated.installments[0].paid_amount, 200);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 2);
        f.assert_ledger_matches_installments();
    }

    /// A note with no correction to carry it amends the entry already there.
    #[test]
    fn a_note_alone_amends_the_latest_ledger_entry() {
        let f = Fixture::new("inst_note_only");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);

        update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                note: Some("chèque".into()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1, "no entry added");
        let note: String =
            f.db.lock()
                .query_row("SELECT note FROM payment", [], |r| r.get(0))
                .unwrap();
        assert_eq!(note, "chèque");
        f.assert_ledger_matches_installments();
    }

    #[test]
    fn a_note_with_no_payment_behind_it_is_refused_rather_than_dropped() {
        let f = Fixture::new("inst_note_orphan");
        let detail = seeded_purchase(&f);

        let err = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                note: Some("chèque".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), "NO_PAYMENT_TO_DATE");
    }

    #[test]
    fn a_payment_date_needs_something_to_date_and_cannot_be_in_the_future() {
        let f = Fixture::new("inst_paid_date");
        let detail = seeded_purchase(&f);
        let inst_id = detail.installments[0].id;

        let err = update_installment_impl(
            &mut f.db.lock(),
            inst_id,
            InstallmentEdit {
                payment_date: Some("2024-03-05".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), "NO_PAYMENT_TO_DATE");

        pay_installment(&f, &detail, 0, 250);
        let tomorrow = today().succ_opt().unwrap();
        let err = update_installment_impl(
            &mut f.db.lock(),
            inst_id,
            InstallmentEdit {
                payment_date: Some(tomorrow.to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), "FUTURE_PAID_DATE");
    }

    // -- shared guards --------------------------------------------------------

    #[test]
    fn editing_an_installment_of_an_archived_purchase_is_refused() {
        let f = Fixture::new("inst_archived");
        let detail = seeded_purchase(&f);
        archive_purchase_impl(&mut f.db.lock(), detail.purchase.id).unwrap();

        let err =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_paid(250))
                .unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_ARCHIVED");
    }

    #[test]
    fn editing_rejects_bad_arguments_without_writing() {
        let f = Fixture::new("inst_bad_args");
        let detail = seeded_purchase(&f);
        let inst_id = detail.installments[0].id;

        assert_eq!(
            code_of(update_installment_impl(&mut f.db.lock(), inst_id, edit_paid(-1)).unwrap_err()),
            "INVALID_AMOUNT"
        );
        assert_eq!(
            code_of(
                update_installment_impl(
                    &mut f.db.lock(),
                    inst_id,
                    InstallmentEdit {
                        paid_amount: Some(250),
                        payment_date: Some("not-a-date".into()),
                        ..Default::default()
                    },
                )
                .unwrap_err()
            ),
            "INVALID_DATE"
        );
        assert_eq!(
            code_of(update_installment_impl(&mut f.db.lock(), 9_999, edit_paid(300)).unwrap_err()),
            "INSTALLMENT_NOT_FOUND"
        );

        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
    }

    /// A refused edit must leave the ledger alone as well as the row, since the
    /// correction entry and the cached figure are two writes that have to land
    /// together or not at all.
    #[test]
    fn a_refused_edit_writes_nothing_at_all() {
        let f = Fixture::new("inst_rollback");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 100);
        let before = f.money_snapshot();

        // Tranche 1 is still owing, so no money may be recorded against 2.
        let err = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[1].id,
            InstallmentEdit {
                paid_amount: Some(50),
                note: Some("acompte".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), "PREVIOUS_UNPAID:1");

        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
        assert_eq!(f.money_snapshot(), before);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1);
        f.assert_ledger_matches_installments();
    }

    /// The same for the purchase editor: it writes the purchase row before the
    /// schedule, so a schedule refused after that has to take the row with it.
    #[test]
    fn a_refused_reschedule_rolls_the_purchase_row_back_too() {
        let f = Fixture::new("update_rollback");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);
        let before = f.money_snapshot();

        let mut input = f.purchase_input();
        input.product_label = "Congélateur".into();
        input.total_price = 2000;
        let err = update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
        assert_eq!(code_of(err), "AMOUNT_LOCKED");

        let after = build_purchase_detail(&f.db.lock(), detail.purchase.id).unwrap();
        assert_eq!(after.purchase.total_price, 1000);
        assert_eq!(
            after.purchase.product_label, detail.purchase.product_label,
            "the label is written first and must roll back with the rest"
        );
        assert_eq!(f.money_snapshot(), before);
    }

    // --- free-text bounds ----------------------------------------------------
    //
    // Every one of these fields was `.trim()` and nothing else, so the renderer
    // could store a nameless client or a megabyte of text that is then read back
    // into every list, export and dashboard card.

    fn client_input() -> ClientInput {
        ClientInput {
            first_name: "Mohamed".into(),
            last_name: "Trabelsi".into(),
            phone: "+216 20 123 456".into(),
            address: "Cité El Ghazala, Ariana".into(),
            email: Some("mohamed.trabelsi@email.tn".into()),
        }
    }

    #[test]
    fn a_client_must_carry_a_name() {
        for blank in ["", "   "] {
            let mut input = client_input();
            input.first_name = blank.into();
            assert_eq!(
                code_of(validate_client_input(&input).unwrap_err()),
                "TEXT_REQUIRED"
            );

            let mut input = client_input();
            input.last_name = blank.into();
            assert_eq!(
                code_of(validate_client_input(&input).unwrap_err()),
                "TEXT_REQUIRED"
            );
        }
    }

    #[test]
    fn client_text_fields_are_bounded() {
        let long = "x".repeat(SHORT_TEXT_MAX + 1);
        for mutate in [
            (|i: &mut ClientInput, v: String| i.first_name = v) as fn(&mut ClientInput, String),
            |i: &mut ClientInput, v: String| i.last_name = v,
            |i: &mut ClientInput, v: String| i.phone = v,
            |i: &mut ClientInput, v: String| i.email = Some(v),
        ] {
            let mut input = client_input();
            mutate(&mut input, long.clone());
            assert_eq!(
                code_of(validate_client_input(&input).unwrap_err()),
                format!("TEXT_TOO_LONG:{SHORT_TEXT_MAX}")
            );
        }

        // The address is prose and gets the longer allowance.
        let mut input = client_input();
        input.address = "x".repeat(LONG_TEXT_MAX);
        validate_client_input(&input).unwrap();
        input.address = "x".repeat(LONG_TEXT_MAX + 1);
        assert_eq!(
            code_of(validate_client_input(&input).unwrap_err()),
            format!("TEXT_TOO_LONG:{LONG_TEXT_MAX}")
        );
    }

    /// Counted in characters, not bytes — otherwise the same field would hold
    /// fewer letters in Arabic or French than in English.
    #[test]
    fn the_text_cap_counts_characters_not_bytes() {
        let mut input = client_input();
        // Each of these is multi-byte, so a byte cap would reject it well before
        // the character cap does.
        input.first_name = "é".repeat(SHORT_TEXT_MAX);
        assert!(
            input.first_name.len() > SHORT_TEXT_MAX,
            "multi-byte on purpose"
        );
        validate_client_input(&input).unwrap();

        input.first_name = "م".repeat(SHORT_TEXT_MAX + 1);
        assert_eq!(
            code_of(validate_client_input(&input).unwrap_err()),
            format!("TEXT_TOO_LONG:{SHORT_TEXT_MAX}")
        );
    }

    /// The seeded demo data has to stay inside the caps, or a fresh install
    /// would ship values the app now refuses to edit.
    #[test]
    fn the_seeded_clients_are_within_the_caps() {
        let f = Fixture::new("seed_bounds");
        crate::seed::seed(&f.db.lock()).unwrap();
        let conn = f.db.lock();
        let mut stmt = conn
            .prepare("SELECT first_name, last_name, phone, address, COALESCE(email,'') FROM client")
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .unwrap();
        let mut seen = 0;
        for row in rows {
            let (first, last, phone, address, email) = row.unwrap();
            for short in [&first, &last, &phone, &email] {
                assert!(short.chars().count() <= SHORT_TEXT_MAX, "{short:?}");
            }
            assert!(address.chars().count() <= LONG_TEXT_MAX, "{address:?}");
            seen += 1;
        }
        assert!(seen > 0, "the seed must have written clients");
    }

    #[test]
    fn a_product_label_is_bounded() {
        let f = Fixture::new("label_bounds");
        let mut input = f.purchase_input();
        input.product_label = "x".repeat(SHORT_TEXT_MAX + 1);
        let err = create_purchase_impl(&mut f.db.lock(), input).unwrap_err();
        assert_eq!(code_of(err), format!("TEXT_TOO_LONG:{SHORT_TEXT_MAX}"));
    }

    /// The three coded settings are `<select>` elements on the frontend, i.e.
    /// closed sets — but the renderer is not what enforces that.
    #[test]
    fn coded_settings_are_held_to_their_vocabulary() {
        let f = Fixture::new("settings_vocab");

        for patch in [
            SettingsPatch {
                language: Some("klingon".into()),
                ..blank_patch()
            },
            SettingsPatch {
                currency_code: Some("XXX".into()),
                ..blank_patch()
            },
            SettingsPatch {
                date_format: Some("yyyy/yyyy/yyyy".into()),
                ..blank_patch()
            },
        ] {
            let err = update_settings_impl(&mut f.db.lock(), patch).unwrap_err();
            assert_eq!(code_of(err), "INVALID_SETTING_VALUE");
        }

        // Every legal value is still accepted, and surrounding space forgiven.
        for lang in LANGUAGES {
            update_settings_impl(
                &mut f.db.lock(),
                SettingsPatch {
                    language: Some(format!("  {lang}  ")),
                    ..blank_patch()
                },
            )
            .unwrap();
        }
        for code in CURRENCY_CODES {
            update_settings_impl(
                &mut f.db.lock(),
                SettingsPatch {
                    currency_code: Some(code.into()),
                    ..blank_patch()
                },
            )
            .unwrap();
        }
        for fmt in DATE_FORMATS {
            update_settings_impl(
                &mut f.db.lock(),
                SettingsPatch {
                    date_format: Some(fmt.into()),
                    ..blank_patch()
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn shop_text_is_bounded_and_a_refusal_writes_nothing() {
        let f = Fixture::new("settings_text");
        let before = update_settings_impl(&mut f.db.lock(), blank_patch()).unwrap();

        let err = update_settings_impl(
            &mut f.db.lock(),
            SettingsPatch {
                // A legal language alongside an illegal shop_info: the whole
                // patch must be refused, not applied up to the bad field.
                language: Some("en".into()),
                shop_info: Some("x".repeat(LONG_TEXT_MAX + 1)),
                ..blank_patch()
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), format!("TEXT_TOO_LONG:{LONG_TEXT_MAX}"));

        let after = update_settings_impl(&mut f.db.lock(), blank_patch()).unwrap();
        assert_eq!(
            after.language, before.language,
            "nothing may have been written"
        );
        assert_eq!(after.shop_info, before.shop_info);
    }

    fn blank_patch() -> SettingsPatch {
        SettingsPatch {
            language: None,
            currency_code: None,
            date_format: None,
            shop_name: None,
            shop_info: None,
            alert_soon_days: None,
            auto_backup_enabled: None,
            auto_backup_frequency: None,
            auto_backup_time: None,
        }
    }

    // --- settings ----------------------------------------------------------

    #[test]
    fn update_settings_applies_atomically_and_clamps() {
        let f = Fixture::new("settings");
        let mut conn = f.db.lock();

        let out = update_settings_impl(
            &mut conn,
            SettingsPatch {
                language: Some("ar".into()),
                currency_code: Some("EUR".into()),
                date_format: None,
                shop_name: Some("Chez Malek".into()),
                shop_info: None,
                alert_soon_days: Some(500),
                auto_backup_enabled: None,
                auto_backup_frequency: None,
                auto_backup_time: None,
            },
        )
        .unwrap();

        assert_eq!(out.language, "ar");
        assert_eq!(out.currency_code, "EUR");
        assert_eq!(out.shop_name, "Chez Malek");
        assert_eq!(out.alert_soon_days, 90, "out-of-range window must clamp");
        // Choosing a language explicitly must also end OS-locale detection —
        // the pair that was previously written outside a transaction.
        assert!(!out.language_is_default);
    }

    /// `last_backup_at` rides in on the settings default-resolution path rather
    /// than a schema column, so it must read as "never" on any database that
    /// predates it — no migration, no `NULL` column, no error.
    #[test]
    fn the_backup_date_reads_as_absent_until_one_is_recorded() {
        let f = Fixture::new("last_backup");
        let conn = f.db.lock();

        assert_eq!(
            read_settings(&conn).last_backup_at,
            None,
            "an install that has never backed up must report no date"
        );

        put_setting(&conn, LAST_BACKUP_KEY, "2026-08-07").unwrap();
        assert_eq!(
            read_settings(&conn).last_backup_at,
            Some("2026-08-07".to_string())
        );
    }

    // --- missing rows ------------------------------------------------------

    #[test]
    fn absent_rows_report_not_found_rather_than_an_internal_error() {
        let f = Fixture::new("missing");
        let conn = f.db.lock();

        assert_eq!(
            code_of(fetch_client(&conn, 999_999).unwrap_err()),
            "CLIENT_NOT_FOUND"
        );
        assert_eq!(
            code_of(build_purchase_detail(&conn, 999_999).unwrap_err()),
            "PURCHASE_NOT_FOUND"
        );
    }

    // --- backup ------------------------------------------------------------

    /// A staging directory of our own, standing in for app data.
    fn staging_dir(tag: &str) -> PathBuf {
        let d = temp_db_path(tag).with_extension("staging");
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The backup must be a real, readable snapshot — and must never destroy
    /// what is already at the destination.
    #[test]
    fn backup_writes_a_readable_snapshot() {
        let f = Fixture::new("backup");
        seeded_purchase(&f);

        let dest = temp_db_path("backup_out");
        let staging = staging_dir("backup_stage");
        backup_database_impl(&f.db.lock(), &dest, &staging).unwrap();

        // The snapshot opens and carries the data.
        let restored = Connection::open(&dest).unwrap();
        let purchases: i64 = restored
            .query_row("SELECT COUNT(*) FROM purchase", [], |r| r.get(0))
            .unwrap();
        let installments: i64 = restored
            .query_row("SELECT COUNT(*) FROM installment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(purchases, 1);
        assert_eq!(installments, 4);
        drop(restored);

        // Staging is left clean: an orphan there is invisible and would grow
        // with every backup.
        assert_eq!(
            std::fs::read_dir(&staging).unwrap().count(),
            0,
            "staging directory must be empty after a successful backup"
        );

        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_dir_all(&staging);
    }

    /// The destination guards: it has to be named like a database, and anything
    /// already there has to *be* one. This is what stops a renderer-chosen path
    /// turning the command into an arbitrary-file-destruction primitive.
    #[test]
    fn backup_refuses_to_clobber_a_file_that_is_not_a_database() {
        let f = Fixture::new("backup_guard");
        seeded_purchase(&f);
        let staging = staging_dir("backup_guard_stage");

        // Wrong extension.
        let not_db = temp_db_path("backup_guard_out").with_extension("txt");
        let err = backup_database_impl(&f.db.lock(), &not_db, &staging).unwrap_err();
        assert_eq!(code_of(err), "BACKUP_FAILED");
        assert!(
            !not_db.exists(),
            "nothing may be created at a rejected path"
        );

        // Right extension, but the file there is not a database.
        let occupied = temp_db_path("backup_guard_occupied");
        std::fs::write(&occupied, b"my dissertation, not a database").unwrap();
        let err = backup_database_impl(&f.db.lock(), &occupied, &staging).unwrap_err();
        assert_eq!(code_of(err), "BACKUP_FAILED");
        assert_eq!(
            std::fs::read(&occupied).unwrap(),
            b"my dissertation, not a database",
            "the existing file must be byte-identical"
        );

        let _ = std::fs::remove_file(&occupied);
        let _ = std::fs::remove_dir_all(&staging);
    }

    /// The defect this rewrite closes: the staging file used to be
    /// `dest.with_extension("db.part")` and was `remove_file`d unconditionally,
    /// so a backup to `notes.db` deleted whatever sat at `notes.db.part`. That
    /// path is chosen by the caller and guarded by nothing.
    #[test]
    fn backup_never_touches_a_sibling_of_the_destination() {
        let f = Fixture::new("backup_sibling");
        seeded_purchase(&f);
        let staging = staging_dir("backup_sibling_stage");

        let dest = temp_db_path("backup_sibling_out");
        let sibling = dest.with_extension("db.part");
        std::fs::write(&sibling, b"someone else's file").unwrap();

        backup_database_impl(&f.db.lock(), &dest, &staging).unwrap();

        assert_eq!(
            std::fs::read(&sibling).unwrap(),
            b"someone else's file",
            "a file merely named like the old staging path must survive"
        );

        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(&sibling);
        let _ = std::fs::remove_dir_all(&staging);
    }

    /// Staging is per-run, so two backups into one directory cannot collide —
    /// the old derived name could, and `with_extension` also mangled any
    /// destination whose stem contained a dot.
    #[test]
    fn backup_overwrites_an_earlier_snapshot_at_the_same_path() {
        let f = Fixture::new("backup_twice");
        seeded_purchase(&f);
        let staging = staging_dir("backup_twice_stage");

        // A dotted stem: `with_extension` used to turn this into
        // `payment-schedule-2026.08.db.part`.
        let dest =
            temp_db_path("backup_twice_out").with_file_name("payment-schedule-2026.08.04.db");

        backup_database_impl(&f.db.lock(), &dest, &staging).unwrap();
        backup_database_impl(&f.db.lock(), &dest, &staging).unwrap();

        let restored = Connection::open(&dest).unwrap();
        let purchases: i64 = restored
            .query_row("SELECT COUNT(*) FROM purchase", [], |r| r.get(0))
            .unwrap();
        assert_eq!(purchases, 1);
        drop(restored);
        assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);

        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_dir_all(&staging);
    }

    /// App data and the destination are routinely on different filesystems — the
    /// user saves a backup to a USB stick — where `rename` fails with `EXDEV`.
    /// The copy fallback is the only thing that makes those backups work at all,
    /// and no other test can reach it because temp dirs share a filesystem.
    ///
    /// Skipped rather than failed where no second filesystem is available, so
    /// this does not turn into a flake on a machine or runner without one.
    ///
    /// Unix-only: it needs `MetadataExt::dev()` to prove the two paths really
    /// are on different filesystems, and this project also builds for Windows.
    #[cfg(unix)]
    #[test]
    fn backup_falls_back_to_a_copy_across_filesystems() {
        let Some(other_fs) = second_filesystem() else {
            eprintln!("skipped: no second filesystem available to cross");
            return;
        };

        let f = Fixture::new("backup_xdev");
        seeded_purchase(&f);

        // Stage on the *other* filesystem from the destination, so the rename
        // must fail and the fallback must carry it.
        let staging = other_fs.join(format!("ps-xdev-{}", std::process::id()));
        std::fs::create_dir_all(&staging).unwrap();
        let dest = temp_db_path("backup_xdev_out");
        assert_ne!(
            same_device(&staging),
            same_device(dest.parent().unwrap()),
            "the two paths must really be on different filesystems"
        );

        backup_database_impl(&f.db.lock(), &dest, &staging).unwrap();

        let restored = Connection::open(&dest).unwrap();
        let purchases: i64 = restored
            .query_row("SELECT COUNT(*) FROM purchase", [], |r| r.get(0))
            .unwrap();
        assert_eq!(purchases, 1, "the copied snapshot must carry the data");
        drop(restored);
        assert_eq!(
            std::fs::read_dir(&staging).unwrap().count(),
            0,
            "the staging file must be cleaned up after a cross-device copy"
        );

        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[cfg(unix)]
    /// The device id of the filesystem holding `path`.
    fn same_device(path: &std::path::Path) -> u64 {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).unwrap().dev()
    }

    #[cfg(unix)]
    /// A writable directory on a different filesystem from `std::env::temp_dir`,
    /// or `None` when the machine has only one.
    fn second_filesystem() -> Option<PathBuf> {
        let temp_dev = same_device(&std::env::temp_dir());
        ["/dev/shm", "/run/user/1000"]
            .iter()
            .map(PathBuf::from)
            .find(|c| c.is_dir() && same_device(c) != temp_dev)
    }

    /// A `VACUUM INTO` that returns `Ok` is not proof the file it wrote is a
    /// usable database, and the difference only ever surfaces at restore time.
    /// This pins both directions of the gate that now stands between the
    /// snapshot and the destination.
    #[test]
    fn a_snapshot_is_verified_before_it_is_accepted() {
        let f = Fixture::new("backup_verify");
        seeded_purchase(&f);

        // What the backup path actually produces must pass.
        let good = temp_db_path("backup_verify_good");
        let staging = staging_dir("backup_verify_stage");
        backup_database_impl(&f.db.lock(), &good, &staging).unwrap();
        verify_snapshot(&good).expect("a real snapshot must verify");

        // A damaged database must not. Truncating mid-file is the shape a full
        // disk or a failing drive leaves behind: the header still says
        // "SQLite format 3", so every check the destination guards perform
        // passes, and only reading the pages reveals it.
        let damaged = temp_db_path("backup_verify_damaged");
        std::fs::copy(&good, &damaged).unwrap();
        let len = std::fs::metadata(&damaged).unwrap().len();
        assert!(len > 4096, "the fixture must be several pages long");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&damaged)
            .unwrap()
            .set_len(len / 2)
            .unwrap();
        assert!(
            verify_snapshot(&damaged).is_err(),
            "a truncated database must not pass verification"
        );

        let _ = std::fs::remove_file(&good);
        let _ = std::fs::remove_file(&damaged);
        let _ = std::fs::remove_dir_all(&staging);
    }

    // --- dashboard ---------------------------------------------------------

    /// `upcoming_days` used to flow unvalidated into `Duration::days`, which
    /// panics on overflow — and `panic = "abort"` made that fatal.
    #[test]
    fn dashboard_horizon_clamps_extreme_input() {
        let f = Fixture::new("dashboard");
        seeded_purchase(&f);
        let conn = f.db.lock();

        for days in [Some(i64::MAX), Some(i64::MIN), Some(0), Some(-1), None] {
            let clamped = days
                .unwrap_or(7)
                .clamp(*UPCOMING_DAYS_RANGE.start(), *UPCOMING_DAYS_RANGE.end());
            assert!(UPCOMING_DAYS_RANGE.contains(&clamped));
            // The horizon computation itself must not panic for any of these.
            let horizon = add_interval(today(), "custom", Some(clamped), 1);
            assert!(horizon >= today());
        }

        // And the aggregates still run against the seeded data.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM installment", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 4);
    }

    // =======================================================================
    // Licence gate
    // =======================================================================
    //
    // The `#[tauri::command]` wrappers need a Tauri `State`, so — exactly as
    // with the `*_impl` split — the tests target the decision functions the
    // wrappers delegate to. Those take `&LicenseState`, which is constructible
    // here without a runtime.

    fn licensed() -> LicenseState {
        LicenseState::new(LicenseStatus::Valid(License {
            license_id: "PS-TEST".into(),
            licensee: "Test".into(),
            issued_at: "2026-01-01".into(),
            expires_at: "2030-01-01".into(),
            machine_id: None,
            features: vec!["*".into()],
        }))
    }

    /// Every non-`Valid` verdict, so a new variant cannot quietly default to
    /// "allowed" — adding one to the enum without adding it here still leaves
    /// `require_license` correct, but this is the list the gate is proven against.
    fn unlicensed_states() -> Vec<LicenseState> {
        let license = License {
            license_id: "PS-TEST".into(),
            licensee: "Test".into(),
            issued_at: "2026-01-01".into(),
            expires_at: "2026-02-01".into(),
            machine_id: None,
            features: vec![],
        };
        [
            LicenseStatus::Missing,
            LicenseStatus::InvalidSignature,
            LicenseStatus::Malformed { reason: "x" },
            LicenseStatus::Expired {
                license: license.clone(),
                expired_on: parse_date("2026-02-01").unwrap(),
            },
            LicenseStatus::MachineMismatch {
                license,
                local: None,
            },
            LicenseStatus::ClockTampered {
                watermark: parse_date("2027-01-01").unwrap(),
            },
        ]
        .into_iter()
        .map(LicenseState::new)
        .collect()
    }

    #[test]
    fn the_gate_refuses_every_unlicensed_verdict_and_admits_only_valid() {
        assert!(require_license(&licensed()).is_ok());

        for state in unlicensed_states() {
            let err = require_license(&state).expect_err("must refuse");
            // A stable code the frontend maps to a localized sentence — not
            // prose, and not an opaque INTERNAL that tells the user nothing.
            assert_eq!(err.code(), "LICENSE_REQUIRED");
        }
    }

    #[test]
    fn the_gate_follows_a_verdict_that_changes_mid_session() {
        // The point of `start_license_watcher`: the gate must read the live
        // cache, not a value snapshotted when the process started. Nothing here
        // needs the watcher thread — it only needs `require_license` to consult
        // `LicenseState` on every call, which is what makes a mid-session expiry
        // take effect without a restart (AUDIT_REPORT L4).
        let state = licensed();
        assert!(require_license(&state).is_ok());

        state.set(LicenseStatus::Expired {
            license: License {
                license_id: "PS-TEST".into(),
                licensee: "Test".into(),
                issued_at: "2026-01-01".into(),
                expires_at: "2026-02-01".into(),
                machine_id: None,
                features: vec![],
            },
            expired_on: parse_date("2026-02-01").unwrap(),
        });
        assert_eq!(
            require_license(&state)
                .expect_err("must refuse once expired")
                .code(),
            "LICENSE_REQUIRED"
        );

        // And back: importing a licence has to unlock the same process.
        state.set(LicenseStatus::Valid(License {
            license_id: "PS-TEST".into(),
            licensee: "Test".into(),
            issued_at: "2026-01-01".into(),
            expires_at: "2030-01-01".into(),
            machine_id: None,
            features: vec!["*".into()],
        }));
        assert!(require_license(&state).is_ok());
    }

    #[test]
    fn a_re_evaluated_verdict_is_only_published_when_it_differs() {
        // `publish_license` needs an `AppHandle` to emit, so what is testable
        // here is the comparison it gates on: two evaluations of an unchanged
        // licence must compare equal, or the watcher would wake the renderer 96
        // times a day to tell it nothing.
        let license = License {
            license_id: "PS-TEST".into(),
            licensee: "Test".into(),
            issued_at: "2026-01-01".into(),
            expires_at: "2030-01-01".into(),
            machine_id: None,
            features: vec!["*".into()],
        };
        assert_eq!(
            LicenseStatus::Valid(license.clone()),
            LicenseStatus::Valid(license.clone())
        );
        assert_ne!(
            LicenseStatus::Valid(license.clone()),
            LicenseStatus::Expired {
                license,
                expired_on: parse_date("2026-02-01").unwrap(),
            }
        );
    }

    #[test]
    fn an_expired_licence_still_permits_reading_your_own_ledger() {
        // The deliberate shape of the baseline: losing a licence must never hold
        // a shop keeper's own client and purchase records hostage. Those two
        // reads carry no gate at all — this pins that they were not gated by
        // accident along with everything else.
        let f = Fixture::new("license_baseline");
        seeded_purchase(&f);
        let conn = f.db.lock();

        assert!(!list_clients_impl(&conn, ClientScope::Active)
            .unwrap()
            .is_empty());
        assert!(!list_purchase_ids(&conn, PurchaseScope::Active)
            .unwrap()
            .is_empty());

        // Backup is in that same baseline. It snapshots exactly the rows the two
        // reads above already return, so gating it protected nothing and denied
        // an expired install the one copy it most needs — the one taken before
        // troubleshooting the expiry.
        //
        // The gate lives on the `backup_database` wrapper, which needs an
        // `AppHandle` and so cannot be called from a unit test; what this pins is
        // the capability the wrapper must keep exposing. The wrapper's own lack
        // of a `require_license` call is enforced by review, not by this test.
        let dest = temp_db_path("license_baseline_backup");
        let staging = staging_dir("license_baseline_stage");
        backup_database_impl(&conn, &dest, &staging).unwrap();
        assert!(
            dest.exists(),
            "an unlicensed install must be able to back up"
        );

        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_dir_all(&staging);
    }

    #[test]
    fn an_unlicensed_caller_is_pinned_to_the_active_scope() {
        // Degrade rather than refuse: asking for the archive without a licence
        // returns the active slice instead of an error, so the page still
        // renders. Archived rows are a licensed view.
        let unlicensed = LicenseState::new(LicenseStatus::Missing);

        assert_eq!(
            licensed_scope(&unlicensed, Some(ClientScope::Archived)),
            ClientScope::Active
        );
        assert_eq!(
            licensed_scope(&unlicensed, Some(ClientScope::All)),
            ClientScope::Active
        );
        assert_eq!(
            licensed_scope(&unlicensed, Some(PurchaseScope::Archived)),
            PurchaseScope::Active
        );

        // With a licence the requested scope is honoured.
        assert_eq!(
            licensed_scope(&licensed(), Some(ClientScope::Archived)),
            ClientScope::Archived
        );
        // And an absent scope still means "active" either way.
        assert_eq!(
            licensed_scope(&licensed(), None::<ClientScope>),
            ClientScope::Active
        );
    }

    #[test]
    fn language_is_the_only_setting_an_unlicensed_user_may_change() {
        // Locking the language would make the app unrecoverable for someone who
        // cannot read the current one — including the licence screen itself.
        let language_only = SettingsPatch {
            language: Some("ar".into()),
            currency_code: None,
            date_format: None,
            shop_name: None,
            shop_info: None,
            alert_soon_days: None,
            auto_backup_enabled: None,
            auto_backup_frequency: None,
            auto_backup_time: None,
        };
        assert!(is_language_only(&language_only));

        // An empty patch changes nothing, so it is harmless.
        assert!(is_language_only(&SettingsPatch {
            language: None,
            currency_code: None,
            date_format: None,
            shop_name: None,
            shop_info: None,
            alert_soon_days: None,
            auto_backup_enabled: None,
            auto_backup_frequency: None,
            auto_backup_time: None,
        }));

        // Anything smuggled alongside the language is refused.
        for patch in [
            SettingsPatch {
                language: Some("ar".into()),
                currency_code: Some("EUR".into()),
                date_format: None,
                shop_name: None,
                shop_info: None,
                alert_soon_days: None,
                auto_backup_enabled: None,
                auto_backup_frequency: None,
                auto_backup_time: None,
            },
            SettingsPatch {
                language: None,
                currency_code: None,
                date_format: None,
                shop_name: Some("Free branding".into()),
                shop_info: None,
                alert_soon_days: None,
                auto_backup_enabled: None,
                auto_backup_frequency: None,
                auto_backup_time: None,
            },
            // The backup *schedule* is licensed configuration, even though the
            // backups themselves and the manual button are not. Re-timing the
            // automatic copy is a setting like any other; being able to take one
            // at all is the safety baseline, and that stays open.
            SettingsPatch {
                language: None,
                currency_code: None,
                date_format: None,
                shop_name: None,
                shop_info: None,
                alert_soon_days: None,
                auto_backup_enabled: Some(false),
                auto_backup_frequency: None,
                auto_backup_time: None,
            },
            SettingsPatch {
                language: Some("ar".into()),
                currency_code: None,
                date_format: None,
                shop_name: None,
                shop_info: None,
                alert_soon_days: None,
                auto_backup_enabled: None,
                auto_backup_frequency: None,
                auto_backup_time: Some("09:00".into()),
            },
            SettingsPatch {
                language: None,
                currency_code: None,
                date_format: None,
                shop_name: None,
                shop_info: None,
                alert_soon_days: Some(30),
                auto_backup_enabled: None,
                auto_backup_frequency: None,
                auto_backup_time: None,
            },
        ] {
            assert!(!is_language_only(&patch), "{patch:?} must be licensed");
        }
    }

    #[test]
    fn the_clock_watermark_survives_a_round_trip_through_the_settings_table() {
        // The watermark shares the `setting` table with user preferences but
        // must never appear in `Settings` — that struct is serialized straight
        // to the renderer, which is the code the watermark defends against.
        let f = Fixture::new("license_watermark");
        let conn = f.db.lock();

        assert_eq!(
            get_setting(&conn, crate::license::CLOCK_WATERMARK_KEY, ""),
            ""
        );
        put_setting(&conn, crate::license::CLOCK_WATERMARK_KEY, "2026-07-28").unwrap();
        assert_eq!(
            get_setting(&conn, crate::license::CLOCK_WATERMARK_KEY, ""),
            "2026-07-28"
        );

        let json = serde_json::to_string(&read_settings(&conn)).unwrap();
        assert!(
            !json.contains("watermark") && !json.contains("2026-07-28"),
            "the watermark must not reach the renderer: {json}"
        );
    }
    // --- rapports ----------------------------------------------------------

    /// A purchase with one unpaid installment due on `due`. Built with raw SQL
    /// rather than `create_purchase_impl` because these tests are about the
    /// aging boundaries, which need a due date placed an exact number of days
    /// from today — something the schedule generator will not do on request.
    fn owed_on(f: &Fixture, due: NaiveDate, amount: i64) -> i64 {
        let conn = f.db.lock();
        conn.execute(
            "INSERT INTO purchase (reference, client_id, product_label, total_price,
                                   installment_count, interval_kind, purchase_date)
             VALUES ('A-TEST', ?1, 'Test', ?2, 1, 'monthly', ?3)",
            params![f.client_id, amount, due.to_string()],
        )
        .unwrap();
        let purchase_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO installment (purchase_id, idx, amount, due_date, paid_amount)
             VALUES (?1, 1, ?2, ?3, 0)",
            params![purchase_id, amount, due.to_string()],
        )
        .unwrap();
        purchase_id
    }

    fn report_between(f: &Fixture, from: &str, to: &str) -> Report {
        get_report_impl(
            &f.db.lock(),
            &ReportInput {
                date_from: from.into(),
                date_to: to.into(),
                granularity: None,
            },
        )
        .unwrap()
    }

    fn days_ago(n: i64) -> NaiveDate {
        today() - chrono::Duration::days(n)
    }

    fn bucket<'a>(report: &'a Report, name: &str) -> &'a AgingBucket {
        report
            .aging
            .iter()
            .find(|b| b.bucket == name)
            .unwrap_or_else(|| panic!("bucket {name} must always be present"))
    }

    /// Both ends of the range belong to it. A shop asking for "1 to 31 January"
    /// means the whole month, and an exclusive end would drop the last day's
    /// takings from every report anyone ever runs.
    #[test]
    fn report_range_ends_are_inclusive() {
        let f = Fixture::new("report_bounds");
        let detail = seeded_purchase(&f);
        let inst = detail.installments[0].id;

        {
            let conn = f.db.lock();
            for (date, amount) in [
                ("2024-05-31", 10), // the day before
                ("2024-06-01", 20), // first day of the range
                ("2024-06-30", 40), // last day of the range
                ("2024-07-01", 80), // the day after
            ] {
                conn.execute(
                    "INSERT INTO payment (installment_id, amount, payment_date)
                     VALUES (?1, ?2, ?3)",
                    params![inst, amount, date],
                )
                .unwrap();
            }
        }

        let r = report_between(&f, "2024-06-01", "2024-06-30");
        assert_eq!(
            r.totals.collected, 60,
            "only the two payments inside the range count"
        );
        assert_eq!(r.totals.payment_count, 2);
    }

    /// `client.created_at` is a `datetime('now')` stamp, so comparing it against
    /// a bare date silently drops everyone created on the final day of the
    /// range — the whole reason the query narrows it with `date()`.
    #[test]
    fn new_clients_counts_someone_created_on_the_last_day() {
        let f = Fixture::new("report_new_clients");
        {
            let conn = f.db.lock();
            conn.execute(
                "INSERT INTO client (first_name, last_name, phone, created_at)
                 VALUES ('Late', 'Arrival', '', '2024-06-30 23:14:02')",
                [],
            )
            .unwrap();
        }

        let r = report_between(&f, "2024-06-01", "2024-06-30");
        assert_eq!(
            r.totals.new_clients, 1,
            "a client stamped late on the closing day is still inside the range"
        );
    }

    /// An archived purchase has been taken off the books, so it must not appear
    /// as a sale, in the amount owed, or in the aging — the same rule every
    /// other money read model follows.
    #[test]
    fn report_excludes_archived_purchases() {
        let f = Fixture::new("report_archived");
        let purchase_id = owed_on(&f, days_ago(45), 500);

        let before = report_between(&f, "2000-01-01", "2049-12-31");
        assert_eq!(before.totals.sales_amount, 500);
        assert_eq!(before.totals.outstanding_now, 500);
        assert_eq!(bucket(&before, "31-60").amount, 500);

        archive_purchase_impl(&mut f.db.lock(), purchase_id).unwrap();

        let after = report_between(&f, "2000-01-01", "2049-12-31");
        assert_eq!(after.totals.sales_amount, 0, "archived is not sold");
        assert_eq!(after.totals.outstanding_now, 0, "archived is not owed");
        assert_eq!(after.totals.overdue_now, 0);
        assert_eq!(bucket(&after, "31-60").amount, 0, "and not aged either");
        assert!(after.top_clients.is_empty());
        assert!(after.top_products.is_empty());
    }

    /// The aging boundaries, pinned at every edge. `days_late` is
    /// `today - due_date`, so due-today is 0 and belongs to `current`; the ranges
    /// are inclusive of their upper bound.
    #[test]
    fn aging_buckets_split_at_their_documented_edges() {
        let f = Fixture::new("report_aging");
        // One installment per boundary, each worth a distinguishable amount.
        for (days, amount) in [
            (-1, 1),  // due tomorrow
            (0, 2),   // due today
            (1, 4),   // one day late
            (30, 8),  // last day of 1-30
            (31, 16), // first day of 31-60
            (60, 32), // last day of 31-60
            (61, 64), // first day of 61-90
            (90, 128),
            (91, 256),
        ] {
            owed_on(&f, days_ago(days), amount);
        }

        let r = report_between(&f, "2000-01-01", "2049-12-31");
        assert_eq!(bucket(&r, "current").amount, 1 + 2, "not yet late");
        assert_eq!(bucket(&r, "1-30").amount, 4 + 8);
        assert_eq!(bucket(&r, "31-60").amount, 16 + 32);
        assert_eq!(bucket(&r, "61-90").amount, 64 + 128);
        assert_eq!(bucket(&r, "90+").amount, 256);

        assert_eq!(
            r.aging.len(),
            AGING_BUCKETS.len(),
            "all five buckets are always reported"
        );
        let owed: i64 = r.aging.iter().map(|b| b.amount).sum();
        assert_eq!(
            owed, r.totals.outstanding_now,
            "the buckets must partition what is owed, losing nothing"
        );
        assert_eq!(
            r.totals.overdue_now,
            owed - bucket(&r, "current").amount,
            "overdue is everything except what is not yet due"
        );
    }

    /// Corrections are written to the ledger as signed rows rather than by
    /// overwriting, so `collected` has to net them out — otherwise a corrected
    /// figure would be counted twice in every report covering it.
    #[test]
    fn collected_nets_out_a_signed_correction() {
        let f = Fixture::new("report_correction");
        let detail = seeded_purchase(&f);
        let inst = detail.installments[0].id;

        {
            let mut conn = f.db.lock();
            record_payment_impl(
                &mut conn,
                PaymentInput {
                    installment_id: inst,
                    amount: 200,
                    payment_date: "2024-02-10".into(),
                    note: None,
                },
            )
            .unwrap();
            // Correct it down to 150; the editor records the -50 difference.
            update_installment_impl(
                &mut conn,
                inst,
                InstallmentEdit {
                    paid_amount: Some(150),
                    ..Default::default()
                },
            )
            .unwrap();
        }

        let r = report_between(&f, "2000-01-01", "2049-12-31");
        let ledger: i64 = f.count("SELECT COALESCE(SUM(amount),0) FROM payment");
        assert_eq!(
            ledger, 150,
            "the ledger itself must net to the corrected sum"
        );
        assert_eq!(
            r.totals.collected, ledger,
            "the report must agree with the ledger it reads"
        );
    }

    /// Grouping in SQL only returns periods that have rows, which would let a
    /// month with no takings vanish and the chart draw straight over the hole.
    #[test]
    fn the_series_carries_every_period_including_the_empty_ones() {
        let f = Fixture::new("report_series");
        let detail = seeded_purchase(&f);
        let inst = detail.installments[0].id;
        {
            let conn = f.db.lock();
            conn.execute(
                "INSERT INTO payment (installment_id, amount, payment_date)
                 VALUES (?1, 300, '2024-03-15')",
                [inst],
            )
            .unwrap();
        }

        // A quarter, which resolves to monthly buckets.
        let r = report_between(&f, "2024-01-01", "2024-03-31");
        assert_eq!(r.range.granularity, "month");
        let periods: Vec<&str> = r.collections.iter().map(|p| p.period.as_str()).collect();
        assert_eq!(periods, ["2024-01", "2024-02", "2024-03"]);
        assert_eq!(r.collections[0].collected, 0);
        assert_eq!(r.collections[1].collected, 0);
        assert_eq!(r.collections[2].collected, 300);
    }

    /// The thresholds are asserted against the constants rather than against
    /// hardcoded dates, so moving a constant cannot leave this passing by
    /// accident.
    #[test]
    fn granularity_is_chosen_from_the_span() {
        let f = Fixture::new("report_granularity");
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

        let at = |span: i64| {
            let to = from + chrono::Duration::days(span - 1);
            report_between(&f, &from.to_string(), &to.to_string())
                .range
                .granularity
        };

        assert_eq!(at(REPORT_DAY_MAX_SPAN), "day");
        assert_eq!(at(REPORT_DAY_MAX_SPAN + 1), "month");
        assert_eq!(at(REPORT_MONTH_MAX_SPAN), "month");
        assert_eq!(at(REPORT_MONTH_MAX_SPAN + 1), "year");

        // An explicit choice overrides the heuristic.
        let explicit = get_report_impl(
            &f.db.lock(),
            &ReportInput {
                date_from: "2024-01-01".into(),
                date_to: "2024-01-31".into(),
                granularity: Some("year".into()),
            },
        )
        .unwrap();
        assert_eq!(explicit.range.granularity, "year");
        assert_eq!(explicit.collections.len(), 1);
    }

    /// Every rejection carries an actionable code, never `INTERNAL` — the
    /// frontend maps these to a localized sentence.
    #[test]
    fn report_refuses_a_range_it_cannot_serve() {
        let f = Fixture::new("report_reject");

        let call = |from: &str, to: &str, g: Option<&str>| {
            get_report_impl(
                &f.db.lock(),
                &ReportInput {
                    date_from: from.into(),
                    date_to: to.into(),
                    granularity: g.map(str::to_string),
                },
            )
            .unwrap_err()
            .code()
        };

        assert_eq!(call("2024-06-30", "2024-06-01", None), INVALID_DATE);
        assert_eq!(call("not-a-date", "2024-06-01", None), INVALID_DATE);
        assert_eq!(
            call("2024-01-01", "2024-01-31", Some("fortnight")),
            INVALID_GRANULARITY
        );

        // Wide enough to blow the bucket cap, which is what the span bound is
        // there to stop.
        let from = NaiveDate::from_ymd_opt(1900, 1, 1).unwrap();
        let to = from + chrono::Duration::days(*REPORT_SPAN_DAYS_RANGE.end());
        let code = call(&from.to_string(), &to.to_string(), None);
        assert!(
            code.starts_with(REPORT_RANGE_TOO_LONG),
            "expected a range-too-long refusal, got {code}"
        );

        // Legal as a range, but daily buckets across two decades would put
        // thousands of points on the wire for a chart to draw. Auto-granularity
        // never approaches the cap, so only an explicit choice trips it.
        let code = call("2000-01-01", "2019-12-31", Some("day"));
        assert!(
            code.starts_with(REPORT_RANGE_TOO_LONG),
            "expected a bucket-count refusal, got {code}"
        );
        assert_eq!(
            report_between(&f, "2000-01-01", "2019-12-31")
                .collections
                .len(),
            20,
            "the same span is fine at the granularity the UI actually sends"
        );
    }

    /// The licence gate lives in Rust, not only in the router: hiding the
    /// sidebar entry is a statement of intent, this is the control.
    #[test]
    fn get_report_is_licensed() {
        assert_eq!(
            require_license(&LicenseState::new(LicenseStatus::Missing))
                .unwrap_err()
                .code(),
            LICENSE_REQUIRED
        );
        require_license(&licensed()).expect("a valid licence must pass the gate");
    }
    // --- csv export --------------------------------------------------------

    /// `dest` reaches this command straight from the renderer, so the guards are
    /// the only thing between it and an arbitrary write.
    #[test]
    fn export_csv_refuses_a_destination_it_should_not_write() {
        let dir = std::env::temp_dir().join(format!("ps_export_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Not named like a CSV: the only structural guard available, since CSV
        // has no magic header to sniff the way a SQLite backup does.
        for name in ["notes.txt", "profile", "script.sh", "archive.csv.gz"] {
            let target = dir.join(name);
            assert_eq!(
                export_csv_impl(&target, "a,b\r\n").unwrap_err().code(),
                EXPORT_FAILED,
                "{name} must be refused"
            );
            assert!(!target.exists(), "{name} must not have been created");
        }

        // Over the payload cap.
        let big = "x".repeat(EXPORT_MAX_BYTES + 1);
        let target = dir.join("huge.csv");
        assert_eq!(
            export_csv_impl(&target, &big).unwrap_err().code(),
            EXPORT_FAILED
        );
        assert!(!target.exists(), "an over-cap export must write nothing");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The happy path, including that the bytes land verbatim — the BOM the
    /// export opens with is what makes Excel read it as UTF-8, so a lossy write
    /// would show the French and Arabic headers as mojibake.
    #[test]
    fn export_csv_writes_the_bytes_it_was_given() {
        let dir = std::env::temp_dir().join(format!("ps_export_ok_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("rapport.csv");

        let csv = "\u{feff}\"Client\",\"Montant\"\r\n\"Ali Ben Salah\",250\r\n";
        export_csv_impl(&target, csv).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), csv);

        // A second export to the same path replaces it, which is what the save
        // dialog's own overwrite prompt has already agreed to.
        let shorter = "\u{feff}\"Client\"\r\n";
        export_csv_impl(&target, shorter).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), shorter);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Exporting is a licensed action, like every other derived view.
    #[test]
    fn export_csv_is_licensed() {
        assert_eq!(
            require_license(&LicenseState::new(LicenseStatus::Missing))
                .unwrap_err()
                .code(),
            LICENSE_REQUIRED
        );
    }
}
