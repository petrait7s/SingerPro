//! P2 test-suite: catalogue, suppliers, purchasing, stock ledger, serial lifecycle, reconciliation.

use crate::auth::{self, SessionStore};
use crate::authz::{self, Principal};
use crate::catalogue::{self, NewProduct, NewVariant, ProductFilter};
use crate::inventory::{self, AdjustIn, MoveIn, OpeningStockIn, SerialFilter, StockFilter};
use crate::purchases::{
    self, CostIn, LineIn, PaymentIn, PurchaseFilter, PurchaseIn, ReceiveIn, ReceiveLine, ReturnIn,
    ReturnLine,
};
use crate::recovery::DataPaths;
use crate::stock::{self, SerialStatus};
use crate::suppliers::{self, SupplierIn};
use crate::users::{self, NewUser};
use crate::{db, CoreError, Id};
use sqlx::SqlitePool;

const PW: &str = "Correct-horse-battery-9";

struct Env {
    pool: SqlitePool,
    owner: Principal,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let paths = DataPaths::new(dir.path());
    let pool = db::open(&paths.db).await.unwrap();
    let id = auth::setup_owner(&pool, "owner", "Owner", PW)
        .await
        .unwrap();
    let owner = authz::load_principal(&pool, id).await.unwrap();
    Env {
        pool,
        owner,
        _dir: dir,
    }
}

async fn user_with_role(e: &Env, name: &str, role: &str) -> Principal {
    let rid: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
        .bind(role)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    let id = users::create_user(
        &e.pool,
        &e.owner,
        NewUser {
            username: name.into(),
            full_name: name.into(),
            password: PW.into(),
            role_ids: vec![rid],
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE users SET must_change_password = 0 WHERE id = ?")
        .bind(id.to_string())
        .execute(&e.pool)
        .await
        .unwrap();
    authz::load_principal(&e.pool, id).await.unwrap()
}

fn forbidden<T: std::fmt::Debug>(r: Result<T, CoreError>) -> bool {
    matches!(r, Err(CoreError::Forbidden(_)))
}

fn vin(sku: &str, cost: i64) -> NewVariant {
    NewVariant {
        sku: sku.into(),
        barcode: None,
        colour: "".into(),
        capacity: "".into(),
        size: "".into(),
        purchase_cost: cost,
        retail_price: cost * 13 / 10,
        wholesale_price: cost * 12 / 10,
        installment_price: Some(cost * 15 / 10),
    }
}

struct Cat {
    brand: Id,
    category: Id,
}

async fn cat(e: &Env) -> Cat {
    let brand = catalogue::create_brand(&e.pool, &e.owner, "Singer")
        .await
        .unwrap();
    let category = catalogue::create_category(&e.pool, &e.owner, "Refrigerators", None)
        .await
        .unwrap();
    Cat { brand, category }
}

async fn product(
    e: &Env,
    c: &Cat,
    name: &str,
    serialized: bool,
    skus: &[(&str, i64)],
) -> (Id, Vec<String>) {
    let id = catalogue::create_product(
        &e.pool,
        &e.owner,
        NewProduct {
            brand_id: c.brand.to_string(),
            category_id: c.category.to_string(),
            name: name.into(),
            model: "M1".into(),
            description: "".into(),
            warranty_months: 12,
            warranty_terms: "".into(),
            min_stock: 0,
            is_serialized: serialized,
            variants: skus.iter().map(|(s, c)| vin(s, *c)).collect(),
        },
    )
    .await
    .unwrap();
    let p = catalogue::get_product(&e.pool, &e.owner, id).await.unwrap();
    (id, p.variants.into_iter().map(|v| v.id).collect())
}

async fn supplier(e: &Env, name: &str) -> Id {
    suppliers::create_supplier(
        &e.pool,
        &e.owner,
        SupplierIn {
            name: name.into(),
            contact_person: "".into(),
            phone: "".into(),
            alt_phone: "".into(),
            email: "".into(),
            address: "".into(),
            notes: "".into(),
        },
    )
    .await
    .unwrap()
}

fn pin(
    sup: Id,
    inv: &str,
    items: Vec<(&str, i64, i64)>,
    costs: Vec<(&str, i64)>,
    discount: i64,
) -> PurchaseIn {
    PurchaseIn {
        supplier_id: sup.to_string(),
        supplier_invoice_no: Some(inv.into()),
        order_date: "2026-03-01".into(),
        discount,
        notes: "".into(),
        items: items
            .into_iter()
            .map(|(v, q, c)| LineIn {
                variant_id: v.into(),
                qty: q,
                unit_cost: c,
            })
            .collect(),
        costs: costs
            .into_iter()
            .map(|(l, a)| CostIn {
                label: l.into(),
                amount: a,
            })
            .collect(),
    }
}

async fn audit_count(pool: &SqlitePool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = ?")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn avail(pool: &SqlitePool, variant: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE((SELECT qty FROM stock_balances WHERE variant_id = ? AND bucket = 'available'), 0)").bind(variant).fetch_one(pool).await.unwrap()
}

async fn assert_reconciled(e: &Env) {
    let d = stock::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(d.is_empty(), "stock must reconcile: {d:?}");
}

fn recv(item: &str, qty: i64, serials: &[&str]) -> ReceiveLine {
    ReceiveLine {
        item_id: item.into(),
        qty,
        serials: serials.iter().map(|s| s.to_string()).collect(),
    }
}

fn rin(lines: Vec<ReceiveLine>) -> ReceiveIn {
    ReceiveIn {
        received_on: "2026-03-05".into(),
        note: "".into(),
        lines,
    }
}

async fn confirmed_purchase(
    e: &Env,
    sup: Id,
    inv: &str,
    items: Vec<(&str, i64, i64)>,
    costs: Vec<(&str, i64)>,
    discount: i64,
) -> Id {
    let id = purchases::create_draft(&e.pool, &e.owner, pin(sup, inv, items, costs, discount))
        .await
        .unwrap();
    purchases::confirm(&e.pool, &e.owner, id).await.unwrap();
    id
}

// ------------------------------------------------------------------ catalogue

#[tokio::test]
async fn brands_categories_and_products_crud_with_validation() {
    let e = env().await;
    let c = cat(&e).await;
    assert!(
        matches!(
            catalogue::create_brand(&e.pool, &e.owner, "singer").await,
            Err(CoreError::Conflict(_))
        ),
        "case-insensitive duplicate"
    );
    assert!(catalogue::create_brand(&e.pool, &e.owner, "  ")
        .await
        .is_err());
    let sub = catalogue::create_category(&e.pool, &e.owner, "Double door", Some(c.category))
        .await
        .unwrap();
    assert!(
        catalogue::create_category(&e.pool, &e.owner, "x", Some(sub))
            .await
            .is_err(),
        "two levels only"
    );
    assert!(
        catalogue::create_category(&e.pool, &e.owner, "Double door", Some(c.category))
            .await
            .is_err()
    );
    assert!(
        catalogue::create_category(&e.pool, &e.owner, "Double door", None)
            .await
            .is_ok(),
        "same name allowed at another level"
    );

    let (pid, v) = product(
        &e,
        &c,
        "Frost Free 300L",
        true,
        &[("FR-300-W", 4_000_000), ("FR-300-B", 4_100_000)],
    )
    .await;
    let p = catalogue::get_product(&e.pool, &e.owner, pid)
        .await
        .unwrap();
    assert_eq!(p.variants.len(), 2);
    assert!(p.is_serialized && p.brand == "Singer");

    // validation
    let np = |name: &str, variants: Vec<NewVariant>| NewProduct {
        brand_id: c.brand.to_string(),
        category_id: c.category.to_string(),
        name: name.into(),
        model: "".into(),
        description: "".into(),
        warranty_months: 0,
        warranty_terms: "".into(),
        min_stock: 0,
        is_serialized: false,
        variants,
    };
    assert!(
        catalogue::create_product(&e.pool, &e.owner, np("", vec![vin("A", 1)]))
            .await
            .is_err(),
        "name required"
    );
    assert!(
        catalogue::create_product(&e.pool, &e.owner, np("X", vec![]))
            .await
            .is_err(),
        "variant required"
    );
    assert!(
        catalogue::create_product(&e.pool, &e.owner, np("X", vec![vin("FR-300-W", 1)]))
            .await
            .is_err(),
        "duplicate SKU (case-insens.)"
    );
    assert!(
        catalogue::create_product(&e.pool, &e.owner, np("X", vec![vin("fr-300-w", 1)]))
            .await
            .is_err()
    );
    let mut neg = vin("NEG", 1);
    neg.retail_price = -5;
    assert!(
        catalogue::create_product(&e.pool, &e.owner, np("X", vec![neg]))
            .await
            .is_err()
    );
    // a failed create leaves nothing behind
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM products")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // barcode: empty -> NULL (so many products may have none), non-empty must be unique
    let mut b1 = vin("B1", 1);
    b1.barcode = Some("  ".into());
    let mut b2 = vin("B2", 1);
    b2.barcode = Some("".into());
    catalogue::create_product(&e.pool, &e.owner, np("Y", vec![b1, b2]))
        .await
        .unwrap();
    let mut b3 = vin("B3", 1);
    b3.barcode = Some("8901234567890".into());
    catalogue::create_product(&e.pool, &e.owner, np("Z", vec![b3.clone()]))
        .await
        .unwrap();
    b3.sku = "B4".into();
    assert!(matches!(
        catalogue::create_product(&e.pool, &e.owner, np("Z2", vec![b3])).await,
        Err(CoreError::Conflict(_))
    ));
    let _ = v;
}

#[tokio::test]
async fn price_changes_are_audited_with_before_and_after() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, v) = product(&e, &c, "Fan", false, &[("FAN-1", 100_000)]).await;
    let p = catalogue::get_product(&e.pool, &e.owner, product_id_of(&e, &v[0]).await)
        .await
        .unwrap();
    let cur = &p.variants[0];
    catalogue::update_variant(
        &e.pool,
        &e.owner,
        catalogue::UpdateVariant {
            id: cur.id.clone(),
            sku: cur.sku.clone(),
            barcode: None,
            colour: "".into(),
            capacity: "".into(),
            size: "".into(),
            purchase_cost: cur.purchase_cost,
            retail_price: 150_000,
            wholesale_price: cur.wholesale_price,
            installment_price: None,
        },
    )
    .await
    .unwrap();
    let d: String =
        sqlx::query_scalar("SELECT detail FROM audit_logs WHERE action='variant.prices_changed'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert!(
        d.contains(r#""retail_price":{"after":150000,"before":130000}"#),
        "{d}"
    );
    // a non-price edit uses a different action
    catalogue::update_variant(
        &e.pool,
        &e.owner,
        catalogue::UpdateVariant {
            id: cur.id.clone(),
            sku: cur.sku.clone(),
            barcode: None,
            colour: "Red".into(),
            capacity: "".into(),
            size: "".into(),
            purchase_cost: cur.purchase_cost,
            retail_price: 150_000,
            wholesale_price: cur.wholesale_price,
            installment_price: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(audit_count(&e.pool, "variant.updated").await, 1);
}

async fn product_id_of(e: &Env, variant: &str) -> Id {
    Id::parse(
        &sqlx::query_scalar::<_, String>("SELECT product_id FROM product_variants WHERE id = ?")
            .bind(variant)
            .fetch_one(&e.pool)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn product_search_filters_and_picker() {
    let e = env().await;
    let c = cat(&e).await;
    let b2 = catalogue::create_brand(&e.pool, &e.owner, "Beko")
        .await
        .unwrap();
    let mut v = vin("AC-12K", 5_000_000);
    v.barcode = Some("123456".into());
    catalogue::create_product(
        &e.pool,
        &e.owner,
        NewProduct {
            brand_id: b2.to_string(),
            category_id: c.category.to_string(),
            name: "Split AC 1.5T".into(),
            model: "BK-12".into(),
            description: "".into(),
            warranty_months: 24,
            warranty_terms: "".into(),
            min_stock: 2,
            is_serialized: true,
            variants: vec![v],
        },
    )
    .await
    .unwrap();
    product(&e, &c, "Frost Free", false, &[("FR-1", 1_000_000)]).await;
    let all = catalogue::list_products(&e.pool, &e.owner, ProductFilter::default())
        .await
        .unwrap();
    assert_eq!(all.total, 2);
    let r = catalogue::list_products(
        &e.pool,
        &e.owner,
        ProductFilter {
            search: Some("beko".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(r.total, 1);
    let r = catalogue::list_products(
        &e.pool,
        &e.owner,
        ProductFilter {
            search: Some("123456".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(r.rows[0].name, "Split AC 1.5T");
    // LIKE wildcards in user input are literal
    let r = catalogue::list_products(
        &e.pool,
        &e.owner,
        ProductFilter {
            search: Some("%".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(r.total, 0);
    let low = catalogue::list_products(
        &e.pool,
        &e.owner,
        ProductFilter {
            low_stock_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(low.total, 1, "min_stock 2 with 0 on hand is low");
    assert!(low.rows[0].low_stock);
    // picker: exact barcode first
    let pick = catalogue::search_variants(&e.pool, &e.owner, "123456", 5)
        .await
        .unwrap();
    assert_eq!(pick[0].sku, "AC-12K");
    assert!(pick[0].is_serialized && pick[0].label.starts_with("Beko Split AC"));
    assert!(catalogue::search_variants(&e.pool, &e.owner, "  ", 5)
        .await
        .unwrap()
        .is_empty());
    // inactive products leave pickers and default lists
    let pid = Id::parse(&all.rows[0].id).unwrap();
    catalogue::set_product_active(&e.pool, &e.owner, pid, false)
        .await
        .unwrap();
    assert_eq!(
        catalogue::list_products(&e.pool, &e.owner, ProductFilter::default())
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        catalogue::list_products(
            &e.pool,
            &e.owner,
            ProductFilter {
                include_inactive: true,
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .total,
        2
    );
}

#[tokio::test]
async fn product_image_validation() {
    let e = env().await;
    let c = cat(&e).await;
    let (pid, _) = product(&e, &c, "TV", false, &[("TV-1", 1)]).await;
    let png = [
        &[0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        &[0u8; 8][..],
    ]
    .concat();
    catalogue::set_product_image(&e.pool, &e.owner, pid, &png)
        .await
        .unwrap();
    assert_eq!(
        catalogue::get_product_image(&e.pool, &e.owner, pid)
            .await
            .unwrap()
            .unwrap()
            .0,
        "image/png"
    );
    assert!(
        catalogue::set_product_image(&e.pool, &e.owner, pid, b"<svg/>")
            .await
            .is_err()
    );
    assert!(
        catalogue::set_product_image(&e.pool, &e.owner, Id::new(), &png)
            .await
            .is_err()
    );
}

// ------------------------------------------------------------------ purchasing

#[tokio::test]
async fn purchase_to_stock_full_flow_with_exact_landed_cost() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, fans) = product(&e, &c, "Fan", false, &[("FAN-1", 100_001)]).await;
    let (_, fridges) = product(&e, &c, "Fridge", true, &[("FR-1", 5_000_003)]).await;
    let sup = supplier(&e, "Dhaka Traders").await;

    // 7 fans @ 1000.01 and 3 fridges @ 50000.03, freight 1234.57 + unloading 100.00, discount 500.01
    let po = purchases::create_draft(
        &e.pool,
        &e.owner,
        pin(
            sup,
            "INV-77",
            vec![(&fans[0], 7, 100_001), (&fridges[0], 3, 5_000_003)],
            vec![("Freight", 123_457), ("Unloading", 10_000)],
            50_001,
        ),
    )
    .await
    .unwrap();
    let d = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    assert_eq!(d.purchase.status, "draft");
    assert_eq!(d.subtotal, 7 * 100_001 + 3 * 5_000_003);
    assert_eq!(
        d.purchase.grand_total,
        d.subtotal - 50_001,
        "payable excludes landed costs"
    );
    assert_eq!(d.purchase.number, "PO-000001");
    // drafts do not touch stock or the ledger
    assert_eq!(avail(&e.pool, &fans[0]).await, 0);
    assert_eq!(suppliers::balance(&e.pool, sup).await.unwrap(), 0);

    purchases::confirm(&e.pool, &e.owner, po).await.unwrap();
    assert_eq!(
        suppliers::balance(&e.pool, sup).await.unwrap(),
        d.purchase.grand_total,
        "payable booked at confirmation"
    );
    let d = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    // allocation: whole-purchase inventory cost = grand_total + landed_total, exactly
    let inventory_cost: i64 = d.items.iter().map(|i| i.line_total).sum();
    assert_eq!(inventory_cost, d.purchase.grand_total + 123_457 + 10_000);
    assert!(
        matches!(
            purchases::confirm(&e.pool, &e.owner, po).await,
            Err(CoreError::Conflict(_))
        ),
        "cannot confirm twice"
    );
    assert!(matches!(
        purchases::update_draft(
            &e.pool,
            &e.owner,
            po,
            pin(sup, "X", vec![(&fans[0], 1, 1)], vec![], 0)
        )
        .await,
        Err(CoreError::Conflict(_))
    ));

    // partial receipt: 4 fans, 1 fridge
    let fan_item = d
        .items
        .iter()
        .find(|i| !i.is_serialized)
        .unwrap()
        .id
        .clone();
    let fr_item = d.items.iter().find(|i| i.is_serialized).unwrap().id.clone();
    purchases::receive(
        &e.pool,
        &e.owner,
        po,
        rin(vec![recv(&fan_item, 4, &[]), recv(&fr_item, 1, &["SN-A1"])]),
    )
    .await
    .unwrap();
    assert_eq!(avail(&e.pool, &fans[0]).await, 4);
    assert_eq!(avail(&e.pool, &fridges[0]).await, 1);
    assert_reconciled(&e).await;
    // finish
    purchases::receive(
        &e.pool,
        &e.owner,
        po,
        rin(vec![
            recv(&fan_item, 3, &[]),
            recv(&fr_item, 2, &["SN-A2", "SN-A3"]),
        ]),
    )
    .await
    .unwrap();
    assert_eq!(avail(&e.pool, &fans[0]).await, 7);
    assert_eq!(avail(&e.pool, &fridges[0]).await, 3);
    // received unit costs add up EXACTLY to the allocated line totals (no paisa lost to rounding)
    let lv: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(value),0) FROM stock_balances")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(lv, inventory_cost);
    let serial_costs: i64 = sqlx::query_scalar("SELECT SUM(cost) FROM product_serials")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    let fr_line = d.items.iter().find(|i| i.is_serialized).unwrap().line_total;
    assert_eq!(serial_costs, fr_line);
    assert_reconciled(&e).await;
    // over-receiving is refused
    assert!(matches!(
        purchases::receive(&e.pool, &e.owner, po, rin(vec![recv(&fan_item, 1, &[])])).await,
        Err(CoreError::Conflict(_))
    ));
    let d = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    assert_eq!(d.purchase.received_qty, 10);
    assert_eq!(d.receipts.len(), 2);
    assert_eq!(audit_count(&e.pool, "purchase.received").await, 2);
}

#[tokio::test]
async fn receiving_is_atomic_and_validates_serials() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, fans) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let (_, fr) = product(&e, &c, "Fridge", true, &[("FR-1", 5000)]).await;
    let sup = supplier(&e, "S").await;
    let po = confirmed_purchase(
        &e,
        sup,
        "I1",
        vec![(&fans[0], 5, 1000), (&fr[0], 2, 5000)],
        vec![],
        0,
    )
    .await;
    let d = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    let fan = d
        .items
        .iter()
        .find(|i| !i.is_serialized)
        .unwrap()
        .id
        .clone();
    let fri = d.items.iter().find(|i| i.is_serialized).unwrap().id.clone();
    // second line invalid -> the valid first line must not be received either
    let bad = [
        rin(vec![recv(&fan, 2, &[]), recv(&fri, 2, &["ONLY-ONE"])]), // serial count mismatch
        rin(vec![recv(&fan, 2, &[]), recv(&fri, 2, &["DUP", "dup"])]), // duplicate serials
        rin(vec![recv(&fan, 2, &["S1", "S2"])]), // serials on a non-serialized item
        rin(vec![recv(&fan, 9, &[])]),           // over-receive
        rin(vec![recv(&fan, -1, &[])]),          // negative
        rin(vec![recv("not-an-item", 1, &[])]),  // foreign item
        rin(vec![]),                             // nothing
    ];
    for r in bad {
        assert!(purchases::receive(&e.pool, &e.owner, po, r).await.is_err());
    }
    assert_eq!(avail(&e.pool, &fans[0]).await, 0);
    let rc: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM purchase_receipts")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(rc, 0, "failed receipts leave no trace");
    assert_reconciled(&e).await;
    // serial numbers are unique per product across purchases
    purchases::receive(
        &e.pool,
        &e.owner,
        po,
        rin(vec![recv(&fri, 2, &["SN-1", "SN-2"])]),
    )
    .await
    .unwrap();
    let po2 = confirmed_purchase(&e, sup, "I2", vec![(&fr[0], 1, 5000)], vec![], 0).await;
    let it2 = purchases::get_purchase(&e.pool, &e.owner, po2)
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    let r = purchases::receive(&e.pool, &e.owner, po2, rin(vec![recv(&it2, 1, &["sn-1"])])).await;
    assert!(matches!(r, Err(CoreError::Conflict(_))), "{r:?}");
    assert_eq!(avail(&e.pool, &fr[0]).await, 2);
    assert_reconciled(&e).await;
}

#[tokio::test]
async fn draft_rules_duplicate_invoice_and_cancellation() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, v) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let sup = supplier(&e, "S").await;
    let sup2 = supplier(&e, "S2").await;
    let a = purchases::create_draft(
        &e.pool,
        &e.owner,
        pin(sup, "INV-1", vec![(&v[0], 1, 1000)], vec![], 0),
    )
    .await
    .unwrap();
    // duplicate supplier invoice number is blocked per supplier (case-insensitive), allowed for another supplier
    assert!(matches!(
        purchases::create_draft(
            &e.pool,
            &e.owner,
            pin(sup, "inv-1", vec![(&v[0], 1, 1000)], vec![], 0)
        )
        .await,
        Err(CoreError::Conflict(_))
    ));
    purchases::create_draft(
        &e.pool,
        &e.owner,
        pin(sup2, "INV-1", vec![(&v[0], 1, 1000)], vec![], 0),
    )
    .await
    .unwrap();
    // validation
    for bad in [
        pin(sup, "N1", vec![], vec![], 0),
        pin(sup, "N2", vec![(&v[0], 0, 1000)], vec![], 0),
        pin(sup, "N3", vec![(&v[0], 1, -1)], vec![], 0),
        pin(
            sup,
            "N4",
            vec![(&v[0], 1, 1000), (&v[0], 2, 1000)],
            vec![],
            0,
        ),
        pin(sup, "N5", vec![(&v[0], 1, 1000)], vec![], 1001),
        pin(sup, "N6", vec![(&v[0], 1, 1000)], vec![("", 5)], 0),
        pin(sup, "N7", vec![(&v[0], 1, 1000)], vec![("x", 0)], 0),
        pin(sup, "N8", vec![("ghost", 1, 1000)], vec![], 0),
    ] {
        assert!(purchases::create_draft(&e.pool, &e.owner, bad)
            .await
            .is_err());
    }
    let mut bad_date = pin(sup, "N9", vec![(&v[0], 1, 1000)], vec![], 0);
    bad_date.order_date = "31/01/2026".into();
    assert!(purchases::create_draft(&e.pool, &e.owner, bad_date)
        .await
        .is_err());
    // edit draft
    purchases::update_draft(
        &e.pool,
        &e.owner,
        a,
        pin(
            sup,
            "INV-1",
            vec![(&v[0], 3, 1100)],
            vec![("Freight", 50)],
            100,
        ),
    )
    .await
    .unwrap();
    let d = purchases::get_purchase(&e.pool, &e.owner, a).await.unwrap();
    assert_eq!(
        (d.items[0].qty, d.purchase.grand_total, d.costs.len()),
        (3, 3200, 1)
    );
    // cancel draft: no ledger effect; reason required; cancelled invoice number may be reused
    assert!(purchases::cancel(&e.pool, &e.owner, a, "  ").await.is_err());
    purchases::cancel(&e.pool, &e.owner, a, "ordered by mistake")
        .await
        .unwrap();
    assert_eq!(suppliers::balance(&e.pool, sup).await.unwrap(), 0);
    purchases::create_draft(
        &e.pool,
        &e.owner,
        pin(sup, "INV-1", vec![(&v[0], 1, 1000)], vec![], 0),
    )
    .await
    .unwrap();
    assert!(matches!(
        purchases::cancel(&e.pool, &e.owner, a, "again").await,
        Err(CoreError::Conflict(_))
    ));
}

#[tokio::test]
async fn cancelling_a_confirmed_purchase_reverses_the_payable_but_only_if_untouched() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, v) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let sup = supplier(&e, "S").await;
    let po = confirmed_purchase(&e, sup, "I1", vec![(&v[0], 5, 1000)], vec![], 0).await;
    assert_eq!(suppliers::balance(&e.pool, sup).await.unwrap(), 5000);
    purchases::cancel(&e.pool, &e.owner, po, "supplier withdrew")
        .await
        .unwrap();
    assert_eq!(suppliers::balance(&e.pool, sup).await.unwrap(), 0);
    let st = suppliers::statement(&e.pool, &e.owner, sup).await.unwrap();
    assert_eq!(
        st.lines.iter().map(|l| l.kind.as_str()).collect::<Vec<_>>(),
        vec!["purchase", "purchase_cancel"],
        "history kept, never deleted"
    );
    // with a payment or a receipt it is refused
    let po2 = confirmed_purchase(&e, sup, "I2", vec![(&v[0], 5, 1000)], vec![], 0).await;
    purchases::pay_supplier(
        &e.pool,
        &e.owner,
        PaymentIn {
            supplier_id: sup.to_string(),
            purchase_id: Some(po2.to_string()),
            amount: 1000,
            method: "cash".into(),
            paid_on: "2026-03-02".into(),
            reference: "".into(),
            note: "".into(),
            client_key: "k1".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        purchases::cancel(&e.pool, &e.owner, po2, "x").await,
        Err(CoreError::Conflict(_))
    ));
    let po3 = confirmed_purchase(&e, sup, "I3", vec![(&v[0], 5, 1000)], vec![], 0).await;
    let it = purchases::get_purchase(&e.pool, &e.owner, po3)
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    purchases::receive(&e.pool, &e.owner, po3, rin(vec![recv(&it, 1, &[])]))
        .await
        .unwrap();
    assert!(matches!(
        purchases::cancel(&e.pool, &e.owner, po3, "x").await,
        Err(CoreError::Conflict(_))
    ));
}

fn pay(sup: Id, po: Option<Id>, amount: i64, key: &str) -> PaymentIn {
    PaymentIn {
        supplier_id: sup.to_string(),
        purchase_id: po.map(|p| p.to_string()),
        amount,
        method: "bank".into(),
        paid_on: "2026-03-10".into(),
        reference: "TT-1".into(),
        note: "".into(),
        client_key: key.into(),
    }
}

#[tokio::test]
async fn supplier_payments_are_idempotent_bounded_and_reversible() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, v) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let sup = supplier(&e, "S").await;
    let other = supplier(&e, "Other").await;
    let po = confirmed_purchase(&e, sup, "I1", vec![(&v[0], 10, 1000)], vec![], 0).await;
    // duplicate submit (same key) posts once
    let p1 = purchases::pay_supplier(&e.pool, &e.owner, pay(sup, Some(po), 4000, "form-1"))
        .await
        .unwrap();
    let p1b = purchases::pay_supplier(&e.pool, &e.owner, pay(sup, Some(po), 4000, "form-1"))
        .await
        .unwrap();
    assert_eq!(p1, p1b);
    assert_eq!(suppliers::balance(&e.pool, sup).await.unwrap(), 6000);
    // concurrent duplicates
    let mut hs = vec![];
    for _ in 0..8 {
        let (pool, owner) = (e.pool.clone(), e.owner.clone());
        hs.push(tokio::spawn(async move {
            purchases::pay_supplier(&pool, &owner, pay(sup, Some(po), 1000, "form-2"))
                .await
                .map(|i| i.to_string())
        }));
    }
    let mut ids = std::collections::HashSet::new();
    for h in hs {
        if let Ok(id) = h.await.unwrap() {
            ids.insert(id);
        }
    }
    assert_eq!(
        ids.len(),
        1,
        "exactly one payment recorded for one submission key"
    );
    assert_eq!(suppliers::balance(&e.pool, sup).await.unwrap(), 5000);
    // bounds & validation
    assert!(
        purchases::pay_supplier(&e.pool, &e.owner, pay(sup, Some(po), 5001, "k3"))
            .await
            .is_err(),
        "cannot overpay a purchase"
    );
    assert!(
        purchases::pay_supplier(&e.pool, &e.owner, pay(sup, Some(po), 0, "k4"))
            .await
            .is_err()
    );
    assert!(
        purchases::pay_supplier(&e.pool, &e.owner, pay(other, Some(po), 10, "k5"))
            .await
            .is_err(),
        "wrong supplier"
    );
    let mut m = pay(sup, Some(po), 10, "k6");
    m.method = "barter".into();
    assert!(purchases::pay_supplier(&e.pool, &e.owner, m).await.is_err());
    assert!(
        purchases::pay_supplier(&e.pool, &e.owner, pay(sup, None, 10, ""))
            .await
            .is_err(),
        "key required"
    );
    // advance payment on account (no purchase) is allowed and shows as a negative payable
    purchases::pay_supplier(&e.pool, &e.owner, pay(other, None, 700, "k7"))
        .await
        .unwrap();
    assert_eq!(suppliers::balance(&e.pool, other).await.unwrap(), -700);
    // reversal: new row + ledger entry, original untouched, can't repeat, can't reverse a reversal
    assert!(matches!(
        purchases::reverse_payment(&e.pool, &e.owner, p1, " ").await,
        Err(CoreError::Validation(_))
    ));
    let rev = purchases::reverse_payment(&e.pool, &e.owner, p1, "bounced cheque")
        .await
        .unwrap();
    assert_eq!(suppliers::balance(&e.pool, sup).await.unwrap(), 9000);
    assert!(purchases::reverse_payment(&e.pool, &e.owner, p1, "again")
        .await
        .is_err());
    assert!(purchases::reverse_payment(&e.pool, &e.owner, rev, "undo")
        .await
        .is_err());
    let d = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    assert_eq!(d.purchase.paid, 1000);
    assert_eq!(d.purchase.due, 9000);
    assert_eq!(audit_count(&e.pool, "supplier.payment_reversed").await, 1);
    // ledger lines always reconcile to the balance
    let st = suppliers::statement(&e.pool, &e.owner, sup).await.unwrap();
    assert_eq!(
        st.closing_balance,
        suppliers::balance(&e.pool, sup).await.unwrap()
    );
    assert_eq!(
        st.lines.iter().map(|l| l.amount).sum::<i64>(),
        st.closing_balance
    );
}

#[tokio::test]
async fn returns_to_supplier_reduce_stock_and_payable() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, fans) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let (_, fr) = product(&e, &c, "Fridge", true, &[("FR-1", 5000)]).await;
    let sup = supplier(&e, "S").await;
    let po = confirmed_purchase(
        &e,
        sup,
        "I1",
        vec![(&fans[0], 10, 1000), (&fr[0], 2, 5000)],
        vec![("Freight", 600)],
        0,
    )
    .await;
    let d = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    let fan = d
        .items
        .iter()
        .find(|i| !i.is_serialized)
        .unwrap()
        .id
        .clone();
    let fri = d.items.iter().find(|i| i.is_serialized).unwrap().id.clone();
    // nothing received yet -> nothing to return
    let rl = |item: &str, q: i64, s: &[&str]| ReturnLine {
        item_id: item.into(),
        qty: q,
        serials: s.iter().map(|x| x.to_string()).collect(),
    };
    let rr = |lines: Vec<ReturnLine>| ReturnIn {
        returned_on: "2026-03-20".into(),
        reason: "defective".into(),
        lines,
    };
    assert!(
        purchases::return_to_supplier(&e.pool, &e.owner, po, rr(vec![rl(&fan, 1, &[])]))
            .await
            .is_err()
    );
    purchases::receive(
        &e.pool,
        &e.owner,
        po,
        rin(vec![recv(&fan, 10, &[]), recv(&fri, 2, &["R1", "R2"])]),
    )
    .await
    .unwrap();
    let balance_before = suppliers::balance(&e.pool, sup).await.unwrap();
    // return 3 fans + 1 fridge
    purchases::return_to_supplier(
        &e.pool,
        &e.owner,
        po,
        rr(vec![rl(&fan, 3, &[]), rl(&fri, 1, &["r1"])]),
    )
    .await
    .unwrap();
    assert_eq!(avail(&e.pool, &fans[0]).await, 7);
    assert_eq!(avail(&e.pool, &fr[0]).await, 1);
    assert_eq!(
        suppliers::balance(&e.pool, sup).await.unwrap(),
        balance_before - (3 * 1000 + 5000),
        "credit = units x original cost"
    );
    let st: String =
        sqlx::query_scalar("SELECT status FROM product_serials WHERE serial_no = 'R1'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(st, "returned_to_supplier");
    assert_reconciled(&e).await;
    // limits and states
    assert!(
        matches!(
            purchases::return_to_supplier(&e.pool, &e.owner, po, rr(vec![rl(&fan, 8, &[])])).await,
            Err(CoreError::Conflict(_))
        ),
        "only 7 returnable"
    );
    assert!(
        purchases::return_to_supplier(&e.pool, &e.owner, po, rr(vec![rl(&fri, 1, &["r1"])]))
            .await
            .is_err(),
        "already returned"
    );
    assert!(
        purchases::return_to_supplier(&e.pool, &e.owner, po, rr(vec![rl(&fri, 1, &["NOPE"])]))
            .await
            .is_err()
    );
    let mut noreason = rr(vec![rl(&fan, 1, &[])]);
    noreason.reason = "".into();
    assert!(
        purchases::return_to_supplier(&e.pool, &e.owner, po, noreason)
            .await
            .is_err()
    );
    // a failed return leaves no partial effects
    assert!(purchases::return_to_supplier(
        &e.pool,
        &e.owner,
        po,
        rr(vec![rl(&fan, 1, &[]), rl(&fri, 1, &["NOPE"])])
    )
    .await
    .is_err());
    assert_eq!(avail(&e.pool, &fans[0]).await, 7);
    assert_reconciled(&e).await;
    let det = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    assert_eq!(det.returns.len(), 1);
    assert_eq!(det.returns[0].credit_total, 8000);
    // a sold/otherwise-moved serial cannot be returned
    let sid: String = sqlx::query_scalar("SELECT id FROM product_serials WHERE serial_no = 'R2'")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    inventory::change_serial_status(
        &e.pool,
        &e.owner,
        Id::parse(&sid).unwrap(),
        "written_off",
        "stolen",
    )
    .await
    .unwrap();
    assert!(
        purchases::return_to_supplier(&e.pool, &e.owner, po, rr(vec![rl(&fri, 1, &["R2"])]))
            .await
            .is_err()
    );
}

// ------------------------------------------------------------------ inventory operations

#[tokio::test]
async fn opening_stock_rules() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, fans) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let (_, fr) = product(&e, &c, "Fridge", true, &[("FR-1", 5000)]).await;
    let op = |v: &str, q: i64, s: &[&str]| OpeningStockIn {
        variant_id: v.into(),
        qty: q,
        unit_cost: 1500,
        serials: s.iter().map(|x| x.to_string()).collect(),
        reason: "Opening balance".into(),
    };
    assert!(
        inventory::record_opening_stock(&e.pool, &e.owner, op(&fans[0], 0, &[]))
            .await
            .is_err()
    );
    assert!(
        inventory::record_opening_stock(&e.pool, &e.owner, op(&fans[0], 3, &["x"]))
            .await
            .is_err(),
        "serials on non-serialized"
    );
    assert!(
        inventory::record_opening_stock(&e.pool, &e.owner, op(&fr[0], 0, &["A", "a"]))
            .await
            .is_err(),
        "dup serials"
    );
    let mut noreason = op(&fans[0], 3, &[]);
    noreason.reason = " ".into();
    assert!(inventory::record_opening_stock(&e.pool, &e.owner, noreason)
        .await
        .is_err());
    assert_eq!(avail(&e.pool, &fans[0]).await, 0);
    inventory::record_opening_stock(&e.pool, &e.owner, op(&fans[0], 12, &[]))
        .await
        .unwrap();
    inventory::record_opening_stock(&e.pool, &e.owner, op(&fr[0], 0, &["OP-1", "OP-2"]))
        .await
        .unwrap();
    assert_eq!(avail(&e.pool, &fans[0]).await, 12);
    assert_eq!(avail(&e.pool, &fr[0]).await, 2);
    // only once: later corrections use adjustments
    assert!(matches!(
        inventory::record_opening_stock(&e.pool, &e.owner, op(&fans[0], 1, &[])).await,
        Err(CoreError::Conflict(_))
    ));
    let lv = inventory::stock_levels(&e.pool, &e.owner, StockFilter::default())
        .await
        .unwrap();
    assert_eq!(lv.total_value, 12 * 1500 + 2 * 1500);
    assert_reconciled(&e).await;
    let n: String =
        sqlx::query_scalar("SELECT number FROM stock_adjustments ORDER BY number LIMIT 1")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(n, "ADJ-000001");
}

#[tokio::test]
async fn quantity_adjustments_use_average_cost_and_never_go_negative() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, v) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: v[0].clone(),
            qty: 3,
            unit_cost: 1000,
            serials: vec![],
            reason: "open".into(),
        },
    )
    .await
    .unwrap();
    let adj = |q: i64, uc: Option<i64>| AdjustIn {
        variant_id: v[0].clone(),
        bucket: "available".into(),
        qty_delta: q,
        unit_cost: uc,
        reason: "stock count".into(),
    };
    inventory::adjust_quantity(&e.pool, &e.owner, adj(3, Some(1100)))
        .await
        .unwrap(); // 3@1000 + 3@1100 -> value 6300
    let lv = inventory::stock_levels(&e.pool, &e.owner, StockFilter::default())
        .await
        .unwrap();
    assert_eq!(
        (lv.rows[0].available, lv.rows[0].value, lv.rows[0].avg_cost),
        (6, 6300, 1050)
    );
    assert!(
        inventory::adjust_quantity(&e.pool, &e.owner, adj(1, None))
            .await
            .is_err(),
        "increase needs a cost"
    );
    assert!(inventory::adjust_quantity(&e.pool, &e.owner, adj(0, None))
        .await
        .is_err());
    assert!(
        matches!(
            inventory::adjust_quantity(&e.pool, &e.owner, adj(-7, None)).await,
            Err(CoreError::Conflict(_))
        ),
        "cannot remove more than on hand"
    );
    inventory::adjust_quantity(&e.pool, &e.owner, adj(-2, None))
        .await
        .unwrap(); // out at avg 1050 each = 2100
    let lv = inventory::stock_levels(&e.pool, &e.owner, StockFilter::default())
        .await
        .unwrap();
    assert_eq!((lv.rows[0].available, lv.rows[0].value), (4, 4200));
    inventory::adjust_quantity(&e.pool, &e.owner, adj(-4, None))
        .await
        .unwrap(); // remove the rest: whole value goes, no residue
    let lv = inventory::stock_levels(&e.pool, &e.owner, StockFilter::default())
        .await
        .unwrap();
    assert_eq!((lv.rows[0].available, lv.rows[0].value), (0, 0));
    assert_reconciled(&e).await;
    // bucket moves
    inventory::adjust_quantity(&e.pool, &e.owner, adj(5, Some(1000)))
        .await
        .unwrap();
    let mv = |from: &str, to: &str, q: i64| MoveIn {
        variant_id: v[0].clone(),
        from_bucket: from.into(),
        to_bucket: to.into(),
        qty: q,
        reason: "water damage".into(),
    };
    inventory::move_quantity(&e.pool, &e.owner, mv("available", "damaged", 2))
        .await
        .unwrap();
    let lv = inventory::stock_levels(&e.pool, &e.owner, StockFilter::default())
        .await
        .unwrap();
    assert_eq!(
        (lv.rows[0].available, lv.rows[0].damaged, lv.rows[0].value),
        (3, 2, 5000),
        "moving between buckets keeps value"
    );
    assert!(
        inventory::move_quantity(&e.pool, &e.owner, mv("available", "available", 1))
            .await
            .is_err()
    );
    assert!(
        inventory::move_quantity(&e.pool, &e.owner, mv("available", "damaged", 99))
            .await
            .is_err()
    );
    assert!(
        inventory::move_quantity(&e.pool, &e.owner, mv("nowhere", "damaged", 1))
            .await
            .is_err()
    );
    assert_reconciled(&e).await;
    assert!(audit_count(&e.pool, "stock.adjusted").await >= 4);
    assert_eq!(audit_count(&e.pool, "stock.moved").await, 1);
    // every movement has who/when/why/document
    let bad: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stock_movements WHERE reason = '' OR actor_id IS NULL OR doc_type = '' OR doc_id = ''").fetch_one(&e.pool).await.unwrap();
    assert_eq!(bad, 0);
}

#[tokio::test]
async fn serial_lifecycle_is_enforced_and_logged() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, fr) = product(&e, &c, "Fridge", true, &[("FR-1", 5000)]).await;
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: fr[0].clone(),
            qty: 0,
            unit_cost: 5000,
            serials: vec!["L1".into(), "L2".into()],
            reason: "open".into(),
        },
    )
    .await
    .unwrap();
    let ser = inventory::serials(
        &e.pool,
        &e.owner,
        SerialFilter {
            variant_id: Some(fr[0].clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let l1 = Id::parse(&ser.iter().find(|s| s.serial_no == "L1").unwrap().id).unwrap();
    // available -> damaged -> warranty_service -> available
    inventory::change_serial_status(&e.pool, &e.owner, l1, "damaged", "dropped")
        .await
        .unwrap();
    assert_eq!(avail(&e.pool, &fr[0]).await, 1);
    let lv = inventory::stock_levels(&e.pool, &e.owner, StockFilter::default())
        .await
        .unwrap();
    assert_eq!((lv.rows[0].available, lv.rows[0].damaged), (1, 1));
    assert_reconciled(&e).await;
    inventory::change_serial_status(
        &e.pool,
        &e.owner,
        l1,
        "warranty_service",
        "sent to Singer service",
    )
    .await
    .unwrap();
    inventory::change_serial_status(&e.pool, &e.owner, l1, "available", "repaired")
        .await
        .unwrap();
    assert_eq!(avail(&e.pool, &fr[0]).await, 2);
    // illegal / reserved transitions
    assert!(
        matches!(
            inventory::change_serial_status(&e.pool, &e.owner, l1, "sold", "x").await,
            Err(CoreError::Conflict(_))
        ),
        "sales own the sold transition"
    );
    assert!(inventory::change_serial_status(
        &e.pool,
        &e.owner,
        l1,
        "returned_pending_inspection",
        "x"
    )
    .await
    .is_err());
    assert!(
        inventory::change_serial_status(&e.pool, &e.owner, l1, "available", "x")
            .await
            .is_err(),
        "no self transition"
    );
    assert!(
        inventory::change_serial_status(&e.pool, &e.owner, l1, "bogus", "x")
            .await
            .is_err()
    );
    assert!(
        inventory::change_serial_status(&e.pool, &e.owner, l1, "damaged", "")
            .await
            .is_err(),
        "reason required"
    );
    inventory::change_serial_status(&e.pool, &e.owner, l1, "written_off", "irreparable")
        .await
        .unwrap();
    assert!(
        inventory::change_serial_status(&e.pool, &e.owner, l1, "available", "oops")
            .await
            .is_err(),
        "written off is terminal"
    );
    assert_eq!(avail(&e.pool, &fr[0]).await, 1);
    // full history, oldest first
    let h = inventory::serial_history(&e.pool, &e.owner, l1)
        .await
        .unwrap();
    assert_eq!(
        h.iter().map(|x| x.to_status.as_str()).collect::<Vec<_>>(),
        vec![
            "available",
            "damaged",
            "warranty_service",
            "available",
            "written_off"
        ]
    );
    assert!(h
        .iter()
        .all(|x| x.actor.as_deref() == Some("owner") && !x.reason.is_empty()));
    // quantity adjustment of serial-tracked stock is refused
    assert!(inventory::adjust_quantity(
        &e.pool,
        &e.owner,
        AdjustIn {
            variant_id: fr[0].clone(),
            bucket: "available".into(),
            qty_delta: -1,
            unit_cost: None,
            reason: "x".into()
        }
    )
    .await
    .is_err());
    assert_reconciled(&e).await;
    // search filters
    let s = inventory::serials(
        &e.pool,
        &e.owner,
        SerialFilter {
            status: Some("written_off".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(s.len(), 1);
    assert!(inventory::serials(
        &e.pool,
        &e.owner,
        SerialFilter {
            status: Some("nope".into()),
            ..Default::default()
        }
    )
    .await
    .is_err());
}

#[test]
fn lifecycle_graph_is_exactly_as_specified() {
    use SerialStatus::*;
    let all = [
        Available,
        Reserved,
        Sold,
        ReturnedPendingInspection,
        Damaged,
        WarrantyService,
        WrittenOff,
        ReturnedToSupplier,
    ];
    let allowed: Vec<(SerialStatus, SerialStatus)> = vec![
        (Available, Reserved),
        (Available, Damaged),
        (Available, WarrantyService),
        (Available, Sold),
        (Available, WrittenOff),
        (Available, ReturnedToSupplier),
        (Reserved, Available),
        (Reserved, Sold),
        (Sold, ReturnedPendingInspection),
        (ReturnedPendingInspection, Available),
        (ReturnedPendingInspection, Damaged),
        (ReturnedPendingInspection, WrittenOff),
        (Damaged, Available),
        (Damaged, WrittenOff),
        (Damaged, ReturnedToSupplier),
        (Damaged, WarrantyService),
        (WarrantyService, Available),
        (WarrantyService, Damaged),
        (WarrantyService, WrittenOff),
    ];
    for a in all {
        for b in all {
            assert_eq!(
                stock::transition_allowed(a, b),
                allowed.contains(&(a, b)),
                "{a:?} -> {b:?}"
            );
        }
    }
    // a returned appliance can never become sellable without inspection
    assert!(!stock::transition_allowed(Sold, Available));
    assert!(
        !stock::manual_transition_allowed(ReturnedPendingInspection, Available),
        "inspection is a returns-workflow decision"
    );
}

#[tokio::test]
async fn the_ledgers_are_append_only() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, v) = product(&e, &c, "Fan", true, &[("FAN-1", 1000)]).await;
    let sup = supplier(&e, "S").await;
    let po = confirmed_purchase(&e, sup, "I1", vec![(&v[0], 1, 1000)], vec![], 0).await;
    let it = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    purchases::receive(&e.pool, &e.owner, po, rin(vec![recv(&it, 1, &["Z1"])]))
        .await
        .unwrap();
    purchases::pay_supplier(&e.pool, &e.owner, pay(sup, Some(po), 100, "k"))
        .await
        .unwrap();
    for (t, set) in [
        ("stock_movements", "reason='x'"),
        ("serial_events", "reason='x'"),
        ("supplier_ledger_entries", "amount=1"),
        ("supplier_payments", "amount=1"),
    ] {
        assert!(
            sqlx::query(&format!("UPDATE {t} SET {set}"))
                .execute(&e.pool)
                .await
                .is_err(),
            "UPDATE {t} must be blocked"
        );
        assert!(
            sqlx::query(&format!("DELETE FROM {t}"))
                .execute(&e.pool)
                .await
                .is_err(),
            "DELETE {t} must be blocked"
        );
    }
}

#[tokio::test]
async fn reconciliation_detects_tampering() {
    let e = env().await;
    let c = cat(&e).await;
    let (_, fans) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let (_, fr) = product(&e, &c, "Fridge", true, &[("FR-1", 5000)]).await;
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: fans[0].clone(),
            qty: 5,
            unit_cost: 1000,
            serials: vec![],
            reason: "o".into(),
        },
    )
    .await
    .unwrap();
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: fr[0].clone(),
            qty: 0,
            unit_cost: 5000,
            serials: vec!["T1".into()],
            reason: "o".into(),
        },
    )
    .await
    .unwrap();
    assert_reconciled(&e).await;
    // someone edits the cache directly
    sqlx::query(
        "UPDATE stock_balances SET qty = qty + 1 WHERE variant_id = ? AND bucket = 'available'",
    )
    .bind(&fans[0])
    .execute(&e.pool)
    .await
    .unwrap();
    let d = stock::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(
        d.iter()
            .any(|x| x.check == "ledger_qty_vs_cached_qty" && x.expected == 5 && x.actual == 6),
        "{d:?}"
    );
    sqlx::query("UPDATE stock_balances SET qty = qty - 1 WHERE variant_id = ?")
        .bind(&fans[0])
        .execute(&e.pool)
        .await
        .unwrap();
    assert_reconciled(&e).await;
    // a serial status changed behind the ledger's back
    sqlx::query("UPDATE product_serials SET status = 'damaged' WHERE serial_no = 'T1'")
        .execute(&e.pool)
        .await
        .unwrap();
    let d = stock::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(
        d.iter().any(|x| x.check == "serial_count_vs_quantity"),
        "{d:?}"
    );
}

#[tokio::test]
async fn stock_queries_movements_and_low_stock() {
    let e = env().await;
    let c = cat(&e).await;
    let (pid, v) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let mut up = catalogue::get_product(&e.pool, &e.owner, pid)
        .await
        .unwrap();
    catalogue::update_product(
        &e.pool,
        &e.owner,
        catalogue::UpdateProduct {
            id: pid.to_string(),
            brand_id: up.brand_id.clone(),
            category_id: up.category_id.clone(),
            name: up.name.clone(),
            model: up.model.clone(),
            description: "".into(),
            warranty_months: 12,
            warranty_terms: "".into(),
            min_stock: 5,
            is_serialized: false,
        },
    )
    .await
    .unwrap();
    up = catalogue::get_product(&e.pool, &e.owner, pid)
        .await
        .unwrap();
    assert!(up.low_stock);
    let low = inventory::stock_levels(
        &e.pool,
        &e.owner,
        StockFilter {
            low_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(low.total, 1);
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: v[0].clone(),
            qty: 6,
            unit_cost: 1000,
            serials: vec![],
            reason: "open".into(),
        },
    )
    .await
    .unwrap();
    let low = inventory::stock_levels(
        &e.pool,
        &e.owner,
        StockFilter {
            low_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(low.total, 0);
    inventory::adjust_quantity(
        &e.pool,
        &e.owner,
        AdjustIn {
            variant_id: v[0].clone(),
            bucket: "available".into(),
            qty_delta: -2,
            unit_cost: None,
            reason: "count".into(),
        },
    )
    .await
    .unwrap();
    let mv = inventory::movements(&e.pool, &e.owner, Id::parse(&v[0]).unwrap(), 10, 0)
        .await
        .unwrap();
    assert_eq!(mv.len(), 2);
    assert_eq!(
        (mv[0].qty_delta, mv[0].balance_after),
        (-2, 4),
        "newest first with running balance"
    );
    assert_eq!((mv[1].qty_delta, mv[1].balance_after), (6, 6));
    // serial-tracked type cannot flip once history exists
    let (spid, sv) = product(&e, &c, "TV", false, &[("TV-1", 100)]).await;
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: sv[0].clone(),
            qty: 1,
            unit_cost: 100,
            serials: vec![],
            reason: "o".into(),
        },
    )
    .await
    .unwrap();
    let t = catalogue::get_product(&e.pool, &e.owner, spid)
        .await
        .unwrap();
    let r = catalogue::update_product(
        &e.pool,
        &e.owner,
        catalogue::UpdateProduct {
            id: t.id.clone(),
            brand_id: t.brand_id,
            category_id: t.category_id,
            name: t.name,
            model: t.model,
            description: "".into(),
            warranty_months: 0,
            warranty_terms: "".into(),
            min_stock: 0,
            is_serialized: true,
        },
    )
    .await;
    assert!(matches!(r, Err(CoreError::Conflict(_))));
}

// ------------------------------------------------------------------ authorization & upgrade

#[tokio::test]
async fn p2_services_enforce_permissions() {
    let e = env().await;
    let c = cat(&e).await;
    let (pid, v) = product(&e, &c, "Fan", false, &[("FAN-1", 1000)]).await;
    let sup = supplier(&e, "S").await;
    let po = purchases::create_draft(
        &e.pool,
        &e.owner,
        pin(sup, "I1", vec![(&v[0], 1, 1000)], vec![], 0),
    )
    .await
    .unwrap();
    let cashier = user_with_role(&e, "cash", "Cashier").await; // view products/stock only
    let acct = user_with_role(&e, "acct", "Accountant").await; // no catalogue/stock perms
                                                               // writes denied for a view-only role
    assert!(forbidden(
        catalogue::create_brand(&e.pool, &cashier, "X").await
    ));
    assert!(forbidden(
        catalogue::create_category(&e.pool, &cashier, "X", None).await
    ));
    assert!(forbidden(
        catalogue::create_product(
            &e.pool,
            &cashier,
            NewProduct {
                brand_id: c.brand.to_string(),
                category_id: c.category.to_string(),
                name: "x".into(),
                model: "".into(),
                description: "".into(),
                warranty_months: 0,
                warranty_terms: "".into(),
                min_stock: 0,
                is_serialized: false,
                variants: vec![vin("Q", 1)]
            }
        )
        .await
    ));
    assert!(forbidden(
        catalogue::set_product_active(&e.pool, &cashier, pid, false).await
    ));
    assert!(forbidden(
        suppliers::create_supplier(
            &e.pool,
            &cashier,
            SupplierIn {
                name: "x".into(),
                contact_person: "".into(),
                phone: "".into(),
                alt_phone: "".into(),
                email: "".into(),
                address: "".into(),
                notes: "".into()
            }
        )
        .await
    ));
    assert!(forbidden(
        purchases::create_draft(
            &e.pool,
            &cashier,
            pin(sup, "I2", vec![(&v[0], 1, 1)], vec![], 0)
        )
        .await
    ));
    assert!(forbidden(purchases::confirm(&e.pool, &cashier, po).await));
    assert!(forbidden(
        purchases::cancel(&e.pool, &cashier, po, "x").await
    ));
    assert!(forbidden(
        purchases::receive(&e.pool, &cashier, po, rin(vec![])).await
    ));
    assert!(forbidden(
        purchases::pay_supplier(&e.pool, &cashier, pay(sup, None, 1, "k")).await
    ));
    assert!(forbidden(
        purchases::reverse_payment(&e.pool, &cashier, Id::new(), "x").await
    ));
    assert!(forbidden(
        purchases::return_to_supplier(
            &e.pool,
            &cashier,
            po,
            ReturnIn {
                returned_on: "2026-01-01".into(),
                reason: "x".into(),
                lines: vec![]
            }
        )
        .await
    ));
    assert!(forbidden(
        inventory::record_opening_stock(
            &e.pool,
            &cashier,
            OpeningStockIn {
                variant_id: v[0].clone(),
                qty: 1,
                unit_cost: 1,
                serials: vec![],
                reason: "x".into()
            }
        )
        .await
    ));
    assert!(forbidden(
        inventory::adjust_quantity(
            &e.pool,
            &cashier,
            AdjustIn {
                variant_id: v[0].clone(),
                bucket: "available".into(),
                qty_delta: 1,
                unit_cost: Some(1),
                reason: "x".into()
            }
        )
        .await
    ));
    assert!(forbidden(
        inventory::move_quantity(
            &e.pool,
            &cashier,
            MoveIn {
                variant_id: v[0].clone(),
                from_bucket: "available".into(),
                to_bucket: "damaged".into(),
                qty: 1,
                reason: "x".into()
            }
        )
        .await
    ));
    assert!(forbidden(
        inventory::change_serial_status(&e.pool, &cashier, Id::new(), "damaged", "x").await
    ));
    // reads denied without the view permission
    assert!(forbidden(
        catalogue::list_products(&e.pool, &acct, ProductFilter::default()).await
    ));
    assert!(forbidden(
        catalogue::search_variants(&e.pool, &acct, "x", 5).await
    ));
    assert!(forbidden(
        inventory::stock_levels(&e.pool, &acct, StockFilter::default()).await
    ));
    assert!(forbidden(
        inventory::movements(&e.pool, &acct, Id::new(), 5, 0).await
    ));
    assert!(forbidden(
        stock::reconcile(&e.pool, &cashier_without_inventory(&e).await).await
    ));
    // supplier/purchase reads need their own permissions; cashier has neither
    assert!(forbidden(
        suppliers::list_suppliers(&e.pool, &cashier, None, false).await
    ));
    assert!(forbidden(
        suppliers::statement(&e.pool, &cashier, sup).await
    ));
    assert!(forbidden(
        purchases::list_purchases(&e.pool, &cashier, PurchaseFilter::default()).await
    ));
    assert!(forbidden(
        purchases::get_purchase(&e.pool, &cashier, po).await
    ));
    // nothing changed
    assert_eq!(
        purchases::get_purchase(&e.pool, &e.owner, po)
            .await
            .unwrap()
            .purchase
            .status,
        "draft"
    );
    // the accountant may view purchases/suppliers and pay, but not receive
    assert!(suppliers::list_suppliers(&e.pool, &acct, None, false)
        .await
        .is_ok());
    assert!(forbidden(
        purchases::receive(&e.pool, &acct, po, rin(vec![])).await
    ));
}

async fn cashier_without_inventory(e: &Env) -> Principal {
    let mut p = user_with_role(e, "noinv", "Cashier").await;
    p.permissions.remove("inventory.view");
    p
}

#[tokio::test]
async fn new_permissions_reach_default_roles_once_and_admin_removals_stick() {
    let e = env().await;
    // simulate an upgrade from P1: grant tracking empty, P2 permissions missing from default roles
    sqlx::query("DELETE FROM default_grants")
        .execute(&e.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM role_permissions WHERE perm_key IN ('purchases.pay','suppliers.view','purchases.view','purchases.return')").execute(&e.pool).await.unwrap();
    // an administrator had removed products.view from Cashier in P1
    sqlx::query("DELETE FROM role_permissions WHERE perm_key = 'products.view' AND role_id = (SELECT id FROM roles WHERE name='Cashier')").execute(&e.pool).await.unwrap();
    authz::sync_catalogue(&e.pool).await.unwrap();
    let has = |role: &str, perm: &str| {
        let pool = e.pool.clone();
        let (role, perm) = (role.to_string(), perm.to_string());
        async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM role_permissions rp JOIN roles r ON r.id = rp.role_id WHERE r.name = ? AND rp.perm_key = ?").bind(role).bind(perm).fetch_one(&pool).await.unwrap() > 0
        }
    };
    assert!(
        has("Manager", "purchases.pay").await,
        "new P2 permission granted to default role"
    );
    assert!(has("Purchase Manager", "suppliers.view").await);
    assert!(
        !has("Cashier", "products.view").await,
        "an admin's earlier removal is not undone"
    );
    // admin now removes a P2 grant; further syncs must not re-add it
    sqlx::query("DELETE FROM role_permissions WHERE perm_key = 'purchases.pay' AND role_id = (SELECT id FROM roles WHERE name='Manager')").execute(&e.pool).await.unwrap();
    authz::sync_catalogue(&e.pool).await.unwrap();
    assert!(!has("Manager", "purchases.pay").await);
    // Owner always has every permission
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM role_permissions rp JOIN roles r ON r.id = rp.role_id WHERE r.name = 'Owner'").fetch_one(&e.pool).await.unwrap();
    assert_eq!(n as usize, authz::CATALOGUE.len());
}

#[tokio::test]
async fn supplier_profiles() {
    let e = env().await;
    let s = supplier(&e, "Rahman Electronics").await;
    assert!(matches!(
        suppliers::create_supplier(
            &e.pool,
            &e.owner,
            SupplierIn {
                name: "rahman electronics".into(),
                contact_person: "".into(),
                phone: "".into(),
                alt_phone: "".into(),
                email: "".into(),
                address: "".into(),
                notes: "".into()
            }
        )
        .await,
        Err(CoreError::Conflict(_))
    ));
    suppliers::update_supplier(
        &e.pool,
        &e.owner,
        s,
        SupplierIn {
            name: "Rahman Electronics Ltd".into(),
            contact_person: "Mr Rahman".into(),
            phone: "01700000000".into(),
            alt_phone: "".into(),
            email: "".into(),
            address: "Dhaka".into(),
            notes: "".into(),
        },
    )
    .await
    .unwrap();
    let l = suppliers::list_suppliers(&e.pool, &e.owner, Some("rahman"), false)
        .await
        .unwrap();
    assert_eq!((l.len(), l[0].balance), (1, 0));
    suppliers::set_supplier_active(&e.pool, &e.owner, s, false)
        .await
        .unwrap();
    assert!(suppliers::list_suppliers(&e.pool, &e.owner, None, false)
        .await
        .unwrap()
        .is_empty());
    // inactive suppliers cannot receive new purchases
    let c = cat(&e).await;
    let (_, v) = product(&e, &c, "Fan", false, &[("F", 1)]).await;
    assert!(purchases::create_draft(
        &e.pool,
        &e.owner,
        pin(s, "I", vec![(&v[0], 1, 1)], vec![], 0)
    )
    .await
    .is_err());
    let _ = SessionStore::standard();
}
