//! First-run demo data. Seeds a set of Tunisian clients and installment
//! purchases (TND, +216 phones) so the dashboard, impayés and schedule screens
//! are populated out of the box. Purchase dates are anchored a few months in
//! the past so that a realistic mix of paid / upcoming / overdue installments
//! exists relative to today.

use rusqlite::{params, Connection};

use crate::db::{add_interval, split_amounts, today, DbResult};

struct SeedClient {
    first: &'static str,
    last: &'static str,
    phone: &'static str,
    address: &'static str,
    email: Option<&'static str>,
}

struct SeedPurchase {
    client_idx: usize,
    product: &'static str,
    total: i64,
    count: i64,
    /// how many months before today the purchase was made
    months_ago: i64,
    /// number of leading installments that have been fully paid
    paid: i64,
}

pub fn seed(conn: &Connection) -> DbResult<()> {
    let clients = [
        SeedClient {
            first: "Mohamed",
            last: "Trabelsi",
            phone: "+216 20 123 456",
            address: "Cité El Ghazala, Ariana",
            email: Some("mohamed.trabelsi@email.tn"),
        },
        SeedClient {
            first: "Fatma",
            last: "Ben Salah",
            phone: "+216 22 345 678",
            address: "Avenue Habib Bourguiba, Tunis",
            email: Some("fatma.bensalah@email.tn"),
        },
        SeedClient {
            first: "Ahmed",
            last: "Gharbi",
            phone: "+216 24 567 890",
            address: "Rue de Marseille, Sfax",
            email: None,
        },
        SeedClient {
            first: "Salma",
            last: "Jlassi",
            phone: "+216 26 789 012",
            address: "Menzah 6, Tunis",
            email: Some("salma.jlassi@email.tn"),
        },
        SeedClient {
            first: "Youssef",
            last: "Hamdi",
            phone: "+216 28 901 234",
            address: "Médina, Sousse",
            email: Some("youssef.hamdi@email.tn"),
        },
        SeedClient {
            first: "Nour",
            last: "Khelifi",
            phone: "+216 29 012 345",
            address: "La Marsa, Tunis",
            email: Some("nour.khelifi@email.tn"),
        },
    ];

    let mut client_ids = Vec::new();
    for c in &clients {
        conn.execute(
            "INSERT INTO client (first_name, last_name, phone, address, email)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![c.first, c.last, c.phone, c.address, c.email],
        )
        .map_err(|e| e.to_string())?;
        client_ids.push(conn.last_insert_rowid());
    }

    // A spread of purchases: some overdue, some on track, one fully paid.
    let purchases = [
        SeedPurchase {
            client_idx: 0,
            product: "Réfrigérateur Samsung 260L",
            total: 2400,
            count: 6,
            months_ago: 5,
            paid: 1,
        },
        SeedPurchase {
            client_idx: 1,
            product: "Machine à laver LG 8kg",
            total: 1800,
            count: 5,
            months_ago: 4,
            paid: 2,
        },
        SeedPurchase {
            client_idx: 2,
            product: "Téléviseur Smart 55\"",
            total: 3200,
            count: 8,
            months_ago: 6,
            paid: 3,
        },
        SeedPurchase {
            client_idx: 3,
            product: "Cuisinière 4 feux",
            total: 1200,
            count: 4,
            months_ago: 4,
            paid: 4,
        },
        SeedPurchase {
            client_idx: 4,
            product: "Climatiseur 1.5 CV",
            total: 2100,
            count: 6,
            months_ago: 3,
            paid: 1,
        },
        SeedPurchase {
            client_idx: 5,
            product: "Congélateur 200L",
            total: 1500,
            count: 5,
            months_ago: 1,
            paid: 1,
        },
        SeedPurchase {
            client_idx: 0,
            product: "Four électrique",
            total: 900,
            count: 3,
            months_ago: 0,
            paid: 0,
        },
        SeedPurchase {
            client_idx: 1,
            product: "Lave-vaisselle Bosch",
            total: 1600,
            count: 4,
            months_ago: 2,
            paid: 1,
        },
    ];

    let base = today();
    for p in &purchases {
        let purchase_date = base
            .checked_sub_months(chrono::Months::new(p.months_ago as u32))
            .unwrap_or(base);
        let purchase_date_str = purchase_date.to_string();

        conn.execute(
            "INSERT INTO purchase
                (reference, client_id, product_label, total_price, installment_count,
                 interval_kind, interval_days, purchase_date)
             VALUES ('', ?1, ?2, ?3, ?4, 'monthly', NULL, ?5)",
            params![
                client_ids[p.client_idx],
                p.product,
                p.total,
                p.count,
                purchase_date_str
            ],
        )
        .map_err(|e| e.to_string())?;
        let purchase_id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE purchase SET reference = ?1 WHERE id = ?2",
            params![format!("A-{:06}", purchase_id), purchase_id],
        )
        .map_err(|e| e.to_string())?;

        let amounts = split_amounts(p.total, p.count);
        for (i, amount) in amounts.iter().enumerate() {
            let idx = (i as i64) + 1;
            let due = add_interval(purchase_date, "monthly", None, i as i64);
            let due_str = due.to_string();

            let fully_paid = idx <= p.paid;
            let (paid_amount, paid_date) = if fully_paid {
                (*amount, Some(due_str.clone()))
            } else {
                (0, None)
            };

            conn.execute(
                "INSERT INTO installment (purchase_id, idx, amount, due_date, paid_amount, paid_date)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![purchase_id, idx, amount, due_str, paid_amount, paid_date],
            )
            .map_err(|e| e.to_string())?;
            let installment_id = conn.last_insert_rowid();

            if fully_paid {
                conn.execute(
                    "INSERT INTO payment (installment_id, amount, payment_date, note)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![installment_id, amount, due_str, Option::<String>::None],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    // Default settings (French default; OS-locale detection runs on the
    // frontend while `language_is_default` is still 1).
    let defaults = [
        ("language", "fr"),
        ("language_is_default", "1"),
        ("currency_code", "TND"),
        ("date_format", "dd/MM/yyyy"),
        ("shop_name", "Électro Ménager"),
        ("shop_info", ""),
        ("logo_path", ""),
        ("alert_soon_days", "7"),
    ];
    for (k, v) in defaults {
        conn.execute(
            "INSERT OR IGNORE INTO setting (key, value) VALUES (?1, ?2)",
            params![k, v],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}
