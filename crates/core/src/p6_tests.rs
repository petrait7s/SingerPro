//! P6 test-suite: reports tie to the books, permissions, exports, dashboard.

use crate::accounting::code;
use crate::installments::InstallmentIn;
use crate::p5_tests::*;
use crate::reports::{self, Column, Params, Report};
use crate::sales::{self, SaleLineIn};
use crate::{customers, settings, time, users, CoreError};
use chrono::Duration;
use serde_json::{json, Value};
use std::collections::HashMap;

fn today() -> String {
    time::today_dhaka().to_string()
}

fn range_all() -> Params {
    Params {
        from: Some("2000-01-01".into()),
        to: Some(today()),
        as_of: None,
    }
}

fn find<'a>(r: &'a Report, label: &str) -> &'a Vec<Value> {
    r.rows
        .iter()
        .find(|row| row.iter().any(|c| c.as_str() == Some(label)))
        .unwrap_or_else(|| panic!("no row {label} in {:?}", r.rows))
}

/// A shop with a bit of everything: retail, wholesale credit, an installment sale, a full return, an expense.
async fn busy_shop() -> Env {
    let e = env().await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("sales.credit_limit_enforced".into(), "false".into())]),
    )
    .await
    .unwrap();
    sell_fridge(&e, "F-1", None, 5_200_000, "s1", None).await;
    sales::create_sale(
        &e.pool,
        &e.owner,
        si(
            "retail",
            None,
            vec![SaleLineIn {
                variant_id: e.fan.clone(),
                qty: 3,
                discount: 0,
                serial_ids: vec![],
            }],
            vec![pay("bank", 450_000)],
            "s2",
            None,
        ),
    )
    .await
    .unwrap();
    let dealer = customers::create_customer(&e.pool, &e.owner, cust("Dealer"))
        .await
        .unwrap();
    sales::create_sale(
        &e.pool,
        &e.owner,
        si(
            "wholesale",
            Some(dealer),
            vec![SaleLineIn {
                variant_id: e.fan.clone(),
                qty: 5,
                discount: 0,
                serial_ids: vec![],
            }],
            vec![],
            "s3",
            None,
        ),
    )
    .await
    .unwrap();
    let buyer = customers::create_customer(&e.pool, &e.owner, cust("Buyer"))
        .await
        .unwrap();
    let sid = serial_id(&e, "F-2").await;
    let mut inst = si(
        "installment",
        Some(buyer),
        vec![SaleLineIn {
            variant_id: e.fridge.clone(),
            qty: 1,
            discount: 0,
            serial_ids: vec![sid],
        }],
        vec![pay("cash", 1_000_000)],
        "s4",
        None,
    );
    inst.installment = Some(InstallmentIn {
        markup_bp: None,
        count: 4,
        frequency: "monthly".into(),
        first_due_date: (time::today_dhaka() + Duration::days(30)).to_string(),
        custom: vec![],
        fees: vec![],
        notes: "".into(),
    });
    sales::create_sale(&e.pool, &e.owner, inst).await.unwrap();
    // F-1 comes back and is refunded in full
    let mgr = user(&e, "mgr", "Manager").await;
    let first = sqlx::query_scalar::<_, String>("SELECT id FROM sales WHERE client_key = 's1'")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    let it = sale_items(&e, &first).await;
    let ret = crate::returns::record_return(&e.pool, &mgr, rec(&first, vec![(&it[0], 1)], "r"))
        .await
        .unwrap();
    let ri = return_items(&e, ret).await;
    crate::returns::approve_return(
        &e.pool,
        &mgr,
        ret,
        approve(dec(&[(&ri[0], "restock")]), 5_200_000, "a"),
    )
    .await
    .unwrap();
    let rent = cat_id(&e, "Rent").await;
    crate::expenses::create_expense(&e.pool, &e.owner, exp(&rent, 200_000, "cash", "x"))
        .await
        .unwrap();
    e
}

async fn run(e: &Env, id: &str, p: Params) -> Report {
    reports::run(&e.pool, &e.owner, id, p).await.unwrap()
}

#[tokio::test]
async fn sales_reports_net_off_returns_and_tie_to_each_other() {
    let e = busy_shop().await;
    let day = run(&e, "sales_by_day", range_all()).await;
    let row = find(&day, &today());
    // 3 invoices today besides the installment one -> 4 invoices; the installment sale is one more
    let installment_total: i64 =
        sqlx::query_scalar("SELECT grand_total FROM sales WHERE client_key = 's4'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(row[1], json!(4));
    assert_eq!(
        row[2],
        json!(5_200_000 + 450_000 + 650_000 + installment_total)
    );
    assert_eq!(row[3], json!(5_200_000));
    assert_eq!(row[4], json!(450_000 + 650_000 + installment_total));
    let totals = day.totals.as_ref().unwrap();
    assert_eq!(totals[4], row[4], "single day: totals equal the row");
    // the same net figure appears by month, by mode and by product
    let month = run(&e, "sales_by_month", range_all()).await;
    assert_eq!(month.totals.as_ref().unwrap()[4], totals[4]);
    let by_mode = run(&e, "sales_by_mode", range_all()).await;
    assert_eq!(by_mode.totals.as_ref().unwrap()[4], totals[4]);
    assert_eq!(
        find(&by_mode, "retail")[3],
        json!(5_200_000),
        "the return belongs to the retail sale"
    );
    assert_eq!(find(&by_mode, "wholesale")[2], json!(650_000));
    let product = run(&e, "sales_by_product", range_all()).await;
    let fan = find(&product, "Singer Fan [FAN]");
    assert_eq!(
        (&fan[1], &fan[2], &fan[3], &fan[4]),
        (
            &json!(8),
            &json!(1_100_000),
            &json!(800_000),
            &json!(300_000)
        ),
        "fans: 8 units, revenue, cost, margin"
    );
    let fridge = find(&product, "Singer Fridge [FRIDGE]");
    assert_eq!(
        fridge[1],
        json!(1),
        "one fridge sold and returned, one on installment"
    );
    // brand and category roll the same numbers up
    let brand = run(&e, "sales_by_brand", range_all()).await;
    assert_eq!(
        brand.totals.as_ref().unwrap()[2],
        product.totals.as_ref().unwrap()[2]
    );
    let cat = run(&e, "sales_by_category", range_all()).await;
    assert_eq!(
        cat.totals.as_ref().unwrap()[4],
        product.totals.as_ref().unwrap()[4]
    );
    // outside the period there is nothing
    let old = run(
        &e,
        "sales_by_day",
        Params {
            from: Some("2020-01-01".into()),
            to: Some("2020-01-31".into()),
            as_of: None,
        },
    )
    .await;
    assert!(old.rows.is_empty());
}

#[tokio::test]
async fn financial_reports_agree_with_the_ledger() {
    let e = busy_shop().await;
    let pl = run(&e, "profit_loss", range_all()).await;
    let accounts = crate::accounting::list_accounts(&e.pool, &e.owner, None)
        .await
        .unwrap();
    let income: i64 = accounts
        .iter()
        .filter(|a| a.account_type == "income")
        .map(|a| a.credit - a.debit)
        .sum();
    let expense: i64 = accounts
        .iter()
        .filter(|a| a.account_type == "expense")
        .map(|a| a.debit - a.credit)
        .sum();
    let net = pl.rows.iter().find(|r| r[1] == "NET PROFIT").unwrap()[2]
        .as_i64()
        .unwrap();
    assert_eq!(
        net,
        income - expense,
        "P&L equals income minus expense accounts"
    );
    assert!(net > 0);
    let bs = run(&e, "balance_sheet", Params::default()).await;
    let last = bs.rows.last().unwrap();
    assert_eq!(
        (last[1].as_str(), last[2].as_i64()),
        (Some("Balanced"), Some(0)),
        "{:?}",
        bs.rows
    );
    let assets = bs.rows.iter().find(|r| r[1] == "Total assets").unwrap()[2]
        .as_i64()
        .unwrap();
    let equity_side = bs
        .rows
        .iter()
        .find(|r| r[1] == "Liabilities + equity")
        .unwrap()[2]
        .as_i64()
        .unwrap();
    assert_eq!(assets, equity_side);
    // supporting reports tie to their control accounts
    let stock = run(&e, "stock_valuation", Params::default()).await;
    assert_eq!(
        stock.totals.as_ref().unwrap()[5].as_i64().unwrap(),
        bal(&e, "1200").await
    );
    let cust = run(&e, "customer_dues", Params::default()).await;
    assert_eq!(
        cust.totals.as_ref().unwrap()[3].as_i64().unwrap(),
        bal(&e, code::AR).await + bal(&e, code::INSTALLMENT_AR).await
    );
    let sup = run(&e, "supplier_dues", Params::default()).await;
    assert_eq!(
        sup.totals.as_ref().unwrap()[2].as_i64().unwrap(),
        bal(&e, code::AP).await
    );
    let cb = run(&e, "cashbook", range_all()).await;
    let closing = cb.totals.as_ref().unwrap()[6].as_i64().unwrap();
    assert_eq!(
        closing,
        bal(&e, code::CASH).await + bal(&e, code::BANK).await + bal(&e, code::WALLET).await
    );
    let ex = run(&e, "expenses_by_category", range_all()).await;
    assert_eq!(find(&ex, "Rent")[2], json!(200_000));
    assert_books(&e).await;
}

#[tokio::test]
async fn installment_and_stock_worklists() {
    let e = busy_shop().await;
    let due = run(
        &e,
        "installments_due",
        Params {
            from: Some(today()),
            to: Some((time::today_dhaka() + Duration::days(200)).to_string()),
            as_of: None,
        },
    )
    .await;
    assert_eq!(due.rows.len(), 4);
    let remaining: i64 = due.rows.iter().map(|r| r[6].as_i64().unwrap()).sum();
    assert_eq!(
        remaining, 4_200_000,
        "price 52,000 less 10,000 down payment, no markup"
    );
    assert!(
        due.rows.iter().all(|r| r[7] == json!(0)),
        "nothing is overdue yet"
    );
    let coll = run(&e, "installment_collections", range_all()).await;
    assert!(
        coll.rows.is_empty(),
        "the down payment is a sale receipt, not an installment collection"
    );
    sqlx::query("UPDATE products SET min_stock = 100 WHERE name = 'Fan'")
        .execute(&e.pool)
        .await
        .unwrap();
    let low = run(&e, "low_stock", Params::default()).await;
    let fan = find(&low, "Singer Fan");
    assert_eq!(
        (fan[2].as_i64(), fan[3].as_i64(), fan[4].as_i64()),
        (Some(100), Some(12), Some(88))
    );
    assert!(run(&e, "purchases_by_supplier", range_all())
        .await
        .rows
        .is_empty());
}

#[tokio::test]
async fn report_permissions_and_cost_visibility() {
    let e = busy_shop().await;
    let rid = users::create_role(
        &e.pool,
        &e.owner,
        "Analyst",
        "",
        vec!["reports.operational".into()],
    )
    .await
    .unwrap();
    let id = users::create_user(
        &e.pool,
        &e.owner,
        users::NewUser {
            username: "analyst".into(),
            full_name: "Analyst".into(),
            password: "Correct-horse-battery-9".into(),
            role_ids: vec![rid.to_string()],
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET must_change_password = 0 WHERE id = ?")
        .bind(id.to_string())
        .execute(&e.pool)
        .await
        .unwrap();
    let analyst = crate::authz::load_principal(&e.pool, id).await.unwrap();
    for fin in ["profit_loss", "balance_sheet", "cashbook"] {
        assert!(
            forbidden(reports::run(&e.pool, &analyst, fin, Params::default()).await),
            "{fin}"
        );
    }
    let r = reports::run(&e.pool, &analyst, "sales_by_product", range_all())
        .await
        .unwrap();
    let keys: Vec<&str> = r.columns.iter().map(|c| c.key).collect();
    assert_eq!(
        keys,
        vec!["name", "qty", "revenue"],
        "cost and margin are hidden"
    );
    assert!(r.rows.iter().all(|row| row.len() == 3) && r.totals.as_ref().unwrap().len() == 3);
    let ids: Vec<&str> = reports::catalogue(&analyst).iter().map(|d| d.id).collect();
    assert!(ids.contains(&"sales_by_day") && !ids.contains(&"profit_loss"));
    let nobody = user(&e, "cash", "Cashier").await;
    assert!(forbidden(
        reports::run(&e.pool, &nobody, "sales_by_day", Params::default()).await
    ));
    assert!(reports::catalogue(&nobody).is_empty());
    assert!(forbidden(
        reports::export(&e.pool, &nobody, "sales_by_day", Params::default(), "csv").await
    ));
    assert!(matches!(
        reports::run(&e.pool, &e.owner, "no_such", Params::default()).await,
        Err(CoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn report_periods_are_validated() {
    let e = env().await;
    for (from, to) in [
        ("2026-13-01", "2026-12-01"),
        ("2026-02-01", "2026-01-01"),
        ("garbage", "2026-01-01"),
    ] {
        let r = reports::run(
            &e.pool,
            &e.owner,
            "sales_by_day",
            Params {
                from: Some(from.into()),
                to: Some(to.into()),
                as_of: None,
            },
        )
        .await;
        assert!(matches!(r, Err(CoreError::Validation(_))), "{from}..{to}");
    }
    let r = reports::run(
        &e.pool,
        &e.owner,
        "balance_sheet",
        Params {
            as_of: Some("nope".into()),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(r, Err(CoreError::Validation(_))));
}

#[tokio::test]
async fn csv_is_exact_quoted_and_safe_from_formula_injection() {
    assert_eq!(reports::plain_taka(12_345_600), "123456.00");
    assert_eq!(reports::plain_taka(5), "0.05");
    assert_eq!(reports::plain_taka(-5), "-0.05");
    assert_eq!(reports::plain_taka(-100_050), "-1000.50");
    let r = Report {
        id: "t".into(),
        title: "Test, \"quoted\"".into(),
        period: "p".into(),
        columns: vec![
            Column {
                key: "n",
                label: "Name",
                kind: "text",
            },
            Column {
                key: "m",
                label: "Amount",
                kind: "money",
            },
            Column {
                key: "q",
                label: "Qty",
                kind: "int",
            },
        ],
        rows: vec![
            vec![
                json!("=HYPERLINK(\"http://evil\")"),
                json!(12_345),
                json!(3),
            ],
            vec![json!("-5 apples"), json!(-5), json!(1)],
            vec![json!("line\nbreak, comma"), json!(0), json!(0)],
            vec![json!("কালার টিভি"), json!(100), json!(2)],
            vec![json!("-12"), json!(1), json!(1)],
        ],
        totals: Some(vec![json!("Total"), json!(12_440), json!(7)]),
    };
    let csv = reports::to_csv(&r);
    assert!(csv.starts_with('\u{feff}'), "BOM for Excel");
    let lines: Vec<&str> = csv.split("\r\n").collect();
    assert_eq!(lines[0], "\u{feff}\"Test, \"\"quoted\"\"\",p");
    assert_eq!(lines[1], "Name,Amount,Qty");
    assert_eq!(lines[2], "\"'=HYPERLINK(\"\"http://evil\"\")\",123.45,3");
    assert_eq!(
        lines[3], "'-5 apples,-0.05,1",
        "text that starts with a sign is neutralised"
    );
    assert!(csv.contains("\"line\nbreak, comma\",0.00,0"));
    assert!(csv.contains("কালার টিভি,1.00,2"));
    assert!(
        csv.contains("\r\n-12,0.01,1\r\n"),
        "a plain number stays a number"
    );
    assert!(csv.trim_end().ends_with("Total,124.40,7"));
}

#[tokio::test]
async fn exports_are_audited_and_xlsx_is_a_real_workbook() {
    let e = busy_shop().await;
    let (name, bytes) = reports::export(&e.pool, &e.owner, "sales_by_mode", range_all(), "xlsx")
        .await
        .unwrap();
    assert!(
        name.starts_with("sales_by_mode_") && name.ends_with(".xlsx"),
        "{name}"
    );
    assert_eq!(&bytes[..2], b"PK", "xlsx is a zip container");
    assert!(bytes.len() > 1000);
    let (cname, csv) = reports::export(&e.pool, &e.owner, "sales_by_day", range_all(), "csv")
        .await
        .unwrap();
    assert!(cname.ends_with(".csv") && String::from_utf8(csv).unwrap().contains(&today()));
    assert_eq!(audit_count(&e.pool, "report.exported").await, 2);
    assert!(matches!(
        reports::export(&e.pool, &e.owner, "sales_by_day", range_all(), "pdf").await,
        Err(CoreError::Validation(_))
    ));
    assert_eq!(
        audit_count(&e.pool, "report.exported").await,
        2,
        "a refused export is not logged as done"
    );
}

#[tokio::test]
async fn dashboard_figures_come_from_the_books_and_respect_permissions() {
    let e = busy_shop().await;
    let d = reports::dashboard(&e.pool, &e.owner).await.unwrap();
    let s = d.sales.as_ref().unwrap();
    let day = run(&e, "sales_by_day", range_all()).await;
    assert_eq!(s.today_net, find(&day, &today())[4].as_i64().unwrap());
    assert_eq!((s.today_invoices, s.month_net), (4, s.today_net));
    let m = d.money.as_ref().unwrap();
    assert_eq!(
        m.cash_and_bank,
        bal(&e, code::CASH).await + bal(&e, code::BANK).await + bal(&e, code::WALLET).await
    );
    assert_eq!(
        m.receivables,
        bal(&e, code::AR).await + bal(&e, code::INSTALLMENT_AR).await
    );
    assert_eq!(m.payables, bal(&e, code::AP).await);
    assert_eq!(d.stock.as_ref().unwrap().value, bal(&e, "1200").await);
    let i = d.installments.as_ref().unwrap();
    assert_eq!((i.overdue_count, i.overdue_amount), (0, 0));
    assert_eq!(d.last_days.len(), 14);
    assert_eq!(d.last_days.last().unwrap(), &(today(), s.today_net));
    assert!(d.last_days[..13].iter().all(|x| x.1 == 0));
    assert_eq!(
        (
            d.pending.returns_awaiting_inspection,
            d.pending.expenses_awaiting_approval,
            d.pending.open_warranty_cases
        ),
        (Some(0), Some(0), Some(0))
    );
    // a user who only holds sales.view sees sales but no money, stock or approval queues
    let rid = users::create_role(&e.pool, &e.owner, "Viewer", "", vec!["sales.view".into()])
        .await
        .unwrap();
    let id = users::create_user(
        &e.pool,
        &e.owner,
        users::NewUser {
            username: "viewer".into(),
            full_name: "Viewer".into(),
            password: "Correct-horse-battery-9".into(),
            role_ids: vec![rid.to_string()],
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET must_change_password = 0 WHERE id = ?")
        .bind(id.to_string())
        .execute(&e.pool)
        .await
        .unwrap();
    let viewer = crate::authz::load_principal(&e.pool, id).await.unwrap();
    let c = reports::dashboard(&e.pool, &viewer).await.unwrap();
    assert!(
        c.sales.is_some() && c.money.is_none() && c.stock.is_none() && c.installments.is_none()
    );
    assert!(
        c.pending.returns_awaiting_inspection.is_none()
            && c.pending.expenses_awaiting_approval.is_none()
    );
}

#[tokio::test]
async fn product_import_validates_everything_first_and_never_overwrites() {
    let e = env().await;
    let tpl = crate::import::template("products").unwrap();
    assert!(tpl.starts_with('\u{feff}') && tpl.contains("purchase_cost"));
    // the template's own sample row is a valid import
    let ok = crate::import::import_csv(&e.pool, &e.owner, "products", &tpl, false)
        .await
        .unwrap();
    assert!(ok.errors.is_empty(), "{:?}", ok.errors);
    let header = tpl
        .lines()
        .next()
        .unwrap()
        .trim_start_matches('\u{feff}')
        .to_string();
    let good = format!(
        "{header}\r\nWalton,Televisions,Smart TV,W-43,TV-43,,Black,43in,,30000,42000.50,40000,,24,Panel only,1,yes\r\nWalton,Televisions,Smart TV,W-43,TV-43-W,,White,43in,,30000,43000,,,24,Panel only,1,yes\r\nSinger,Fans,\"Table, fan\",,FAN-T,,,,,900,1500,1300,,0,,5,no\r\n"
    );
    // a dry run writes nothing
    let dry = crate::import::import_csv(&e.pool, &e.owner, "products", &good, false)
        .await
        .unwrap();
    assert_eq!((dry.rows, dry.errors.len(), dry.imported), (3, 0, false));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_variants WHERE sku LIKE 'TV-%'")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let done = crate::import::import_csv(&e.pool, &e.owner, "products", &good, true)
        .await
        .unwrap();
    assert_eq!(
        (done.imported, done.created),
        (true, 2),
        "two products: a TV with two variants and a fan"
    );
    let tv: (i64, i64, i64, i64) = sqlx::query_as("SELECT COUNT(*), MIN(v.retail_price), MAX(v.retail_price), MIN(pr.is_serialized) FROM product_variants v JOIN products pr ON pr.id = v.product_id WHERE pr.name = 'Smart TV'").fetch_one(&e.pool).await.unwrap();
    assert_eq!(
        tv,
        (2, 4_200_050, 4_300_000, 1),
        "exact paisa, one product with two variants"
    );
    let wholesale: i64 =
        sqlx::query_scalar("SELECT wholesale_price FROM product_variants WHERE sku = 'TV-43-W'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(
        wholesale, 4_300_000,
        "blank wholesale price defaults to retail"
    );
    let brands: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM brands WHERE name = 'Walton'")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(brands, 1);
    assert_eq!(audit_count(&e.pool, "import.completed").await, 1);
    // importing the same file again changes nothing and says why
    let again = crate::import::import_csv(&e.pool, &e.owner, "products", &good, true)
        .await
        .unwrap();
    assert!(
        !again.imported
            && again.errors.len() == 3
            && again.errors[0].message.contains("already exists")
    );
    // one bad row blocks the whole file
    let bad = format!("{header}\r\nA,B,Good,,OK-1,,,,,100,200,,,0,,0,no\r\nA,B,Bad,,OK-2,,,,,abc,-5,,,x,,0,maybe\r\nA,B,Dup,,OK-1,,,,,1,2,,,0,,0,no\r\n");
    let r = crate::import::import_csv(&e.pool, &e.owner, "products", &bad, true)
        .await
        .unwrap();
    assert!(!r.imported);
    assert_eq!(
        r.errors.iter().map(|x| x.line).collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert!(
        r.errors[0].message.contains("purchase cost") && r.errors[0].message.contains("serialized")
    );
    assert!(r.errors[1].message.contains("twice"));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_variants WHERE sku LIKE 'OK-%'")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "the valid row was not written either");
    // wrong headers and permissions
    let wrong = crate::import::import_csv(&e.pool, &e.owner, "products", "a,b\r\n1,2\r\n", false)
        .await
        .unwrap();
    assert!(wrong.errors[0].message.contains("missing column"));
    assert!(
        crate::import::import_csv(&e.pool, &e.owner, "products", "", false)
            .await
            .is_err()
    );
    assert!(
        crate::import::import_csv(&e.pool, &e.owner, "orders", "x", false)
            .await
            .is_err()
    );
    let viewer = user(&e, "cash", "Cashier").await;
    assert!(forbidden(
        crate::import::import_csv(&e.pool, &viewer, "products", &good, true).await
    ));
}

#[tokio::test]
async fn customer_import_rejects_duplicates_and_keeps_quoted_fields() {
    let e = env().await;
    customers::create_customer(&e.pool, &e.owner, cust("Existing"))
        .await
        .unwrap();
    let tpl = crate::import::template("customers").unwrap();
    let header = tpl
        .lines()
        .next()
        .unwrap()
        .trim_start_matches('\u{feff}')
        .to_string();
    let csv = format!(
        "{header}\r\nKarim Uddin,01711000000,,\"House 5,\r\nRoad 3, \"\"Sylhet\"\"\",wholesale,50000.50,vip\r\nRahim,01722000000,,,,,\r\n"
    );
    let r = crate::import::import_csv(&e.pool, &e.owner, "customers", &csv, true)
        .await
        .unwrap();
    assert_eq!((r.imported, r.created), (true, 2));
    let (addr, ty, limit): (String, String, i64) = sqlx::query_as(
        "SELECT address, customer_type, credit_limit FROM customers WHERE name = 'Karim Uddin'",
    )
    .fetch_one(&e.pool)
    .await
    .unwrap();
    assert_eq!(addr, "House 5,  Road 3, \"Sylhet\"");
    assert_eq!((ty.as_str(), limit), ("wholesale", 5_000_050));
    let dup = format!("{header}\r\nNew One,01711000000,,,,,\r\nTwin,01733,,,,,\r\nTwin2,01733,,,,,\r\nOdd,0199,,,platinum,-1,\r\n,01800,,,,,\r\n");
    let r = crate::import::import_csv(&e.pool, &e.owner, "customers", &dup, true)
        .await
        .unwrap();
    assert!(!r.imported);
    assert_eq!(
        r.errors.iter().map(|x| x.line).collect::<Vec<_>>(),
        vec![2, 4, 5, 6]
    );
    assert!(r.errors[0].message.contains("already belongs"));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 3, "nothing from the rejected file was added");
}
