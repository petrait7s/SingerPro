//! P5 test-suite: returns with inspection, refunds, warranty, expenses, cash sessions.

use crate::accounting::{self, code};
use crate::auth;
use crate::authz::{self, Principal};
use crate::cash;
use crate::catalogue::{self, NewProduct, NewVariant};
use crate::customers::{self, CustomerIn};
use crate::expenses::{self, ExpenseFilter, ExpenseIn};
use crate::installments::InstallmentIn;
use crate::inventory::{self, OpeningStockIn};
use crate::purchases::{self, PaymentIn};
use crate::recovery::DataPaths;
use crate::returns::{self, ApproveIn, DecisionIn, RecordReturnIn, ReturnLineIn};
use crate::sales::{self, PayLineIn, SaleIn, SaleLineIn};
use crate::suppliers::{self, SupplierIn};
use crate::users::{self, NewUser};
use crate::warranty::{self, OpenCaseIn};
use crate::{db, settings, time, CoreError, Id};
use chrono::Duration;
use sqlx::SqlitePool;
use std::collections::HashMap;

const PW: &str = "Correct-horse-battery-9";

struct Env {
    pool: SqlitePool,
    owner: Principal,
    fridge: String,
    fan: String,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&DataPaths::new(dir.path()).db).await.unwrap();
    let id = auth::setup_owner(&pool, "owner", "Owner", PW)
        .await
        .unwrap();
    let owner = authz::load_principal(&pool, id).await.unwrap();
    let brand = catalogue::create_brand(&pool, &owner, "Singer")
        .await
        .unwrap();
    let cat = catalogue::create_category(&pool, &owner, "Appliances", None)
        .await
        .unwrap();
    let v = |sku: &str, cost, retail, wholesale| NewVariant {
        sku: sku.into(),
        barcode: None,
        colour: "".into(),
        capacity: "".into(),
        size: "".into(),
        purchase_cost: cost,
        retail_price: retail,
        wholesale_price: wholesale,
        installment_price: None,
    };
    let mk = |name: &str, serialized: bool, months: i64, v: NewVariant| NewProduct {
        brand_id: brand.to_string(),
        category_id: cat.to_string(),
        name: name.into(),
        model: "".into(),
        description: "".into(),
        warranty_months: months,
        warranty_terms: "Parts and labour".into(),
        min_stock: 0,
        is_serialized: serialized,
        variants: vec![v],
    };
    let fridge_p = catalogue::create_product(
        &pool,
        &owner,
        mk(
            "Fridge",
            true,
            12,
            v("FRIDGE", 4_000_000, 5_200_000, 4_800_000),
        ),
    )
    .await
    .unwrap();
    let fan_p = catalogue::create_product(
        &pool,
        &owner,
        mk("Fan", false, 0, v("FAN", 100_000, 150_000, 130_000)),
    )
    .await
    .unwrap();
    let vid = |p: Id| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT id FROM product_variants WHERE product_id = ?")
                .bind(p.to_string())
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let (fridge, fan) = (vid(fridge_p).await, vid(fan_p).await);
    let serials: Vec<String> = (1..=6).map(|i| format!("F-{i}")).collect();
    inventory::record_opening_stock(
        &pool,
        &owner,
        OpeningStockIn {
            variant_id: fridge.clone(),
            qty: 0,
            unit_cost: 4_000_000,
            serials,
            reason: "open".into(),
        },
    )
    .await
    .unwrap();
    inventory::record_opening_stock(
        &pool,
        &owner,
        OpeningStockIn {
            variant_id: fan.clone(),
            qty: 20,
            unit_cost: 100_000,
            serials: vec![],
            reason: "open".into(),
        },
    )
    .await
    .unwrap();
    Env {
        pool,
        owner,
        fridge,
        fan,
        _dir: dir,
    }
}

async fn user(e: &Env, name: &str, role: &str) -> Principal {
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

/// A clerk who may request returns/expenses but approve nothing.
async fn clerk(e: &Env, name: &str) -> Principal {
    let rid = users::create_role(
        &e.pool,
        &e.owner,
        &format!("Clerk {name}"),
        "",
        vec![
            "returns.process".into(),
            "sales.view".into(),
            "expenses.create".into(),
            "cash.session".into(),
            "warranty.manage".into(),
            "customers.view".into(),
        ],
    )
    .await
    .unwrap();
    let id = users::create_user(
        &e.pool,
        &e.owner,
        NewUser {
            username: name.into(),
            full_name: name.into(),
            password: PW.into(),
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
    authz::load_principal(&e.pool, id).await.unwrap()
}

fn forbidden<T: std::fmt::Debug>(r: Result<T, CoreError>) -> bool {
    matches!(r, Err(CoreError::Forbidden(_)))
}

async fn audit_count(pool: &SqlitePool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs WHERE action = ?")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn bal(e: &Env, account: &str) -> i64 {
    accounting::list_accounts(&e.pool, &e.owner, None)
        .await
        .unwrap()
        .into_iter()
        .find(|a| a.code == account)
        .unwrap()
        .balance
}

async fn assert_books(e: &Env) {
    let m = accounting::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(m.is_empty(), "books must reconcile: {m:?}");
    let s = crate::stock::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(s.is_empty(), "stock must reconcile: {s:?}");
    assert!(
        accounting::trial_balance(&e.pool, &e.owner, None)
            .await
            .unwrap()
            .balanced
    );
}

async fn serial_id(e: &Env, sn: &str) -> String {
    sqlx::query_scalar("SELECT id FROM product_serials WHERE serial_no = ?")
        .bind(sn)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

async fn serial_status(e: &Env, sn: &str) -> String {
    sqlx::query_scalar("SELECT status FROM product_serials WHERE serial_no = ?")
        .bind(sn)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

async fn avail(e: &Env, variant: &str, bucket: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE((SELECT qty FROM stock_balances WHERE variant_id = ? AND bucket = ?), 0)",
    )
    .bind(variant)
    .bind(bucket)
    .fetch_one(&e.pool)
    .await
    .unwrap()
}

fn cust(name: &str) -> CustomerIn {
    CustomerIn {
        name: name.into(),
        phone: "017".into(),
        alt_phone: "".into(),
        address: "".into(),
        customer_type: "retail".into(),
        credit_limit: 0,
        nid: "".into(),
        notes: "".into(),
    }
}

fn pay(m: &str, a: i64) -> PayLineIn {
    PayLineIn {
        method: m.into(),
        amount: a,
        reference: "".into(),
    }
}

fn si(
    mode: &str,
    customer: Option<Id>,
    items: Vec<SaleLineIn>,
    payments: Vec<PayLineIn>,
    key: &str,
    date: Option<&str>,
) -> SaleIn {
    SaleIn {
        mode: mode.into(),
        customer_id: customer.map(|c| c.to_string()),
        sale_date: date.map(str::to_string),
        items,
        payments,
        notes: "".into(),
        client_key: key.into(),
        credit_override: false,
        installment: None,
    }
}

async fn sell_fridge(
    e: &Env,
    sn: &str,
    customer: Option<Id>,
    paid: i64,
    key: &str,
    date: Option<&str>,
) -> sales::SaleResult {
    let sid = serial_id(e, sn).await;
    sales::create_sale(
        &e.pool,
        &e.owner,
        si(
            "retail",
            customer,
            vec![SaleLineIn {
                variant_id: e.fridge.clone(),
                qty: 1,
                discount: 0,
                serial_ids: vec![sid],
            }],
            vec![pay("cash", paid)],
            key,
            date,
        ),
    )
    .await
    .unwrap()
}

async fn sale_items(e: &Env, sale_id: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM sale_items WHERE sale_id = ? ORDER BY rowid")
        .bind(sale_id)
        .fetch_all(&e.pool)
        .await
        .unwrap()
}

fn rec(sale: &str, lines: Vec<(&str, i64)>, key: &str) -> RecordReturnIn {
    RecordReturnIn {
        sale_id: sale.into(),
        reason: "Customer changed mind".into(),
        lines: lines
            .into_iter()
            .map(|(i, q)| ReturnLineIn {
                sale_item_id: i.into(),
                qty: q,
            })
            .collect(),
        client_key: key.into(),
    }
}

fn dec(items: &[(&str, &str)]) -> Vec<DecisionIn> {
    items
        .iter()
        .map(|(i, o)| DecisionIn {
            item_id: i.to_string(),
            outcome: o.to_string(),
            note: "checked".into(),
        })
        .collect()
}

async fn return_items(e: &Env, return_id: Id) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM sale_return_items WHERE return_id = ? ORDER BY rowid")
        .bind(return_id.to_string())
        .fetch_all(&e.pool)
        .await
        .unwrap()
}

fn approve(decisions: Vec<DecisionIn>, refund: i64, key: &str) -> ApproveIn {
    ApproveIn {
        decisions,
        refund_amount: refund,
        refund_method: "cash".into(),
        note: "approved".into(),
        client_key: key.into(),
    }
}

// ------------------------------------------------------------------ returns

#[tokio::test]
async fn a_returned_unit_waits_for_inspection_and_is_not_sellable_or_refunded_until_approved() {
    let e = env().await;
    let c = customers::create_customer(&e.pool, &e.owner, cust("Karim"))
        .await
        .unwrap();
    let r = sell_fridge(&e, "F-1", Some(c), 5_200_000, "s1", None).await;
    let items = sale_items(&e, &r.id).await;
    let clerk = clerk(&e, "clerk").await;
    let ret = returns::record_return(&e.pool, &clerk, rec(&r.id, vec![(&items[0], 1)], "r1"))
        .await
        .unwrap();
    // goods are back but quarantined: not available, not sellable, no refund, no credit note yet
    assert_eq!(
        serial_status(&e, "F-1").await,
        "returned_pending_inspection"
    );
    assert_eq!(
        (
            avail(&e, &e.fridge, "available").await,
            avail(&e, &e.fridge, "pending_inspection").await
        ),
        (5, 1)
    );
    let sid = serial_id(&e, "F-1").await;
    let again = sales::create_sale(
        &e.pool,
        &e.owner,
        si(
            "retail",
            None,
            vec![SaleLineIn {
                variant_id: e.fridge.clone(),
                qty: 1,
                discount: 0,
                serial_ids: vec![sid.clone()],
            }],
            vec![pay("cash", 5_200_000)],
            "again",
            None,
        ),
    )
    .await;
    assert!(
        matches!(again, Err(CoreError::Conflict(_))),
        "a unit awaiting inspection cannot be sold"
    );
    assert!(
        inventory::change_serial_status(
            &e.pool,
            &e.owner,
            Id::parse(&sid).unwrap(),
            "available",
            "skip inspection"
        )
        .await
        .is_err(),
        "no back door to available stock"
    );
    assert_eq!(bal(&e, code::CASH).await, 5_200_000, "no money moved yet");
    assert_eq!(bal(&e, code::SALES_RETURNS).await, 0);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        0
    );
    assert_eq!(
        bal(&e, code::COGS).await,
        0,
        "goods back in our hands: cost of sale reversed"
    );
    assert_books(&e).await;
    // the clerk can request but cannot approve; the manager can
    let mgr = user(&e, "mgr", "Manager").await;
    let item = return_items(&e, ret).await;
    assert!(forbidden(
        returns::approve_return(
            &e.pool,
            &clerk,
            ret,
            approve(dec(&[(&item[0], "restock")]), 0, "k")
        )
        .await
    ));
    assert!(forbidden(
        returns::reject_return(&e.pool, &clerk, ret, "no").await
    ));
    assert_eq!(
        serial_status(&e, "F-1").await,
        "returned_pending_inspection"
    );
    // approve: restock, full cash refund
    let res = returns::approve_return(
        &e.pool,
        &mgr,
        ret,
        approve(dec(&[(&item[0], "restock")]), 5_200_000, "a1"),
    )
    .await
    .unwrap();
    assert_eq!(res.value, 5_200_000);
    assert!(res.refund_receipt.is_some());
    assert_eq!(serial_status(&e, "F-1").await, "available");
    assert_eq!(
        (
            avail(&e, &e.fridge, "available").await,
            avail(&e, &e.fridge, "pending_inspection").await
        ),
        (6, 0)
    );
    assert_eq!(
        (
            bal(&e, code::CASH).await,
            bal(&e, code::SALES_RETURNS).await,
            bal(&e, code::AR).await
        ),
        (0, 5_200_000, 0)
    );
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        0
    );
    let d = sales::get_sale(&e.pool, &e.owner, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (
            d.returned_total,
            d.sale.grand_total,
            d.sale.paid,
            d.sale.due
        ),
        (5_200_000, 0, 0, 0),
        "invoice nets to zero after a full return and refund"
    );
    assert_eq!(audit_count(&e.pool, "return.requested").await, 1);
    assert_eq!(audit_count(&e.pool, "return.approved").await, 1);
    assert_books(&e).await;
    // the same unit can now be sold again
    sell_fridge(&e, "F-1", None, 5_200_000, "resell", None).await;
    // approving twice or re-returning is refused
    assert!(matches!(
        returns::approve_return(
            &e.pool,
            &mgr,
            ret,
            approve(dec(&[(&item[0], "restock")]), 0, "k2")
        )
        .await,
        Err(CoreError::Conflict(_))
    ));
    assert!(matches!(
        returns::reject_return(&e.pool, &mgr, ret, "x").await,
        Err(CoreError::Conflict(_))
    ));
    assert!(
        matches!(
            returns::record_return(&e.pool, &clerk, rec(&r.id, vec![(&items[0], 1)], "r2")).await,
            Err(CoreError::Conflict(_))
        ),
        "already returned"
    );
}

#[tokio::test]
async fn inspection_outcomes_route_stock_and_post_the_right_journals() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    let a = sell_fridge(&e, "F-1", None, 5_200_000, "a", None).await;
    let b = sell_fridge(&e, "F-2", None, 5_200_000, "b", None).await;
    let c = sell_fridge(&e, "F-3", None, 5_200_000, "c", None).await;
    let mut rets = vec![];
    for (i, (sale, key)) in [(&a, "ra"), (&b, "rb"), (&c, "rc")].into_iter().enumerate() {
        let it = sale_items(&e, &sale.id).await;
        rets.push((
            i,
            returns::record_return(&e.pool, &mgr, rec(&sale.id, vec![(&it[0], 1)], key))
                .await
                .unwrap(),
        ));
    }
    let item = |r: Id| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT id FROM sale_return_items WHERE return_id = ?")
                .bind(r.to_string())
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    // damaged: back in the damaged bucket, value stays in inventory
    let r0 = rets[0].1;
    returns::approve_return(
        &e.pool,
        &mgr,
        r0,
        approve(dec(&[(&item(r0).await, "damaged")]), 5_200_000, "k0"),
    )
    .await
    .unwrap();
    assert_eq!(serial_status(&e, "F-1").await, "damaged");
    assert_eq!(avail(&e, &e.fridge, "damaged").await, 1);
    // write-off needs a note and books the loss
    let r1 = rets[1].1;
    let bad = ApproveIn {
        decisions: vec![DecisionIn {
            item_id: item(r1).await,
            outcome: "write_off".into(),
            note: " ".into(),
        }],
        ..approve(vec![], 0, "k1x")
    };
    assert!(returns::approve_return(&e.pool, &mgr, r1, bad)
        .await
        .is_err());
    returns::approve_return(
        &e.pool,
        &mgr,
        r1,
        approve(dec(&[(&item(r1).await, "write_off")]), 5_200_000, "k1"),
    )
    .await
    .unwrap();
    assert_eq!(serial_status(&e, "F-2").await, "written_off");
    assert_eq!(
        bal(&e, code::INVENTORY_ADJUSTMENTS).await,
        4_000_000,
        "unit cost lost"
    );
    // incomplete / unknown decisions are refused and change nothing
    let r2 = rets[2].1;
    let i2 = item(r2).await;
    assert!(
        returns::approve_return(&e.pool, &mgr, r2, approve(vec![], 0, "k2a"))
            .await
            .is_err(),
        "every item needs an outcome"
    );
    assert!(
        returns::approve_return(&e.pool, &mgr, r2, approve(dec(&[(&i2, "melt")]), 0, "k2b"))
            .await
            .is_err()
    );
    assert!(returns::approve_return(
        &e.pool,
        &mgr,
        r2,
        approve(dec(&[(&i2, "restock"), ("ghost", "restock")]), 0, "k2c")
    )
    .await
    .is_err());
    assert_eq!(
        serial_status(&e, "F-3").await,
        "returned_pending_inspection"
    );
    assert_books(&e).await;
}

#[tokio::test]
async fn refunds_are_capped_by_what_the_customer_has_overpaid() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("sales.credit_limit_enforced".into(), "false".into())]),
    )
    .await
    .unwrap();
    let c = customers::create_customer(&e.pool, &e.owner, cust("Dealer"))
        .await
        .unwrap();
    // wholesale invoice 2 fridges = 96,000; customer paid only 30,000
    let ids = [serial_id(&e, "F-1").await, serial_id(&e, "F-2").await];
    let r = sales::create_sale(
        &e.pool,
        &e.owner,
        si(
            "wholesale",
            Some(c),
            vec![SaleLineIn {
                variant_id: e.fridge.clone(),
                qty: 2,
                discount: 0,
                serial_ids: ids.to_vec(),
            }],
            vec![pay("bank", 3_000_000)],
            "w",
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!((r.grand_total, r.due), (9_600_000, 6_600_000));
    let it = sale_items(&e, &r.id).await;
    assert_eq!(it.len(), 2, "one row per unit");
    let ret = returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], "r1"))
        .await
        .unwrap();
    let ri = return_items(&e, ret).await;
    // returning one unit (48,000) leaves 48,000 invoiced against 30,000 paid -> nothing to refund
    let too_much = returns::approve_return(
        &e.pool,
        &mgr,
        ret,
        approve(dec(&[(&ri[0], "restock")]), 1, "x1"),
    )
    .await;
    assert!(
        matches!(too_much, Err(CoreError::Validation(_))),
        "{too_much:?}"
    );
    returns::approve_return(
        &e.pool,
        &mgr,
        ret,
        approve(dec(&[(&ri[0], "restock")]), 0, "x2"),
    )
    .await
    .unwrap();
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        4_800_000 - 3_000_000,
        "credit note reduces what the dealer owes"
    );
    let d = sales::get_sale(&e.pool, &e.owner, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (d.sale.grand_total, d.sale.paid, d.sale.due),
        (4_800_000, 3_000_000, 1_800_000)
    );
    // returning the second unit makes the 30,000 paid exceed the (now zero) invoice: up to 30,000 may be refunded
    let ret2 = returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[1], 1)], "r2"))
        .await
        .unwrap();
    let ri2 = return_items(&e, ret2).await;
    assert!(returns::approve_return(
        &e.pool,
        &mgr,
        ret2,
        approve(dec(&[(&ri2[0], "restock")]), 3_000_001, "y1")
    )
    .await
    .is_err());
    let mut bank = approve(dec(&[(&ri2[0], "restock")]), 3_000_000, "y2");
    bank.refund_method = "bank".into();
    returns::approve_return(&e.pool, &mgr, ret2, bank)
        .await
        .unwrap();
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        0
    );
    assert_eq!(bal(&e, code::BANK).await, 0);
    assert_books(&e).await;
    // refund needs a valid method
    let r3 = sell_fridge(&e, "F-3", None, 5_200_000, "s3", None).await;
    let i3 = sale_items(&e, &r3.id).await;
    let ret3 = returns::record_return(&e.pool, &mgr, rec(&r3.id, vec![(&i3[0], 1)], "r3"))
        .await
        .unwrap();
    let ri3 = return_items(&e, ret3).await;
    let mut bad = approve(dec(&[(&ri3[0], "restock")]), 100, "z");
    bad.refund_method = "barter".into();
    assert!(returns::approve_return(&e.pool, &mgr, ret3, bad)
        .await
        .is_err());
}

#[tokio::test]
async fn partial_quantity_returns_of_non_serial_goods_add_up_exactly_with_tax() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([
            ("tax.enabled".into(), "true".into()),
            ("tax.default_rate_bp".into(), "750".into()),
        ]),
    )
    .await
    .unwrap();
    let r = sales::create_sale(
        &e.pool,
        &e.owner,
        si(
            "retail",
            None,
            vec![SaleLineIn {
                variant_id: e.fan.clone(),
                qty: 3,
                discount: 7,
                serial_ids: vec![],
            }],
            vec![pay("cash", 483_743 + 36)],
            "t",
            None,
        ),
    )
    .await;
    // grand total = (450000 - 7) * 1.075 = 483,742.025 -> tax 33,750.5 -> rounds half away
    let r = match r {
        Ok(r) => r,
        Err(_) => {
            let pv = sales::preview_sale(
                &e.pool,
                &e.owner,
                &si(
                    "retail",
                    None,
                    vec![SaleLineIn {
                        variant_id: e.fan.clone(),
                        qty: 3,
                        discount: 7,
                        serial_ids: vec![],
                    }],
                    vec![],
                    "p",
                    None,
                ),
            )
            .await
            .unwrap();
            sales::create_sale(
                &e.pool,
                &e.owner,
                si(
                    "retail",
                    None,
                    vec![SaleLineIn {
                        variant_id: e.fan.clone(),
                        qty: 3,
                        discount: 7,
                        serial_ids: vec![],
                    }],
                    vec![pay("cash", pv.grand_total)],
                    "t2",
                    None,
                ),
            )
            .await
            .unwrap()
        }
    };
    let it = sale_items(&e, &r.id).await;
    let total = r.grand_total;
    let mut values = vec![];
    for (i, q) in [1i64, 1, 1].into_iter().enumerate() {
        let ret = returns::record_return(
            &e.pool,
            &mgr,
            rec(&r.id, vec![(&it[0], q)], &format!("r{i}")),
        )
        .await
        .unwrap();
        let ri = return_items(&e, ret).await;
        let v: i64 = sqlx::query_scalar("SELECT value FROM sale_return_items WHERE id = ?")
            .bind(&ri[0])
            .fetch_one(&e.pool)
            .await
            .unwrap();
        values.push(v);
        returns::approve_return(
            &e.pool,
            &mgr,
            ret,
            approve(dec(&[(&ri[0], "restock")]), v, &format!("a{i}")),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        values.iter().sum::<i64>(),
        total,
        "three partial returns add up to the invoice line to the paisa: {values:?}"
    );
    assert!(matches!(
        returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], "r9")).await,
        Err(CoreError::Conflict(_))
    ));
    let d = sales::get_sale(&e.pool, &e.owner, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!((d.sale.grand_total, d.sale.due), (0, 0));
    assert_eq!(
        bal(&e, code::TAX_PAYABLE).await,
        0,
        "tax collected was reversed in full"
    );
    assert_eq!(
        bal(&e, code::SALES).await - bal(&e, code::SALES_RETURNS).await,
        0
    );
    assert_eq!(avail(&e, &e.fan, "available").await, 20);
    assert_books(&e).await;
}

#[tokio::test]
async fn rejected_returns_hand_the_goods_back_and_reverse_the_cost_entry() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    let r = sell_fridge(&e, "F-1", None, 5_200_000, "s", None).await;
    let it = sale_items(&e, &r.id).await;
    let ret = returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], "r"))
        .await
        .unwrap();
    assert_eq!(bal(&e, code::COGS).await, 0);
    assert!(returns::reject_return(&e.pool, &mgr, ret, " ")
        .await
        .is_err());
    returns::reject_return(
        &e.pool,
        &mgr,
        ret,
        "Customer misused the unit - not covered",
    )
    .await
    .unwrap();
    assert_eq!(serial_status(&e, "F-1").await, "sold");
    assert_eq!(avail(&e, &e.fridge, "pending_inspection").await, 0);
    assert_eq!(
        bal(&e, code::COGS).await,
        4_000_000,
        "the sale stands again"
    );
    assert_eq!(bal(&e, code::SALES_RETURNS).await, 0);
    let d = returns::get_return(&e.pool, &mgr, ret).await.unwrap();
    assert_eq!(d.row.status, "rejected");
    assert_eq!(
        d.decision_note.as_deref(),
        Some("Customer misused the unit - not covered")
    );
    // a rejected return frees the line to be requested again
    returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], "r2"))
        .await
        .unwrap();
    assert_eq!(audit_count(&e.pool, "return.rejected").await, 1);
    assert_books(&e).await;
}

#[tokio::test]
async fn return_validation_idempotency_and_installment_exclusion() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    let r = sell_fridge(&e, "F-1", None, 5_200_000, "s", None).await;
    let it = sale_items(&e, &r.id).await;
    for bad in [
        rec(&r.id, vec![], "a"),
        rec(&r.id, vec![(&it[0], 0)], "b"),
        rec(&r.id, vec![(&it[0], 2)], "c"),
        rec(&r.id, vec![("ghost", 1)], "d"),
        rec("no-such-sale", vec![(&it[0], 1)], "e"),
        rec(&r.id, vec![(&it[0], 1), (&it[0], 1)], "f"),
    ] {
        assert!(returns::record_return(&e.pool, &mgr, bad).await.is_err());
    }
    let mut noreason = rec(&r.id, vec![(&it[0], 1)], "g");
    noreason.reason = " ".into();
    assert!(returns::record_return(&e.pool, &mgr, noreason)
        .await
        .is_err());
    assert!(
        returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], ""))
            .await
            .is_err()
    );
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sale_returns")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let a = returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], "same"))
        .await
        .unwrap();
    let b = returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], "same"))
        .await
        .unwrap();
    assert_eq!(a, b, "duplicate submit");
    // lookups
    let found = returns::find_sale_by_serial(&e.pool, &mgr, "f-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.0, r.id);
    assert!(returns::find_sale_by_serial(&e.pool, &mgr, "NOPE")
        .await
        .unwrap()
        .is_none());
    let items = returns::returnable_items(&e.pool, &mgr, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (items[0].returnable_qty, items[0].serial_status.as_deref()),
        (0, Some("returned_pending_inspection"))
    );
    // installment sales are not supported here and say so
    let c = customers::create_customer(&e.pool, &e.owner, cust("Buyer"))
        .await
        .unwrap();
    let sid = serial_id(&e, "F-2").await;
    let mut inst = si(
        "installment",
        Some(c),
        vec![SaleLineIn {
            variant_id: e.fridge.clone(),
            qty: 1,
            discount: 0,
            serial_ids: vec![sid],
        }],
        vec![pay("cash", 1_000_000)],
        "i",
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
    let isale = sales::create_sale(&e.pool, &e.owner, inst).await.unwrap();
    let iit = sale_items(&e, &isale.id).await;
    assert!(matches!(
        returns::record_return(&e.pool, &mgr, rec(&isale.id, vec![(&iit[0], 1)], "ri")).await,
        Err(CoreError::Conflict(_))
    ));
    // permissions on reads
    let acct = user(&e, "acct", "Accountant").await;
    assert!(forbidden(
        returns::list_returns(&e.pool, &acct, None, None).await
    ));
    assert!(
        returns::list_returns(&e.pool, &mgr, Some("requested"), Some("RET"))
            .await
            .unwrap()
            .len()
            == 1
    );
    assert_books(&e).await;
}

// ------------------------------------------------------------------ warranty

#[tokio::test]
async fn warranty_coverage_cases_and_history() {
    let e = env().await;
    let c = customers::create_customer(&e.pool, &e.owner, cust("Karim"))
        .await
        .unwrap();
    let recent = sell_fridge(&e, "F-1", Some(c), 5_200_000, "a", None).await;
    let old = sell_fridge(&e, "F-2", Some(c), 5_200_000, "b", Some("2024-01-10")).await;
    let _ = (recent, old);
    let sx = user(&e, "sexec", "Sales Executive").await;
    let cov = warranty::lookup_serial(&e.pool, &sx, "F-1")
        .await
        .unwrap()
        .unwrap();
    assert!(
        cov.in_warranty && cov.warranty_months == 12 && cov.customer.as_deref() == Some("Karim")
    );
    let exp =
        chrono::NaiveDate::parse_from_str(cov.expires_on.as_deref().unwrap(), "%Y-%m-%d").unwrap();
    assert_eq!(
        exp,
        time::today_dhaka()
            .checked_add_months(chrono::Months::new(12))
            .unwrap()
    );
    let old_cov = warranty::lookup_serial(&e.pool, &sx, "F-2")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !old_cov.in_warranty,
        "sold in January 2024 with 12 months: expired"
    );
    assert_eq!(old_cov.expires_on.as_deref(), Some("2025-01-10"));
    assert!(warranty::lookup_serial(&e.pool, &sx, "NOPE")
        .await
        .unwrap()
        .is_none());
    let unsold = warranty::lookup_serial(&e.pool, &sx, "F-3")
        .await
        .unwrap()
        .unwrap();
    assert!(unsold.sale_id.is_none() && !unsold.in_warranty);
    // opening cases
    let id = warranty::open_case(
        &e.pool,
        &sx,
        OpenCaseIn {
            serial_no: "F-1".into(),
            issue: "Not cooling".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(
            warranty::open_case(
                &e.pool,
                &sx,
                OpenCaseIn {
                    serial_no: "f-1".into(),
                    issue: "again".into()
                }
            )
            .await,
            Err(CoreError::Conflict(_))
        ),
        "one open case per unit"
    );
    assert!(
        warranty::open_case(
            &e.pool,
            &sx,
            OpenCaseIn {
                serial_no: "F-3".into(),
                issue: "x".into()
            }
        )
        .await
        .is_err(),
        "unsold units have no claim"
    );
    assert!(warranty::open_case(
        &e.pool,
        &sx,
        OpenCaseIn {
            serial_no: "NOPE".into(),
            issue: "x".into()
        }
    )
    .await
    .is_err());
    assert!(warranty::open_case(
        &e.pool,
        &sx,
        OpenCaseIn {
            serial_no: "F-1".into(),
            issue: " ".into()
        }
    )
    .await
    .is_err());
    let oow = warranty::open_case(
        &e.pool,
        &sx,
        OpenCaseIn {
            serial_no: "F-2".into(),
            issue: "Compressor noise".into(),
        },
    )
    .await
    .unwrap();
    let d = warranty::get_case(&e.pool, &sx, oow).await.unwrap();
    assert!(
        !d.case.in_warranty && d.events[0].note.contains("OUT OF WARRANTY"),
        "out-of-warranty claims are recorded as such"
    );
    // lifecycle
    assert!(warranty::update_status(&e.pool, &sx, id, "in_service", " ")
        .await
        .is_err());
    warranty::update_status(
        &e.pool,
        &sx,
        id,
        "in_service",
        "Sent to Singer service centre",
    )
    .await
    .unwrap();
    assert!(matches!(
        warranty::update_status(&e.pool, &sx, id, "open", "back").await,
        Err(CoreError::Conflict(_))
    ));
    warranty::update_status(&e.pool, &sx, id, "resolved", "Replaced thermostat")
        .await
        .unwrap();
    assert!(
        matches!(
            warranty::update_status(&e.pool, &sx, id, "rejected", "oops").await,
            Err(CoreError::Conflict(_))
        ),
        "closed cases are final"
    );
    let d = warranty::get_case(&e.pool, &sx, id).await.unwrap();
    assert_eq!(
        (
            d.case.status.as_str(),
            d.case.resolution.as_deref(),
            d.events.len()
        ),
        ("resolved", Some("Replaced thermostat"), 3)
    );
    assert!(d.events.iter().all(|x| x.actor.as_deref() == Some("sexec")));
    // a new case can be opened after the old one closes
    warranty::open_case(
        &e.pool,
        &sx,
        OpenCaseIn {
            serial_no: "F-1".into(),
            issue: "Door seal".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        warranty::list_cases(&e.pool, &sx, None, Some("F-1"))
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        warranty::list_cases(&e.pool, &sx, Some("resolved"), None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(sqlx::query("UPDATE warranty_events SET note = 'x'")
        .execute(&e.pool)
        .await
        .is_err());
    // permissions
    let acct = user(&e, "acct", "Accountant").await;
    assert!(forbidden(
        warranty::lookup_serial(&e.pool, &acct, "F-1").await
    ));
    let cashier = user(&e, "cash", "Cashier").await;
    assert!(forbidden(
        warranty::open_case(
            &e.pool,
            &cashier,
            OpenCaseIn {
                serial_no: "F-1".into(),
                issue: "x".into()
            }
        )
        .await
    ));
}

// ------------------------------------------------------------------ expenses

async fn cat_id(e: &Env, name: &str) -> String {
    sqlx::query_scalar("SELECT id FROM expense_categories WHERE name = ?")
        .bind(name)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

fn exp(cat: &str, amount: i64, method: &str, key: &str) -> ExpenseIn {
    ExpenseIn {
        category_id: cat.into(),
        expense_date: time::today_dhaka().to_string(),
        amount,
        method: method.into(),
        payee: "Landlord".into(),
        description: "March shop rent".into(),
        client_key: key.into(),
    }
}

#[tokio::test]
async fn expenses_need_approval_from_an_authorised_user_and_post_correctly() {
    let e = env().await;
    let clerk = clerk(&e, "clerk").await;
    let acct = user(&e, "acct", "Accountant").await;
    let rent = cat_id(&e, "Rent").await;
    // a clerk's expense waits; nothing is booked
    let id = expenses::create_expense(&e.pool, &clerk, exp(&rent, 2_500_000, "bank", "k1"))
        .await
        .unwrap();
    assert_eq!(bal(&e, code::BANK).await, 0);
    let list = expenses::list_expenses(&e.pool, &acct, ExpenseFilter::default())
        .await
        .unwrap();
    assert_eq!(
        (
            list.pending_count,
            list.approved_total,
            list.rows[0].status.as_str()
        ),
        (1, 0, "pending")
    );
    assert!(forbidden(
        expenses::approve_expense(&e.pool, &clerk, id).await
    ));
    expenses::approve_expense(&e.pool, &acct, id).await.unwrap();
    assert_eq!(
        (bal(&e, code::BANK).await, bal(&e, "6000").await),
        (-2_500_000, 2_500_000)
    );
    assert!(matches!(
        expenses::approve_expense(&e.pool, &acct, id).await,
        Err(CoreError::Conflict(_))
    ));
    assert!(matches!(
        expenses::reject_expense(&e.pool, &acct, id, "x").await,
        Err(CoreError::Conflict(_))
    ));
    // an approver's own expense posts immediately
    let direct = expenses::create_expense(
        &e.pool,
        &acct,
        exp(&cat_id(&e, "Utilities").await, 120_000, "cash", "k2"),
    )
    .await
    .unwrap();
    assert_eq!(
        (bal(&e, code::CASH).await, bal(&e, "6020").await),
        (-120_000, 120_000)
    );
    // rejection needs a reason and books nothing
    let rej = expenses::create_expense(&e.pool, &clerk, exp(&rent, 999, "cash", "k3"))
        .await
        .unwrap();
    assert!(expenses::reject_expense(&e.pool, &acct, rej, " ")
        .await
        .is_err());
    expenses::reject_expense(&e.pool, &acct, rej, "No receipt")
        .await
        .unwrap();
    assert_eq!(bal(&e, code::CASH).await, -120_000);
    // voiding reverses the journal but keeps the record
    assert!(expenses::void_expense(&e.pool, &acct, direct, " ")
        .await
        .is_err());
    assert!(matches!(
        expenses::void_expense(&e.pool, &acct, rej, "x").await,
        Err(CoreError::Conflict(_))
    ));
    expenses::void_expense(&e.pool, &acct, direct, "Entered twice")
        .await
        .unwrap();
    assert_eq!((bal(&e, code::CASH).await, bal(&e, "6020").await), (0, 0));
    let all = expenses::list_expenses(&e.pool, &acct, ExpenseFilter::default())
        .await
        .unwrap();
    assert_eq!(all.rows.len(), 3);
    assert_eq!(
        all.approved_total, 2_500_000,
        "voided and rejected expenses do not count"
    );
    assert_eq!(audit_count(&e.pool, "expense.voided").await, 1);
    assert_eq!(audit_count(&e.pool, "expense.rejected").await, 1);
    assert_books(&e).await;
}

#[tokio::test]
async fn expense_validation_idempotency_categories_and_permissions() {
    let e = env().await;
    let acct = user(&e, "acct", "Accountant").await;
    let cashier = user(&e, "cash", "Cashier").await;
    let rent = cat_id(&e, "Rent").await;
    let tomorrow = (time::today_dhaka() + Duration::days(1)).to_string();
    let mut future = exp(&rent, 100, "cash", "a");
    future.expense_date = tomorrow;
    for bad in [
        exp(&rent, 0, "cash", "b"),
        exp(&rent, -5, "cash", "c"),
        exp(&rent, 5, "barter", "d"),
        exp(&rent, 5, "cash", ""),
        exp("ghost", 5, "cash", "e"),
        future,
        ExpenseIn {
            description: " ".into(),
            ..exp(&rent, 5, "cash", "f")
        },
        ExpenseIn {
            expense_date: "31/01/2026".into(),
            ..exp(&rent, 5, "cash", "g")
        },
    ] {
        assert!(expenses::create_expense(&e.pool, &acct, bad).await.is_err());
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM expenses")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let a = expenses::create_expense(&e.pool, &acct, exp(&rent, 5_000, "cash", "same"))
        .await
        .unwrap();
    let b = expenses::create_expense(&e.pool, &acct, exp(&rent, 5_000, "cash", "same"))
        .await
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(
        bal(&e, code::CASH).await,
        -5_000,
        "duplicate submit posts once"
    );
    // categories: only expense accounts, unique names, deactivation blocks new use
    assert!(
        expenses::create_category(&e.pool, &acct, "Bad", "1000")
            .await
            .is_err(),
        "must map to an expense account"
    );
    assert!(expenses::create_category(&e.pool, &acct, "Bad", "9999")
        .await
        .is_err());
    let fuel = expenses::create_category(&e.pool, &acct, "Fuel", "6030")
        .await
        .unwrap();
    assert!(matches!(
        expenses::create_category(&e.pool, &acct, "fuel", "6030").await,
        Err(CoreError::Conflict(_))
    ));
    expenses::set_category_active(&e.pool, &acct, fuel, false)
        .await
        .unwrap();
    assert!(
        expenses::create_expense(&e.pool, &acct, exp(&fuel.to_string(), 5, "cash", "h"))
            .await
            .is_err()
    );
    // permissions
    assert!(forbidden(
        expenses::create_expense(&e.pool, &cashier, exp(&rent, 5, "cash", "i")).await
    ));
    assert!(forbidden(
        expenses::list_expenses(&e.pool, &cashier, ExpenseFilter::default()).await
    ));
    assert!(forbidden(
        expenses::create_category(&e.pool, &cashier, "X", "6900").await
    ));
    assert!(forbidden(
        expenses::void_expense(&e.pool, &cashier, a, "x").await
    ));
    assert_books(&e).await;
}

// ------------------------------------------------------------------ cash sessions

#[tokio::test]
async fn cash_session_expected_cash_counts_only_cash_movements_in_the_session() {
    let e = env().await;
    let clerk = clerk(&e, "clerk").await;
    let acct = user(&e, "acct", "Accountant").await;
    assert!(cash::current_session(&e.pool, &acct)
        .await
        .unwrap()
        .is_none());
    let sid = cash::open_session(&e.pool, &clerk, 1_000_000, "")
        .await
        .unwrap();
    assert!(
        matches!(
            cash::open_session(&e.pool, &clerk, 1, "").await,
            Err(CoreError::Conflict(_))
        ),
        "one open session at a time"
    );
    // activity: cash sale 52,000; bank sale 1,500; refund-free; cash expense 1,200; cash supplier payment 3,000
    sell_fridge(&e, "F-1", None, 5_200_000, "s1", None).await;
    sales::create_sale(
        &e.pool,
        &e.owner,
        si(
            "retail",
            None,
            vec![SaleLineIn {
                variant_id: e.fan.clone(),
                qty: 1,
                discount: 0,
                serial_ids: vec![],
            }],
            vec![pay("bank", 150_000)],
            "s2",
            None,
        ),
    )
    .await
    .unwrap();
    expenses::create_expense(
        &e.pool,
        &acct,
        exp(&cat_id(&e, "Utilities").await, 120_000, "cash", "e1"),
    )
    .await
    .unwrap();
    expenses::create_expense(
        &e.pool,
        &acct,
        exp(&cat_id(&e, "Rent").await, 900_000, "bank", "e2"),
    )
    .await
    .unwrap();
    let sup = suppliers::create_supplier(
        &e.pool,
        &e.owner,
        SupplierIn {
            name: "S".into(),
            contact_person: "".into(),
            phone: "".into(),
            alt_phone: "".into(),
            email: "".into(),
            address: "".into(),
            notes: "".into(),
        },
    )
    .await
    .unwrap();
    purchases::pay_supplier(
        &e.pool,
        &e.owner,
        PaymentIn {
            supplier_id: sup.to_string(),
            purchase_id: None,
            amount: 300_000,
            method: "cash".into(),
            paid_on: time::today_dhaka().to_string(),
            reference: "".into(),
            note: "".into(),
            client_key: "p1".into(),
        },
    )
    .await
    .unwrap();
    let sum = cash::session_summary(&e.pool, &clerk, sid).await.unwrap();
    assert_eq!(sum.cash_sale_receipts, 5_200_000);
    assert_eq!(
        (
            sum.cash_expenses,
            sum.cash_supplier_payments,
            sum.cash_refunds
        ),
        (120_000, 300_000, 0)
    );
    assert_eq!(
        sum.expected_cash,
        1_000_000 + 5_200_000 - 120_000 - 300_000,
        "bank/wallet receipts and bank expenses never touch the drawer"
    );
    assert_eq!(
        sum.sales_by_mode,
        vec![("retail".to_string(), 2, 5_350_000)]
    );
    assert!(sum
        .receipts_by_method
        .iter()
        .any(|m| m.0 == "bank" && m.1 == 150_000));
    assert_eq!(sum.expenses_total, 1_020_000);
    // a cash refund inside the session reduces expected cash
    let mgr = user(&e, "mgr", "Manager").await;
    let r = sell_fridge(&e, "F-2", None, 5_200_000, "s3", None).await;
    let it = sale_items(&e, &r.id).await;
    let ret = returns::record_return(&e.pool, &mgr, rec(&r.id, vec![(&it[0], 1)], "rr"))
        .await
        .unwrap();
    let ri = return_items(&e, ret).await;
    returns::approve_return(
        &e.pool,
        &mgr,
        ret,
        approve(dec(&[(&ri[0], "restock")]), 5_200_000, "ra"),
    )
    .await
    .unwrap();
    let sum = cash::session_summary(&e.pool, &clerk, sid).await.unwrap();
    assert_eq!(sum.cash_refunds, 5_200_000);
    assert_eq!(
        sum.expected_cash,
        1_000_000 + 5_200_000 + 5_200_000 - 5_200_000 - 120_000 - 300_000
    );
    // the books' cash account matches the drawer (opening balance of the drawer aside)
    assert_eq!(bal(&e, code::CASH).await, sum.expected_cash - 1_000_000);
}

#[tokio::test]
async fn closing_a_session_records_variance_locks_it_and_corrects_the_books() {
    let e = env().await;
    let clerk = clerk(&e, "clerk").await;
    accounting::opening_balance(&e.pool, &e.owner, "1000", 1_000_000, "2026-01-01")
        .await
        .unwrap();
    let sid = cash::open_session(&e.pool, &clerk, 1_000_000, "")
        .await
        .unwrap();
    sell_fridge(&e, "F-1", None, 5_200_000, "s", None).await;
    // expected 62,000; counted 61,500 -> shortage of 500 needs an explanation
    assert!(cash::close_session(&e.pool, &clerk, sid, 6_150_000, " ")
        .await
        .is_err());
    assert!(cash::close_session(&e.pool, &clerk, sid, -1, "x")
        .await
        .is_err());
    let sum = cash::close_session(
        &e.pool,
        &clerk,
        sid,
        6_150_000,
        "Counted twice; 500 short, change given in error",
    )
    .await
    .unwrap();
    assert_eq!(
        (
            sum.session.status.as_str(),
            sum.session.expected_cash,
            sum.session.counted_cash,
            sum.session.variance
        ),
        ("closed", Some(6_200_000), Some(6_150_000), Some(-50_000))
    );
    assert_eq!(bal(&e, code::CASH_OVER_SHORT).await, 50_000);
    assert_eq!(
        bal(&e, code::CASH).await,
        6_150_000,
        "the books now equal the counted cash"
    );
    assert_books(&e).await;
    // locked: no second close, no edits, no deletes
    assert!(matches!(
        cash::close_session(&e.pool, &clerk, sid, 6_150_000, "again").await,
        Err(CoreError::Conflict(_))
    ));
    assert!(sqlx::query("UPDATE cash_sessions SET counted_cash = 1")
        .execute(&e.pool)
        .await
        .is_err());
    assert!(sqlx::query("DELETE FROM cash_sessions")
        .execute(&e.pool)
        .await
        .is_err());
    assert_eq!(audit_count(&e.pool, "cash.session_closed").await, 1);
    // next day: opening cash must match the last count unless explained
    assert!(cash::open_session(&e.pool, &clerk, 6_000_000, "")
        .await
        .is_err());
    let next = cash::open_session(&e.pool, &clerk, 6_000_000, "Owner took 1,500 home")
        .await
        .unwrap();
    // an exact count needs no note and books nothing
    let s2 = cash::close_session(&e.pool, &clerk, next, 6_000_000, "")
        .await
        .unwrap();
    assert_eq!(s2.session.variance, Some(0));
    assert_eq!(bal(&e, code::CASH_OVER_SHORT).await, 50_000);
    // overage
    let third = cash::open_session(&e.pool, &clerk, 6_000_000, "")
        .await
        .unwrap();
    cash::close_session(&e.pool, &clerk, third, 6_010_000, "Found an uncounted note")
        .await
        .unwrap();
    assert_eq!(bal(&e, code::CASH_OVER_SHORT).await, 50_000 - 10_000);
    assert_eq!(bal(&e, code::CASH).await, 6_150_000 + 10_000);
    assert_eq!(cash::list_sessions(&e.pool, &clerk).await.unwrap().len(), 3);
    assert_books(&e).await;
}

#[tokio::test]
async fn requiring_a_session_blocks_cash_movements_but_not_other_methods() {
    let e = env().await;
    let clerk = clerk(&e, "clerk").await;
    let acct = user(&e, "acct", "Accountant").await;
    let cashier = user(&e, "cash", "Cashier").await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("cash.require_session".into(), "true".into())]),
    )
    .await
    .unwrap();
    let fan_sale = |key: &str, method: &str| {
        si(
            "retail",
            None,
            vec![SaleLineIn {
                variant_id: e.fan.clone(),
                qty: 1,
                discount: 0,
                serial_ids: vec![],
            }],
            vec![pay(method, 150_000)],
            key,
            None,
        )
    };
    let blocked = sales::create_sale(&e.pool, &cashier, fan_sale("a", "cash")).await;
    assert!(
        matches!(blocked, Err(CoreError::Conflict(_))),
        "{blocked:?}"
    );
    assert!(
        sales::create_sale(&e.pool, &cashier, fan_sale("b", "bank"))
            .await
            .is_ok(),
        "bank sales are not cash-drawer movements"
    );
    let rent = cat_id(&e, "Rent").await;
    assert!(
        expenses::create_expense(&e.pool, &acct, exp(&rent, 100, "cash", "x"))
            .await
            .is_err()
    );
    assert!(
        expenses::create_expense(&e.pool, &acct, exp(&rent, 100, "bank", "y"))
            .await
            .is_ok()
    );
    let sid = cash::open_session(&e.pool, &clerk, 0, "").await.unwrap();
    assert!(sales::create_sale(&e.pool, &cashier, fan_sale("c", "cash"))
        .await
        .is_ok());
    cash::close_session(&e.pool, &clerk, sid, 150_000, "")
        .await
        .unwrap();
    assert!(
        sales::create_sale(&e.pool, &cashier, fan_sale("d", "cash"))
            .await
            .is_err(),
        "closed again"
    );
    // permissions
    assert!(forbidden(
        cash::open_session(&e.pool, &acct_without_cash(&acct), 0, "").await
    ));
    assert!(forbidden(
        cash::close_session(&e.pool, &cashier_without_cash(&cashier), Id::new(), 0, "").await
    ));
    assert_books(&e).await;
}

fn acct_without_cash(p: &Principal) -> Principal {
    let mut q = p.clone();
    q.permissions.remove("cash.session");
    q
}

fn cashier_without_cash(p: &Principal) -> Principal {
    let mut q = p.clone();
    q.permissions.remove("cash.session");
    q
}

#[tokio::test]
async fn new_installs_seed_expense_categories_and_the_over_short_account() {
    let e = env().await;
    let cats = expenses::list_categories(&e.pool, &e.owner).await.unwrap();
    assert!(
        cats.len() >= 9
            && cats
                .iter()
                .any(|c| c.name == "Rent" && c.account_code == "6000")
    );
    let accts = accounting::list_accounts(&e.pool, &e.owner, None)
        .await
        .unwrap();
    assert!(accts
        .iter()
        .any(|a| a.code == "6910" && a.account_type == "expense"));
    // seeding is idempotent
    expenses::seed_categories(&e.pool).await.unwrap();
    assert_eq!(
        expenses::list_categories(&e.pool, &e.owner)
            .await
            .unwrap()
            .len(),
        cats.len()
    );
}
