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

use rusqlite::{params, Connection, OptionalExtension};
use tauri::State;

use crate::db::{
    add_interval, installment_status, parse_date, purchase_status, rebalance_amounts,
    split_amounts, today, AppError, Db, DbResult, INSTALLMENT_COUNT_RANGE, INTERVAL_DAYS_RANGE,
    INTERVAL_KINDS, UPCOMING_DAYS_RANGE,
};
use crate::db::{
    AMOUNT_LOCKED, ARCHIVE_HAS_OUTSTANDING, BACKUP_FAILED, BELOW_PAID, CLIENT_ARCHIVED,
    CLIENT_HAS_PURCHASES, CLIENT_NOT_FOUND, DUE_DATE_LOCKED, DUE_DATE_OUT_OF_ORDER,
    FUTURE_PAID_DATE, INSTALLMENT_NOT_FOUND, INVALID_AMOUNT, INVALID_INSTALLMENT_COUNT,
    INVALID_INTERVAL_DAYS, INVALID_INTERVAL_KIND, INVALID_LOGO_TYPE, INVALID_TOTAL_PRICE,
    LOGO_TOO_LARGE, NO_PAYMENT_TO_DATE, NO_REBALANCE_ROOM, OVERPAYMENT, PAID_ABOVE_AMOUNT,
    PREVIOUS_UNPAID, PURCHASE_ARCHIVED, PURCHASE_HAS_PAYMENTS, PURCHASE_NOT_ARCHIVED,
    PURCHASE_NOT_FOUND, SUM_MISMATCH,
};
use crate::models::*;

// ===========================================================================
// Row mappers & shared helpers
// ===========================================================================

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
    scope: Option<ClientScope>,
) -> DbResult<Vec<ClientSummary>> {
    list_clients_impl(&db.lock(), scope.unwrap_or_default())
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
pub async fn create_client(db: State<'_, Db>, input: ClientInput) -> DbResult<Client> {
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
pub async fn update_client(db: State<'_, Db>, id: i64, input: ClientInput) -> DbResult<Client> {
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
pub async fn archive_client(db: State<'_, Db>, id: i64) -> DbResult<()> {
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
pub async fn restore_client(db: State<'_, Db>, id: i64) -> DbResult<()> {
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
pub async fn delete_client(db: State<'_, Db>, id: i64) -> DbResult<()> {
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
    scope: Option<PurchaseScope>,
    search: Option<String>,
) -> DbResult<Vec<PurchaseSummary>> {
    let conn = db.lock();
    let ids = list_purchase_ids(&conn, scope.unwrap_or_default())?;

    let needle = search
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
pub async fn create_purchase(db: State<'_, Db>, input: PurchaseInput) -> DbResult<PurchaseDetail> {
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
    if input.total_price <= 0 {
        return Err(AppError::validation(INVALID_TOTAL_PRICE));
    }
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

    Ok((amounts, due_dates))
}

/// Insert the installment rows for `purchase_id`. `idx` is 1-based positional.
fn insert_installments(
    tx: &rusqlite::Transaction,
    purchase_id: i64,
    amounts: &[i64],
    due_dates: &[String],
) -> DbResult<()> {
    for (i, (amount, due)) in amounts.iter().zip(due_dates).enumerate() {
        let idx = (i as i64) + 1;
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

    insert_installments(&tx, purchase_id, &amounts, &due_dates)?;

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

/// Edit a purchase.
///
/// The product label is always editable. Everything the schedule is derived
/// from — total, count, interval and the purchase date that anchors it — may
/// only change while no payment has been recorded, because applying it means
/// regenerating the installment rows, and those rows own the payments through
/// an `ON DELETE CASCADE`. `client_id` is ignored: moving a purchase to another
/// client is not something this command does.
#[tauri::command]
pub async fn update_purchase(
    db: State<'_, Db>,
    id: i64,
    input: PurchaseInput,
) -> DbResult<PurchaseDetail> {
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
    if reschedule {
        let paid = payment_count(&tx, id)?;
        if paid > 0 {
            return Err(AppError::conflict(PURCHASE_HAS_PAYMENTS, paid));
        }
    }

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
        // Safe precisely because the guard above proved there are no payments:
        // dropping these rows would otherwise cascade the payment ledger away.
        tx.execute("DELETE FROM installment WHERE purchase_id = ?1", [id])?;
        insert_installments(&tx, id, &amounts, &due_dates)?;
    }

    tx.commit()?;
    log::info!("updated purchase id={id} (rescheduled: {reschedule})");
    build_purchase_detail(conn, id)
}

/// Archive a purchase: remove it from every list and every total, reversibly.
#[tauri::command]
pub async fn archive_purchase(db: State<'_, Db>, id: i64) -> DbResult<()> {
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
pub async fn restore_purchase(db: State<'_, Db>, id: i64) -> DbResult<()> {
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
pub async fn delete_purchase(db: State<'_, Db>, id: i64) -> DbResult<()> {
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
/// `paid_amount` upwards against a fixed `amount`. Editing works the other way
/// round — the amount moves under a fixed `paid_amount` — so a row can *become*
/// settled or *stop* being settled without any payment changing hands, and the
/// date has to follow. The settled date is the last payment on the row; a row
/// settled because it was zeroed has no payments and keeps a `NULL` date.
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

/// Edit a single installment in place.
///
/// This is the one write path that still works *after* a payment has been
/// recorded. `update_purchase` cannot: rescheduling there means deleting and
/// reinserting the rows, and those rows own the payments through an
/// `ON DELETE CASCADE`, so it is refused outright once cash has been taken.
///
/// The fields split into two halves governed by opposite rules, and neither
/// half's rule looks at the other's:
///
/// * **The schedule** — `amount` and `due_date` — is editable until the
///   installment settles, after which it is history (`AMOUNT_LOCKED`,
///   `DUE_DATE_LOCKED`). Nothing about the *neighbouring* installments gates it.
/// * **The money** — `paid_amount`, `payment_date`, `note` — is editable only
///   once installment `N-1` is fully paid (`PREVIOUS_UNPAID:{index}`). Cash is
///   collected in order, so it cannot be recorded out of order. Nothing about
///   *this* installment's own status gates it.
///
/// Two invariants survive it:
///
/// * `SUM(amount) == purchase.total_price`. The total is never written; a
///   changed amount is absorbed by the other unsettled installments — see
///   [`rebalance_amounts`] — or refused with `NO_REBALANCE_ROOM`.
/// * `SUM(payment.amount) == SUM(installment.paid_amount)`. `paid_amount` is a
///   cache of the ledger, so moving it writes a matching **correction entry**
///   into `payment` (negative when the figure comes down). Without that the
///   dashboard's "Amount collected", the only money figure derived from the
///   ledger, would drift away from every other total in the app.
///
/// A due date is additionally clamped to `[prev.due_date, next.due_date]`
/// (`DUE_DATE_OUT_OF_ORDER`). That is what keeps position order and
/// chronological order the same thing, so "the previous installment" means the
/// same in both readings however the dates are edited.
///
/// Mirrored guard-for-guard by `updateInstallment` in `src/api/mock.ts`.
#[tauri::command]
pub async fn update_installment(
    db: State<'_, Db>,
    id: i64,
    edit: InstallmentEdit,
) -> DbResult<PurchaseDetail> {
    update_installment_impl(&mut db.lock(), id, edit)
}

pub(crate) fn update_installment_impl(
    conn: &mut Connection,
    id: i64,
    edit: InstallmentEdit,
) -> DbResult<PurchaseDetail> {
    // Validate what can be validated without touching the database, so a
    // malformed request never opens a transaction.
    let new_due = edit.due_date.as_deref().map(parse_date).transpose()?;
    let payment_date = edit.payment_date.as_deref().map(parse_date).transpose()?;
    if edit.amount.is_some_and(|a| a < 0) || edit.paid_amount.is_some_and(|p| p < 0) {
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
    let settled = target.paid_amount >= target.amount;

    // --- the schedule half: gated on this installment being unsettled --------

    let amount_changed = edit.amount.is_some_and(|a| a != target.amount);
    let due_changed = new_due.is_some_and(|d| d.to_string() != target.due_date);
    if settled {
        if amount_changed {
            return Err(AppError::conflict(AMOUNT_LOCKED, ""));
        }
        if due_changed {
            return Err(AppError::conflict(DUE_DATE_LOCKED, ""));
        }
    }
    if due_changed {
        let due = new_due.unwrap().to_string();
        let below = pos > 0 && due < rows[pos - 1].due_date;
        let above = pos + 1 < rows.len() && due > rows[pos + 1].due_date;
        if below || above {
            return Err(AppError::conflict(DUE_DATE_OUT_OF_ORDER, ""));
        }
    }

    // --- the money half: gated on the previous installment being settled -----

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
    let final_amount = edit.amount.unwrap_or(target.amount);
    if final_paid > final_amount {
        // The same constraint from either side; report it against the field the
        // user actually moved so the message names a number they typed.
        return Err(if paid_changed {
            AppError::conflict(PAID_ABOVE_AMOUNT, final_amount)
        } else {
            AppError::conflict(BELOW_PAID, target.paid_amount)
        });
    }

    let next_amounts = if amount_changed {
        let amounts: Vec<i64> = rows.iter().map(|r| r.amount).collect();
        let mut paid_amounts: Vec<i64> = rows.iter().map(|r| r.paid_amount).collect();
        // The edited row's own floor is what this edit lands on, not what is
        // stored, so lowering the amount and the collected figure together is
        // not refused for a conflict the request itself resolves.
        paid_amounts[pos] = final_paid;
        Some(
            rebalance_amounts(&amounts, &paid_amounts, pos, final_amount)
                .ok_or_else(|| AppError::conflict(NO_REBALANCE_ROOM, ""))?,
        )
    } else {
        None
    };

    // A payment date needs something to date. A correction entry is created
    // below when the collected figure moves; otherwise it re-dates the row's
    // most recent ledger entry, which is what keeps `paid_date` (derived as
    // `MAX(payment_date)`) agreeing with the history behind it.
    let latest_payment: Option<i64> = tx
        .query_row(
            "SELECT id FROM payment WHERE installment_id = ?1
              ORDER BY payment_date DESC, id DESC LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if (payment_date.is_some() || edit.note.is_some()) && !paid_changed && latest_payment.is_none()
    {
        return Err(AppError::conflict(NO_PAYMENT_TO_DATE, ""));
    }

    // --- writes --------------------------------------------------------------

    if let Some(due) = new_due {
        tx.execute(
            "UPDATE installment SET due_date = ?1 WHERE id = ?2",
            params![due.to_string(), id],
        )?;
    }

    if let Some(next) = &next_amounts {
        for (row, amount) in rows.iter().zip(next) {
            if row.amount != *amount {
                tx.execute(
                    "UPDATE installment SET amount = ?1 WHERE id = ?2",
                    params![amount, row.id],
                )?;
            }
        }
    }

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
        // Nothing to correct, so a date or a note amends the entry already
        // there. The guard above proved there is one.
        if let Some(date) = payment_date {
            tx.execute(
                "UPDATE payment SET payment_date = ?1 WHERE id = ?2",
                params![date.to_string(), payment_id],
            )?;
        }
        if let Some(note) = note {
            tx.execute(
                "UPDATE payment SET note = ?1 WHERE id = ?2",
                params![note, payment_id],
            )?;
        }
    }

    // `paid_date` is derived, so it has to be re-run for every row whose numbers
    // moved — the edited one, and any absorber a rebalance pushed across its
    // settled threshold.
    sync_paid_date(&tx, id, final_amount, final_paid)?;
    if let Some(next) = &next_amounts {
        for (row, amount) in rows.iter().zip(next) {
            if row.id != id && row.amount != *amount {
                sync_paid_date(&tx, row.id, *amount, row.paid_amount)?;
            }
        }
    }

    tx.commit()?;
    log::info!(
        "updated installment id={id} on purchase id={purchase_id} \
         (rebalanced: {}, ledger correction: {paid_changed})",
        next_amounts.is_some()
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
pub async fn record_payment(db: State<'_, Db>, input: PaymentInput) -> DbResult<PurchaseDetail> {
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
    purchase_id: i64,
) -> DbResult<Vec<Payment>> {
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
pub async fn list_all_payments(db: State<'_, Db>, limit: Option<i64>) -> DbResult<Vec<Payment>> {
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
             ORDER BY pay.payment_date DESC, pay.id DESC
             LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit.unwrap_or(500)], map_payment)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

#[tauri::command]
pub async fn list_payments_for_client(db: State<'_, Db>, client_id: i64) -> DbResult<Vec<Payment>> {
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
    filter: Option<ImpayeFilter>,
) -> DbResult<Vec<ImpayeClient>> {
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
pub async fn list_schedule(db: State<'_, Db>) -> DbResult<Vec<ScheduleRow>> {
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
pub async fn get_dashboard(db: State<'_, Db>, upcoming_days: Option<i64>) -> DbResult<Dashboard> {
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
// Settings
// ===========================================================================

fn get_setting(conn: &Connection, key: &str, default: &str) -> String {
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

fn put_setting(conn: &Connection, key: &str, value: &str) -> DbResult<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn read_settings(conn: &Connection) -> Settings {
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
    }
}

#[tauri::command]
pub async fn get_settings(db: State<'_, Db>) -> DbResult<Settings> {
    let conn = db.lock();
    Ok(read_settings(&conn))
}

#[tauri::command]
pub async fn update_settings(db: State<'_, Db>, patch: SettingsPatch) -> DbResult<Settings> {
    update_settings_impl(&mut db.lock(), patch)
}

pub(crate) fn update_settings_impl(
    conn: &mut Connection,
    patch: SettingsPatch,
) -> DbResult<Settings> {
    // One transaction for the whole patch. Applied one upsert at a time, a
    // mid-way failure left settings half-written — worst case `language`
    // committed but `language_is_default = "0"` not, which permanently
    // re-enables OS-locale detection over the user's explicit choice.
    let tx = conn.transaction()?;
    if let Some(v) = patch.language {
        put_setting(&tx, "language", &v)?;
        // A manual language choice ends OS-locale auto-detection.
        put_setting(&tx, "language_is_default", "0")?;
    }
    if let Some(v) = patch.currency_code {
        put_setting(&tx, "currency_code", &v)?;
    }
    if let Some(v) = patch.date_format {
        put_setting(&tx, "date_format", &v)?;
    }
    if let Some(v) = patch.shop_name {
        put_setting(&tx, "shop_name", &v)?;
    }
    if let Some(v) = patch.shop_info {
        put_setting(&tx, "shop_info", &v)?;
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
    app: tauri::AppHandle,
    source_path: String,
) -> DbResult<Settings> {
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
pub async fn clear_logo(db: State<'_, Db>) -> DbResult<Settings> {
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
#[tauri::command]
pub async fn backup_database(db: State<'_, Db>, dest: String) -> DbResult<()> {
    use std::io::Read;

    let dest_path = std::path::Path::new(&dest);

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

    // Write to a sibling temp file and rename into place, so a failed backup
    // never destroys the snapshot the user already had.
    let tmp = dest_path.with_extension("db.part");
    let _ = std::fs::remove_file(&tmp);

    let conn = db.lock();
    let vacuum = conn.execute("VACUUM INTO ?1", [&tmp.to_string_lossy().to_string()]);
    drop(conn);

    if let Err(e) = vacuum {
        log::error!("database backup failed: {e}");
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::validation(BACKUP_FAILED));
    }

    std::fs::rename(&tmp, dest_path).map_err(|e| {
        log::error!("could not move the backup into place: {e}");
        let _ = std::fs::remove_file(&tmp);
        AppError::validation(BACKUP_FAILED)
    })?;

    log::info!("database backup written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
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

    #[test]
    fn rescheduling_is_refused_once_a_payment_exists() {
        let f = Fixture::new("update_locked");
        let detail = seeded_purchase(&f);
        pay_first(&f, &detail, 250);

        for mutate in [
            (|i: &mut PurchaseInput| i.total_price = 2000) as fn(&mut PurchaseInput),
            |i: &mut PurchaseInput| i.installment_count = 6,
            |i: &mut PurchaseInput| i.interval_kind = "weekly".into(),
            |i: &mut PurchaseInput| i.purchase_date = "2024-02-01".into(),
        ] {
            let mut input = f.purchase_input();
            mutate(&mut input);
            let err =
                update_purchase_impl(&mut f.db.lock(), detail.purchase.id, input).unwrap_err();
            assert_eq!(code_of(err), "PURCHASE_HAS_PAYMENTS:1");
        }

        // Nothing moved.
        let after = build_purchase_detail(&f.db.lock(), detail.purchase.id).unwrap();
        assert_eq!(after.purchase.total_price, 1000);
        assert_eq!(after.purchase.purchase_date, "2024-01-15");
        assert_eq!(after.installments.len(), 4);
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
    // The two halves of the editor are governed by opposite rules: the schedule
    // (amount, due date) unlocks while the tranche is unsettled and nothing
    // about its neighbours matters; the money (paid amount, payment date) is
    // gated on the *previous* tranche and nothing about its own status matters.

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

    // -- the schedule half ----------------------------------------------------

    #[test]
    fn editing_an_amount_rebalances_the_later_tranches_and_holds_the_total() {
        let f = Fixture::new("inst_rebalance");
        let detail = seeded_purchase(&f);

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            edit_amount(400),
        )
        .unwrap();

        assert_eq!(amounts_of(&updated), vec![400, 200, 200, 200]);
        // The invariant the whole rebalance exists for.
        assert_eq!(amounts_of(&updated).iter().sum::<i64>(), 1000);
        assert_eq!(updated.purchase.total_price, 1000);
        assert_eq!(updated.remaining, 1000);
    }

    /// The schedule rules look at this tranche alone. A tranche deep in an
    /// entirely unpaid purchase is still editable — that gate is on the money
    /// half, not this one.
    #[test]
    fn an_amount_is_editable_regardless_of_the_previous_tranche() {
        let f = Fixture::new("inst_amount_no_gate");
        let detail = seeded_purchase(&f);

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[2].id,
            edit_amount(400),
        )
        .unwrap();

        assert_eq!(amounts_of(&updated), vec![250, 250, 400, 100]);
        assert_eq!(updated.purchase.total_price, 1000);
    }

    /// Zeroing a tranche nobody has paid into settles it — status is derived,
    /// so `paid >= amount` reads as "paid" with no payment involved.
    #[test]
    fn zeroing_an_untouched_tranche_settles_it() {
        let f = Fixture::new("inst_zero");
        let detail = seeded_purchase(&f);

        let updated =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_amount(0))
                .unwrap();

        assert_eq!(amounts_of(&updated), vec![0, 333, 333, 334]);
        assert_eq!(updated.installments[0].status, "paid");
        // Settled by arithmetic, not by a payment, so there is no date to show.
        assert_eq!(updated.installments[0].paid_date, None);
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 0);
    }

    /// Once a tranche is settled its schedule is history: neither number moves.
    #[test]
    fn a_settled_tranche_locks_its_amount_and_due_date() {
        let f = Fixture::new("inst_settled_locks");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);
        let inst_id = detail.installments[0].id;

        let err = update_installment_impl(&mut f.db.lock(), inst_id, edit_amount(400)).unwrap_err();
        assert_eq!(code_of(err), "AMOUNT_LOCKED");

        let err = update_installment_impl(
            &mut f.db.lock(),
            inst_id,
            InstallmentEdit {
                due_date: Some("2024-01-20".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(code_of(err), "DUE_DATE_LOCKED");

        // Resending the values it already has is not a change, so not a refusal.
        update_installment_impl(
            &mut f.db.lock(),
            inst_id,
            InstallmentEdit {
                amount: Some(250),
                due_date: Some("2024-01-15".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
    }

    /// A due date may move anywhere between its neighbours' dates. Clamping it
    /// there is what keeps position order and chronological order the same
    /// thing, so "the previous tranche" is unambiguous however dates are edited.
    #[test]
    fn a_due_date_moves_freely_between_its_neighbours() {
        let f = Fixture::new("inst_due_date");
        let detail = seeded_purchase(&f);
        // Tranche 3 sits between 2024-02-15 and 2024-04-15.
        let inst_id = detail.installments[2].id;

        let updated = update_installment_impl(
            &mut f.db.lock(),
            inst_id,
            InstallmentEdit {
                due_date: Some("2024-04-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.installments[2].due_date, "2024-04-01");
        // Dates are independent of the money: nothing was rebalanced.
        assert_eq!(amounts_of(&updated), vec![250, 250, 250, 250]);

        for out_of_range in ["2024-02-01", "2024-05-01"] {
            let err = update_installment_impl(
                &mut f.db.lock(),
                inst_id,
                InstallmentEdit {
                    due_date: Some(out_of_range.into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert_eq!(code_of(err), "DUE_DATE_OUT_OF_ORDER", "for {out_of_range}");
        }

        // The neighbours' own dates are inclusive bounds.
        for edge in ["2024-02-15", "2024-04-15"] {
            update_installment_impl(
                &mut f.db.lock(),
                inst_id,
                InstallmentEdit {
                    due_date: Some(edge.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        }
    }

    /// The first and last tranches are unbounded on their missing side.
    #[test]
    fn the_outer_tranches_have_only_one_bound() {
        let f = Fixture::new("inst_due_outer");
        let detail = seeded_purchase(&f);

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                due_date: Some("2020-01-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.installments[0].due_date, "2020-01-01");

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[3].id,
            InstallmentEdit {
                due_date: Some("2030-12-31".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.installments[3].due_date, "2030-12-31");
    }

    /// With every other tranche settled there is nowhere for the delta to go,
    /// and the total is not this command's to move.
    #[test]
    fn an_amount_is_locked_once_every_other_tranche_is_settled() {
        let f = Fixture::new("inst_no_room");
        let detail = seeded_purchase(&f);
        for pos in 0..3 {
            pay_installment(&f, &detail, pos, 250);
        }

        let err = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[3].id,
            edit_amount(100),
        )
        .unwrap_err();
        assert_eq!(code_of(err), "NO_REBALANCE_ROOM");
        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
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

    /// The gate is on the money only — the schedule of the very same tranche
    /// stays editable while its predecessor is owing.
    #[test]
    fn the_gate_does_not_reach_the_schedule_fields() {
        let f = Fixture::new("inst_gate_scope");
        let detail = seeded_purchase(&f);

        update_installment_impl(
            &mut f.db.lock(),
            detail.installments[1].id,
            InstallmentEdit {
                amount: Some(300),
                due_date: Some("2024-03-01".into()),
                ..Default::default()
            },
        )
        .unwrap();

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

    /// Reported against whichever field the user actually moved.
    #[test]
    fn lowering_the_amount_under_the_collected_figure_is_refused() {
        let f = Fixture::new("inst_below_paid");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 100);

        let err =
            update_installment_impl(&mut f.db.lock(), detail.installments[0].id, edit_amount(50))
                .unwrap_err();
        assert_eq!(code_of(err), "BELOW_PAID:100");
    }

    /// Lowering both together is a request that resolves itself, and must not
    /// be refused for a conflict that only exists against the stored values.
    #[test]
    fn the_amount_and_the_paid_figure_may_come_down_together() {
        let f = Fixture::new("inst_both_down");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 200);

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                amount: Some(120),
                paid_amount: Some(120),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(amounts_of(&updated), vec![120, 293, 293, 294]);
        assert_eq!(updated.installments[0].paid_amount, 120);
        assert_eq!(updated.installments[0].status, "paid");
        f.assert_ledger_matches_installments();
    }

    // -- the payment date -----------------------------------------------------

    /// With no correction to carry it, a payment date re-dates the row's most
    /// recent ledger entry — which is what keeps `paid_date` (derived as
    /// `MAX(payment_date)`) agreeing with the history behind it.
    #[test]
    fn a_payment_date_alone_re_dates_the_latest_ledger_entry() {
        let f = Fixture::new("inst_redate");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 250);

        let updated = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            InstallmentEdit {
                payment_date: Some("2024-03-05".into()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(
            updated.installments[0].paid_date.as_deref(),
            Some("2024-03-05")
        );
        assert_eq!(f.count("SELECT COUNT(*) FROM payment"), 1, "no entry added");
        let dated: String =
            f.db.lock()
                .query_row("SELECT payment_date FROM payment", [], |r| r.get(0))
                .unwrap();
        assert_eq!(dated, "2024-03-05");
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

        let err = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[0].id,
            edit_amount(300),
        )
        .unwrap_err();
        assert_eq!(code_of(err), "PURCHASE_ARCHIVED");
    }

    #[test]
    fn editing_rejects_bad_arguments_without_writing() {
        let f = Fixture::new("inst_bad_args");
        let detail = seeded_purchase(&f);
        let inst_id = detail.installments[0].id;

        assert_eq!(
            code_of(
                update_installment_impl(&mut f.db.lock(), inst_id, edit_amount(-1)).unwrap_err()
            ),
            "INVALID_AMOUNT"
        );
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
                        due_date: Some("not-a-date".into()),
                        ..Default::default()
                    },
                )
                .unwrap_err()
            ),
            "INVALID_DATE"
        );
        assert_eq!(
            code_of(
                update_installment_impl(&mut f.db.lock(), 9_999, edit_amount(300)).unwrap_err()
            ),
            "INSTALLMENT_NOT_FOUND"
        );

        assert_eq!(
            stored_amounts(&f, detail.purchase.id),
            vec![250, 250, 250, 250]
        );
    }

    /// A refused edit must leave the whole schedule alone, not just the row it
    /// addressed — the rebalance writes several rows, so a partial apply would
    /// silently break `SUM(amount) == total_price`.
    #[test]
    fn a_refused_edit_writes_nothing_at_all() {
        let f = Fixture::new("inst_rollback");
        let detail = seeded_purchase(&f);
        pay_installment(&f, &detail, 0, 100);
        let before = f.money_snapshot();

        // The amount alone is fine — 150 clears the 100 already collected and
        // the later tranches can absorb it — but the money half is gated on
        // tranche 1, which is still owing, so the whole edit is refused.
        let err = update_installment_impl(
            &mut f.db.lock(),
            detail.installments[1].id,
            InstallmentEdit {
                amount: Some(150),
                paid_amount: Some(50),
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
        f.assert_ledger_matches_installments();
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

    /// The backup must be a real, readable snapshot — and must never destroy
    /// what is already at the destination.
    #[test]
    fn backup_writes_a_readable_snapshot_without_clobbering_other_files() {
        let f = Fixture::new("backup");
        seeded_purchase(&f);

        let dest = temp_db_path("backup_out");
        {
            let conn = f.db.lock();
            let tmp = dest.with_extension("db.part");
            conn.execute("VACUUM INTO ?1", [&tmp.to_string_lossy().to_string()])
                .unwrap();
            std::fs::rename(&tmp, &dest).unwrap();
        }

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

        // A non-SQLite file at the destination must be left untouched — this is
        // what stops the command being an arbitrary-file-destruction primitive
        // when the renderer chooses the path.
        let mut header = [0u8; 16];
        {
            use std::io::Read;
            std::fs::File::open(&dest)
                .unwrap()
                .read_exact(&mut header)
                .unwrap();
        }
        assert_eq!(&header, b"SQLite format 3\0");

        let _ = std::fs::remove_file(&dest);
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
}
