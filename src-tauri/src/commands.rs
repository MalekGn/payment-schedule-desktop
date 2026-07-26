//! Tauri commands — the entire API surface exposed to the frontend.
//! Each command locks the shared connection, runs its queries, and returns
//! serde-serializable models. Errors are surfaced as `String` (shown as a
//! localized toast on the frontend).

use rusqlite::{params, Connection, OptionalExtension};
use tauri::State;

use crate::db::{
    add_interval, installment_status, parse_date, purchase_status, split_amounts, today, Db,
    DbResult,
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
    })
}

fn fetch_client(conn: &Connection, id: i64) -> DbResult<Client> {
    conn.query_row("SELECT * FROM client WHERE id = ?1", [id], map_client)
        .map_err(|e| e.to_string())
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
    })
}

/// Load a purchase's installments with their effective status computed.
fn load_installments(conn: &Connection, purchase_id: i64) -> DbResult<Vec<Installment>> {
    let today = today();
    let mut stmt = conn
        .prepare(
            "SELECT id, purchase_id, idx, amount, due_date, paid_amount, paid_date
             FROM installment WHERE purchase_id = ?1 ORDER BY idx",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([purchase_id], |row| {
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
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn build_purchase_detail(conn: &Connection, purchase_id: i64) -> DbResult<PurchaseDetail> {
    let purchase = conn
        .query_row(
            "SELECT * FROM purchase WHERE id = ?1",
            [purchase_id],
            map_purchase,
        )
        .map_err(|e| e.to_string())?;
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
    })
}

// ===========================================================================
// Clients
// ===========================================================================

#[tauri::command]
pub fn list_clients(db: State<Db>) -> DbResult<Vec<ClientSummary>> {
    let conn = db.conn.lock().unwrap();
    let today_str = today().to_string();
    let mut stmt = conn
        .prepare(
            "SELECT c.*,
                COUNT(DISTINCT p.id) AS purchase_count,
                COALESCE(SUM(i.amount - i.paid_amount), 0) AS outstanding,
                COALESCE(SUM(CASE WHEN i.due_date < ?1 AND i.amount > i.paid_amount
                                  THEN 1 ELSE 0 END), 0) AS overdue_count
             FROM client c
             LEFT JOIN purchase p ON p.client_id = c.id
             LEFT JOIN installment i ON i.purchase_id = p.id
             GROUP BY c.id
             ORDER BY c.last_name COLLATE NOCASE, c.first_name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([today_str], |row| {
            Ok(ClientSummary {
                client: map_client(row)?,
                purchase_count: row.get("purchase_count")?,
                total_outstanding: row.get("outstanding")?,
                overdue_count: row.get("overdue_count")?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_client_detail(db: State<Db>, id: i64) -> DbResult<ClientDetail> {
    let conn = db.conn.lock().unwrap();
    let client = fetch_client(&conn, id)?;

    let mut stmt = conn
        .prepare(
            "SELECT id FROM purchase WHERE client_id = ?1 ORDER BY purchase_date DESC, id DESC",
        )
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = stmt
        .query_map([id], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    let mut purchases = Vec::new();
    let (mut total_purchased, mut total_paid, mut overdue_count) = (0i64, 0i64, 0i64);
    for pid in ids {
        let s = build_purchase_summary(&conn, pid)?;
        total_purchased += s.total_price;
        total_paid += s.paid_amount;
        overdue_count += s.overdue_count;
        purchases.push(s);
    }
    let total_outstanding = (total_purchased - total_paid).max(0);

    Ok(ClientDetail {
        client,
        purchases,
        total_purchased,
        total_paid,
        total_outstanding,
        overdue_count,
    })
}

#[tauri::command]
pub fn create_client(db: State<Db>, input: ClientInput) -> DbResult<Client> {
    let conn = db.conn.lock().unwrap();
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
    )
    .map_err(|e| e.to_string())?;
    fetch_client(&conn, conn.last_insert_rowid())
}

#[tauri::command]
pub fn update_client(db: State<Db>, id: i64, input: ClientInput) -> DbResult<Client> {
    let conn = db.conn.lock().unwrap();
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
    )
    .map_err(|e| e.to_string())?;
    fetch_client(&conn, id)
}

/// Delete a client. Refuses when the client has purchases unless `force` is set
/// (cascades to purchases/installments/payments).
#[tauri::command]
pub fn delete_client(db: State<Db>, id: i64, force: bool) -> DbResult<()> {
    let conn = db.conn.lock().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM purchase WHERE client_id = ?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count > 0 && !force {
        return Err(format!("CLIENT_HAS_PURCHASES:{count}"));
    }
    conn.execute("DELETE FROM client WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ===========================================================================
// Purchases (Achats)
// ===========================================================================

#[tauri::command]
pub fn list_purchases(db: State<Db>, search: Option<String>) -> DbResult<Vec<PurchaseSummary>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id FROM purchase ORDER BY purchase_date DESC, id DESC")
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

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
pub fn get_purchase_detail(db: State<Db>, id: i64) -> DbResult<PurchaseDetail> {
    let conn = db.conn.lock().unwrap();
    build_purchase_detail(&conn, id)
}

#[tauri::command]
pub fn create_purchase(db: State<Db>, input: PurchaseInput) -> DbResult<PurchaseDetail> {
    if input.installment_count < 1 {
        return Err("INVALID_INSTALLMENT_COUNT".into());
    }
    let purchase_date = parse_date(&input.purchase_date)?;

    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction().map_err(|e| e.to_string())?;

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
    )
    .map_err(|e| e.to_string())?;
    let purchase_id = tx.last_insert_rowid();
    let reference = format!("A-{:06}", purchase_id);
    tx.execute(
        "UPDATE purchase SET reference = ?1 WHERE id = ?2",
        params![reference, purchase_id],
    )
    .map_err(|e| e.to_string())?;

    // Determine installment amounts + due dates.
    let amounts = match &input.installments {
        Some(list) if !list.is_empty() => {
            let sum: i64 = list.iter().map(|i| i.amount).sum();
            if sum != input.total_price {
                return Err(format!("SUM_MISMATCH:{sum}:{}", input.total_price));
            }
            list.iter().map(|i| i.amount).collect::<Vec<_>>()
        }
        _ => split_amounts(input.total_price, input.installment_count),
    };

    for (i, amount) in amounts.iter().enumerate() {
        let idx = (i as i64) + 1;
        let due = match &input.installments {
            Some(list) if !list.is_empty() => list
                .get(i)
                .map(|x| x.due_date.clone())
                .unwrap_or_else(|| purchase_date.to_string()),
            // k = i (0-based): the first installment falls on the purchase
            // date, subsequent ones one interval apart.
            _ => add_interval(
                purchase_date,
                &input.interval_kind,
                input.interval_days,
                i as i64,
            )
            .to_string(),
        };
        tx.execute(
            "INSERT INTO installment (purchase_id, idx, amount, due_date)
             VALUES (?1, ?2, ?3, ?4)",
            params![purchase_id, idx, amount, due],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    build_purchase_detail(&conn, purchase_id)
}

#[tauri::command]
pub fn delete_purchase(db: State<Db>, id: i64) -> DbResult<()> {
    let conn = db.conn.lock().unwrap();
    conn.execute("DELETE FROM purchase WHERE id = ?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ===========================================================================
// Payments
// ===========================================================================

/// Record a payment against a specific installment. Supports partial payments:
/// the installment's `paid_amount` accumulates and `paid_date` is set once it
/// is fully covered.
#[tauri::command]
pub fn record_payment(db: State<Db>, input: PaymentInput) -> DbResult<PurchaseDetail> {
    if input.amount <= 0 {
        return Err("INVALID_AMOUNT".into());
    }
    parse_date(&input.payment_date)?;

    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let (purchase_id, amount, paid): (i64, i64, i64) = tx
        .query_row(
            "SELECT purchase_id, amount, paid_amount FROM installment WHERE id = ?1",
            [input.installment_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "INSTALLMENT_NOT_FOUND".to_string())?;

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
    )
    .map_err(|e| e.to_string())?;

    let paid_date = if new_paid >= amount {
        Some(input.payment_date.clone())
    } else {
        None
    };
    tx.execute(
        "UPDATE installment SET paid_amount = ?1, paid_date = ?2 WHERE id = ?3",
        params![new_paid, paid_date, input.installment_id],
    )
    .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;
    build_purchase_detail(&conn, purchase_id)
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
pub fn list_payments_for_purchase(db: State<Db>, purchase_id: i64) -> DbResult<Vec<Payment>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT pay.*, i.idx, i.purchase_id, pu.reference,
                    c.id AS client_id, c.first_name, c.last_name
             FROM payment pay
             JOIN installment i ON i.id = pay.installment_id
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             WHERE i.purchase_id = ?1
             ORDER BY pay.payment_date DESC, pay.id DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([purchase_id], map_payment)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_all_payments(db: State<Db>, limit: Option<i64>) -> DbResult<Vec<Payment>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT pay.*, i.idx, i.purchase_id, pu.reference,
                    c.id AS client_id, c.first_name, c.last_name
             FROM payment pay
             JOIN installment i ON i.id = pay.installment_id
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             ORDER BY pay.payment_date DESC, pay.id DESC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit.unwrap_or(500)], map_payment)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_payments_for_client(db: State<Db>, client_id: i64) -> DbResult<Vec<Payment>> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT pay.*, i.idx, i.purchase_id, pu.reference,
                    c.id AS client_id, c.first_name, c.last_name
             FROM payment pay
             JOIN installment i ON i.id = pay.installment_id
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             WHERE pu.client_id = ?1
             ORDER BY pay.payment_date DESC, pay.id DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([client_id], map_payment)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ===========================================================================
// Échéances / Impayés
// ===========================================================================

/// All installments due in the given window (defaults to everything), enriched
/// for the schedule screen.
#[tauri::command]
pub fn list_impayes(db: State<Db>, filter: Option<ImpayeFilter>) -> DbResult<Vec<ImpayeClient>> {
    let conn = db.conn.lock().unwrap();
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
         WHERE i.due_date < ?1 AND i.amount > i.paid_amount",
    );
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

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

    // Accumulate per client, preserving first-seen order.
    let mut order: Vec<i64> = Vec::new();
    let mut map: std::collections::HashMap<i64, ImpayeClient> = std::collections::HashMap::new();

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
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
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (cid, first, last, phone, address, email, inst) = row.map_err(|e| e.to_string())?;
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
pub fn list_schedule(db: State<Db>) -> DbResult<Vec<ScheduleRow>> {
    let conn = db.conn.lock().unwrap();
    let today = today();
    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.purchase_id, pu.reference, c.id AS client_id,
                    c.first_name, c.last_name, i.idx, pu.installment_count,
                    i.due_date, i.amount, i.paid_amount
             FROM installment i
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             ORDER BY i.due_date ASC, i.id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
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
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ===========================================================================
// Dashboard
// ===========================================================================

#[tauri::command]
pub fn get_dashboard(db: State<Db>, upcoming_days: Option<i64>) -> DbResult<Dashboard> {
    let conn = db.conn.lock().unwrap();
    let today = today();
    let today_str = today.to_string();
    let horizon = (today + chrono::Duration::days(upcoming_days.unwrap_or(7))).to_string();

    let total_purchases: i64 = conn
        .query_row("SELECT COUNT(*) FROM purchase", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let total_sales: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(total_price),0) FROM purchase",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let total_collected: i64 = conn
        .query_row("SELECT COALESCE(SUM(amount),0) FROM payment", [], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let total_outstanding: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(amount - paid_amount),0) FROM installment",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let overdue_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM installment WHERE due_date < ?1 AND amount > paid_amount",
            [&today_str],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let overdue_clients: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT pu.client_id) FROM installment i
             JOIN purchase pu ON pu.id = i.purchase_id
             WHERE i.due_date < ?1 AND i.amount > i.paid_amount",
            [&today_str],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let upcoming_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM installment
             WHERE due_date >= ?1 AND due_date <= ?2 AND amount > paid_amount",
            params![today_str, horizon],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

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
    let mut stmt = conn
        .prepare("SELECT id FROM purchase ORDER BY purchase_date DESC, id DESC LIMIT 5")
        .map_err(|e| e.to_string())?;
    let recent_ids: Vec<i64> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
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
             ORDER BY pu.purchase_date DESC, pu.id DESC LIMIT 1",
            [&today_str],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .or_else(|| recent_ids.first().copied());
    let featured_purchase = match featured_id {
        Some(id) => Some(build_purchase_detail(&conn, id)?),
        None => None,
    };

    // Due alerts: overdue installments, most days late first (top 4).
    let mut stmt = conn
        .prepare(
            "SELECT i.purchase_id, pu.reference, i.idx, pu.installment_count, i.due_date,
                    c.first_name, c.last_name
             FROM installment i
             JOIN purchase pu ON pu.id = i.purchase_id
             JOIN client c ON c.id = pu.client_id
             WHERE i.due_date < ?1 AND i.amount > i.paid_amount
             ORDER BY i.due_date ASC LIMIT 4",
        )
        .map_err(|e| e.to_string())?;
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
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

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
    conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .optional()
    .ok()
    .flatten()
    .unwrap_or_else(|| default.to_string())
}

fn put_setting(conn: &Connection, key: &str, value: &str) -> DbResult<()> {
    conn.execute(
        "INSERT INTO setting (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
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
pub fn get_settings(db: State<Db>) -> DbResult<Settings> {
    let conn = db.conn.lock().unwrap();
    Ok(read_settings(&conn))
}

#[tauri::command]
pub fn update_settings(db: State<Db>, patch: SettingsPatch) -> DbResult<Settings> {
    let conn = db.conn.lock().unwrap();
    if let Some(v) = patch.language {
        put_setting(&conn, "language", &v)?;
        // A manual language choice ends OS-locale auto-detection.
        put_setting(&conn, "language_is_default", "0")?;
    }
    if let Some(v) = patch.currency_code {
        put_setting(&conn, "currency_code", &v)?;
    }
    if let Some(v) = patch.date_format {
        put_setting(&conn, "date_format", &v)?;
    }
    if let Some(v) = patch.shop_name {
        put_setting(&conn, "shop_name", &v)?;
    }
    if let Some(v) = patch.shop_info {
        put_setting(&conn, "shop_info", &v)?;
    }
    if let Some(v) = patch.alert_soon_days {
        // Clamp defensively so the schedule query and UI never see a nonsense
        // window; the UI already constrains the input to the same range.
        let clamped = v.clamp(1, 90);
        put_setting(&conn, "alert_soon_days", &clamped.to_string())?;
    }
    Ok(read_settings(&conn))
}

/// Copy a picked image file into the app data dir and store its path as the
/// shop logo. Returns the updated settings.
#[tauri::command]
pub fn set_logo(db: State<Db>, app: tauri::AppHandle, source_path: String) -> DbResult<Settings> {
    use tauri::Manager;
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    let ext = std::path::Path::new(&source_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();
    let dest = data_dir.join(format!("logo.{ext}"));
    std::fs::copy(&source_path, &dest).map_err(|e| e.to_string())?;

    let conn = db.conn.lock().unwrap();
    put_setting(&conn, "logo_path", &dest.to_string_lossy())?;
    Ok(read_settings(&conn))
}

#[tauri::command]
pub fn clear_logo(db: State<Db>) -> DbResult<Settings> {
    let conn = db.conn.lock().unwrap();
    let existing = get_setting(&conn, "logo_path", "");
    if !existing.is_empty() {
        let _ = std::fs::remove_file(&existing);
    }
    put_setting(&conn, "logo_path", "")?;
    Ok(read_settings(&conn))
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
        let conn = db.conn.lock().unwrap();

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
}
