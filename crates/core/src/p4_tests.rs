//! P4 test-suite: wholesale pricing and installment agreements (schedule, collections, overdue,
//! settlement, rescheduling, late fees, reversals, accounting).

use crate::accounting::{self, code};
use crate::auth;
use crate::authz::{self, Principal};
use crate::catalogue::{self, NewProduct, NewVariant};
use crate::customers::{self, CustomerIn};
use crate::installments::{
    self, AgreementFilter, CollectIn, CustomDue, FeeIn, InstallmentIn, RescheduleIn, SettleIn,
};
use crate::inventory::{self, OpeningStockIn};
use crate::pricing;
use crate::recovery::DataPaths;
use crate::sales::{self, PayLineIn, SaleIn, SaleLineIn};
use crate::users::{self, NewUser};
use crate::{db, settings, time, CoreError, Id};
use chrono::{Duration, NaiveDate};
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
    let mk = |name: &str, serialized: bool, v: NewVariant| NewProduct {
        brand_id: brand.to_string(),
        category_id: cat.to_string(),
        name: name.into(),
        model: "".into(),
        description: "".into(),
        warranty_months: 12,
        warranty_terms: "".into(),
        min_stock: 0,
        is_serialized: serialized,
        variants: vec![v],
    };
    let v = |sku: &str, cost, retail, wholesale, inst| NewVariant {
        sku: sku.into(),
        barcode: None,
        colour: "".into(),
        capacity: "".into(),
        size: "".into(),
        purchase_cost: cost,
        retail_price: retail,
        wholesale_price: wholesale,
        installment_price: inst,
    };
    let fridge_p = catalogue::create_product(
        &pool,
        &owner,
        mk(
            "Fridge",
            true,
            v("FRIDGE", 4_000_000, 5_200_000, 4_800_000, Some(6_000_000)),
        ),
    )
    .await
    .unwrap();
    let fan_p = catalogue::create_product(
        &pool,
        &owner,
        mk("Fan", false, v("FAN", 100_000, 150_000, 130_000, None)),
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
    let serials: Vec<String> = (1..=8).map(|i| format!("F-{i}")).collect();
    inventory::record_opening_stock(
        &pool,
        &owner,
        OpeningStockIn {
            variant_id: fridge.clone(),
            qty: 0,
            unit_cost: 4_000_000,
            serials: serials.clone(),
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
    let tb = accounting::trial_balance(&e.pool, &e.owner, None)
        .await
        .unwrap();
    assert!(tb.balanced);
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

async fn new_customer(e: &Env, name: &str) -> Id {
    customers::create_customer(&e.pool, &e.owner, cust(name))
        .await
        .unwrap()
}

async fn serial_id(e: &Env, sn: &str) -> String {
    sqlx::query_scalar("SELECT id FROM product_serials WHERE serial_no = ?")
        .bind(sn)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

fn plus_days(n: i64) -> String {
    (time::today_dhaka() + Duration::days(n)).to_string()
}

fn terms(count: i64, freq: &str, first: &str) -> InstallmentIn {
    InstallmentIn {
        markup_bp: None,
        count,
        frequency: freq.into(),
        first_due_date: first.into(),
        custom: vec![],
        fees: vec![],
        notes: "".into(),
    }
}

async fn isale(
    e: &Env,
    customer: Id,
    serial: &str,
    dp: i64,
    sale_date: Option<&str>,
    t: InstallmentIn,
    key: &str,
) -> Result<sales::SaleResult, CoreError> {
    let sid = serial_id(e, serial).await;
    let payments = if dp > 0 {
        vec![PayLineIn {
            method: "cash".into(),
            amount: dp,
            reference: "".into(),
        }]
    } else {
        vec![]
    };
    sales::create_sale(
        &e.pool,
        &e.owner,
        SaleIn {
            mode: "installment".into(),
            customer_id: Some(customer.to_string()),
            sale_date: sale_date.map(str::to_string),
            items: vec![SaleLineIn {
                variant_id: e.fridge.clone(),
                qty: 1,
                discount: 0,
                serial_ids: vec![sid],
            }],
            payments,
            notes: "".into(),
            client_key: key.to_string(),
            credit_override: false,
            installment: Some(t),
        },
    )
    .await
}

async fn agreement_of(e: &Env, sale: &sales::SaleResult) -> String {
    sqlx::query_scalar("SELECT id FROM installment_agreements WHERE sale_id = ?")
        .bind(&sale.id)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

fn collect(agreement: &str, amount: i64, key: &str) -> CollectIn {
    CollectIn {
        agreement_id: agreement.into(),
        amount,
        method: "cash".into(),
        reference: "".into(),
        paid_on: None,
        target_schedule_id: None,
        client_key: key.into(),
    }
}

async fn detail(e: &Env, agreement: &str) -> installments::AgreementDetail {
    installments::get_agreement(&e.pool, &e.owner, Id::parse(agreement).unwrap())
        .await
        .unwrap()
}

// ------------------------------------------------------------------ pure functions

#[test]
fn schedules_reconcile_exactly_with_remainder_on_the_last_installment() {
    let first = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    let s = installments::build_schedule("monthly", first, 6, &[], 5_000_000).unwrap();
    assert_eq!(
        s.iter().map(|x| x.1).collect::<Vec<_>>(),
        vec![833_333, 833_333, 833_333, 833_333, 833_333, 833_335]
    );
    assert_eq!(s.iter().map(|x| x.1).sum::<i64>(), 5_000_000);
    // month-end dates clamp but do not drift: 31 Jan, 28 Feb, 31 Mar...
    assert_eq!(
        s.iter().map(|x| x.0.to_string()).collect::<Vec<_>>(),
        vec![
            "2026-01-31",
            "2026-02-28",
            "2026-03-31",
            "2026-04-30",
            "2026-05-31",
            "2026-06-30"
        ]
    );
    let w = installments::build_schedule("weekly", first, 3, &[], 100).unwrap();
    assert_eq!(
        w.iter().map(|x| x.0.to_string()).collect::<Vec<_>>(),
        vec!["2026-01-31", "2026-02-07", "2026-02-14"]
    );
    assert_eq!(w.iter().map(|x| x.1).collect::<Vec<_>>(), vec![33, 33, 34]);
    for total in [1i64, 7, 99, 100_001, 123_456_789] {
        for n in 1..=36 {
            let s = installments::build_schedule("monthly", first, n, &[], total).unwrap();
            assert_eq!(s.iter().map(|x| x.1).sum::<i64>(), total);
            assert!(s.iter().all(|x| x.1 >= 0));
        }
    }
    let cd = |d: &str, a: i64| CustomDue {
        due_date: d.into(),
        amount: a,
    };
    assert!(installments::build_schedule(
        "custom",
        first,
        0,
        &[cd("2026-02-01", 60), cd("2026-04-01", 40)],
        100
    )
    .is_ok());
    assert!(
        installments::build_schedule(
            "custom",
            first,
            0,
            &[cd("2026-02-01", 60), cd("2026-04-01", 41)],
            100
        )
        .is_err(),
        "must add up"
    );
    assert!(
        installments::build_schedule(
            "custom",
            first,
            0,
            &[cd("2026-04-01", 60), cd("2026-02-01", 40)],
            100
        )
        .is_err(),
        "dates ascending"
    );
    assert!(installments::build_schedule(
        "custom",
        first,
        0,
        &[cd("2026-02-01", 100), cd("2026-02-01", 0)],
        100
    )
    .is_err());
    assert!(installments::build_schedule("custom", first, 0, &[], 100).is_err());
    assert!(installments::build_schedule("monthly", first, 0, &[], 100).is_err());
    assert!(installments::build_schedule("monthly", first, 121, &[], 100).is_err());
    assert!(installments::build_schedule("yearly", first, 3, &[], 100).is_err());
    assert!(installments::build_schedule("monthly", first, 3, &[], 0).is_err());
}

#[test]
fn earned_markup_is_monotonic_bounded_and_exact_at_completion() {
    let (u, t) = (1_001i64, 10_007i64);
    let mut prev = 0;
    for c in (0..=t).step_by(137).chain([t]) {
        let e = installments::earned_markup(u, t, c, 0);
        assert!(e >= prev && e <= u, "c={c} e={e}");
        prev = e;
    }
    assert_eq!(
        installments::earned_markup(u, t, t, 0),
        u,
        "everything is earned once everything is paid"
    );
    assert_eq!(installments::earned_markup(u, t, 0, 0), 0);
    // with a rebate the remainder is earned at completion: total income = markup - rebate
    assert_eq!(installments::earned_markup(u, t, t - 300, 300), u - 300);
    assert!(installments::earned_markup(u, t, t / 2, 0) > 0);
}

// ------------------------------------------------------------------ wholesale pricing

#[tokio::test]
async fn wholesale_tiers_and_customer_prices_follow_the_documented_precedence() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    let se = user(&e, "sexec", "Sales Executive").await;
    let dealer = new_customer(&e, "Dealer").await;
    let retailer = new_customer(&e, "Other").await;
    let preview = |c: Option<Id>, mode: &str| {
        let pool = e.pool.clone();
        let owner = e.owner.clone();
        let fan = e.fan.clone();
        let mode = mode.to_string();
        async move {
            sales::preview_sale(
                &pool,
                &owner,
                &SaleIn {
                    mode,
                    customer_id: c.map(|x| x.to_string()),
                    sale_date: None,
                    items: vec![SaleLineIn {
                        variant_id: fan,
                        qty: 1,
                        discount: 0,
                        serial_ids: vec![],
                    }],
                    payments: vec![],
                    notes: "".into(),
                    client_key: "p".into(),
                    credit_override: false,
                    installment: None,
                },
            )
            .await
            .unwrap()
            .lines[0]
                .unit_price
        }
    };
    assert_eq!(
        preview(Some(dealer), "wholesale").await,
        130_000,
        "list wholesale price"
    );
    // permissions & validation
    assert!(forbidden(
        pricing::create_tier(&e.pool, &se, "Gold", -300).await
    ));
    assert!(pricing::create_tier(&e.pool, &mgr, "", 0).await.is_err());
    assert!(
        pricing::create_tier(&e.pool, &mgr, "Wild", 6000)
            .await
            .is_err(),
        "limited to +-50%"
    );
    let gold = pricing::create_tier(&e.pool, &mgr, "Gold dealer", -300)
        .await
        .unwrap();
    assert!(matches!(
        pricing::create_tier(&e.pool, &mgr, "gold DEALER", 0).await,
        Err(CoreError::Conflict(_))
    ));
    assert!(forbidden(
        pricing::set_customer_tier(&e.pool, &se, dealer, Some(gold)).await
    ));
    pricing::set_customer_tier(&e.pool, &mgr, dealer, Some(gold))
        .await
        .unwrap();
    assert_eq!(
        preview(Some(dealer), "wholesale").await,
        126_100,
        "-3% of 1300.00"
    );
    assert_eq!(
        preview(Some(retailer), "wholesale").await,
        130_000,
        "other customers unaffected"
    );
    assert_eq!(
        preview(Some(dealer), "retail").await,
        150_000,
        "retail ignores tiers"
    );
    // customer-specific price beats the tier
    pricing::set_customer_price(&e.pool, &mgr, dealer, Id::parse(&e.fan).unwrap(), 120_000)
        .await
        .unwrap();
    assert_eq!(preview(Some(dealer), "wholesale").await, 120_000);
    assert!(
        pricing::set_customer_price(&e.pool, &mgr, dealer, Id::parse(&e.fan).unwrap(), -1)
            .await
            .is_err()
    );
    let list = pricing::list_customer_prices(&e.pool, &se, dealer)
        .await
        .unwrap();
    assert_eq!(
        (list.len(), list[0].price, list[0].wholesale_price),
        (1, 120_000, 130_000)
    );
    pricing::remove_customer_price(&e.pool, &mgr, dealer, Id::parse(&e.fan).unwrap())
        .await
        .unwrap();
    assert!(
        pricing::remove_customer_price(&e.pool, &mgr, dealer, Id::parse(&e.fan).unwrap())
            .await
            .is_err()
    );
    assert_eq!(preview(Some(dealer), "wholesale").await, 126_100);
    // an inactive tier stops applying; changes are audited
    pricing::update_tier(&e.pool, &mgr, gold, -300, false)
        .await
        .unwrap();
    assert_eq!(preview(Some(dealer), "wholesale").await, 130_000);
    pricing::update_tier(&e.pool, &mgr, gold, 200, true)
        .await
        .unwrap();
    assert_eq!(preview(Some(dealer), "wholesale").await, 132_600, "+2%");
    assert!(audit_count(&e.pool, "pricing.tier_changed").await >= 2);
    assert_eq!(audit_count(&e.pool, "pricing.customer_price_set").await, 1);
    // the posted sale uses the same price as the preview
    let r = sales::create_sale(
        &e.pool,
        &mgr,
        SaleIn {
            mode: "wholesale".into(),
            customer_id: Some(dealer.to_string()),
            sale_date: None,
            items: vec![SaleLineIn {
                variant_id: e.fan.clone(),
                qty: 10,
                discount: 0,
                serial_ids: vec![],
            }],
            payments: vec![PayLineIn {
                method: "cash".into(),
                amount: 1_326_000,
                reference: "".into(),
            }],
            notes: "".into(),
            client_key: "w".into(),
            credit_override: false,
            installment: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(r.grand_total, 1_326_000);
    assert_books(&e).await;
    let tiers = pricing::list_tiers(&e.pool, &se).await.unwrap();
    assert_eq!((tiers.len(), tiers[0].customers), (1, 1));
}

// ------------------------------------------------------------------ agreement creation

#[tokio::test]
async fn agreement_discloses_every_component_and_books_receivables_not_cash() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let r = isale(
        &e,
        c,
        "F-1",
        1_000_000,
        Some("2026-03-01"),
        terms(6, "monthly", "2026-04-01"),
        "a1",
    )
    .await
    .unwrap();
    let ag = agreement_of(&e, &r).await;
    let d = detail(&e, &ag).await;
    // cash price (retail 52,000), installment price (60,000), down payment, financed amount, markup, total payable: all separate
    assert_eq!(
        (
            d.cash_price,
            d.price_total,
            d.agreement.down_payment,
            d.principal
        ),
        (5_200_000, 6_000_000, 1_000_000, 5_000_000)
    );
    assert_eq!(
        (d.markup_bp, d.markup, d.fees.len(), d.late_fees_total),
        (0, 0, 0, 0),
        "default markup is 0% and no fee is ever implicit"
    );
    assert_eq!(
        (
            d.agreement.total_payable,
            d.agreement.outstanding,
            d.agreement.collected
        ),
        (5_000_000, 5_000_000, 0)
    );
    assert_eq!(d.items.len(), 6);
    assert_eq!(d.items.iter().map(|i| i.amount).sum::<i64>(), 5_000_000);
    assert_eq!(
        d.items[5].amount, 833_335,
        "remainder on the last installment"
    );
    assert_eq!(d.agreement.number, "INS-000001");
    // accounting: only the down payment is cash; the financed amount is an installment receivable; nothing unearned
    assert_eq!(bal(&e, code::CASH).await, 1_000_000);
    assert_eq!(bal(&e, code::INSTALLMENT_AR).await, 5_000_000);
    assert_eq!(bal(&e, code::AR).await, 0);
    assert_eq!(bal(&e, code::SALES).await, 6_000_000);
    assert_eq!(bal(&e, code::UNEARNED_MARKUP).await, 0);
    assert_eq!(bal(&e, code::MARKUP_INCOME).await, 0);
    assert_eq!(bal(&e, code::COGS).await, 4_000_000);
    // the customer owes exactly the financed amount; serial is sold; stock reconciles
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        5_000_000
    );
    let st: String =
        sqlx::query_scalar("SELECT status FROM product_serials WHERE serial_no = 'F-1'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(st, "sold");
    assert_books(&e).await;
    // the invoice lists contract totals; generic payments skip installment invoices
    let sale = sales::get_sale(&e.pool, &e.owner, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (
            sale.financing_charges,
            sale.sale.grand_total,
            sale.sale.paid,
            sale.sale.due
        ),
        (0, 6_000_000, 1_000_000, 5_000_000)
    );
    let g = sales::collect_payment(
        &e.pool,
        &e.owner,
        sales::CollectIn {
            customer_id: c.to_string(),
            sale_id: None,
            amount: 100,
            method: "cash".into(),
            reference: "".into(),
            paid_on: None,
            client_key: "g".into(),
        },
    )
    .await;
    assert!(
        g.is_err(),
        "installment dues are collected on the agreement, not as an invoice payment"
    );
    let dp = sale.payments[0].id.clone();
    assert!(matches!(
        sales::reverse_payment(&e.pool, &e.owner, Id::parse(&dp).unwrap(), "x").await,
        Err(CoreError::Conflict(_))
    ));
    assert_eq!(audit_count(&e.pool, "installment.created").await, 1);
}

#[tokio::test]
async fn markup_and_fees_are_explicit_permissioned_and_unearned_until_collected() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let mgr = user(&e, "mgr", "Manager").await;
    let se = user(&e, "sexec", "Sales Executive").await;
    let sale_as = |p: Principal, serial: String, t: InstallmentIn, key: &str| {
        let (pool, fridge, cid, key) = (
            e.pool.clone(),
            e.fridge.clone(),
            c.to_string(),
            key.to_string(),
        );
        async move {
            let sid: String =
                sqlx::query_scalar("SELECT id FROM product_serials WHERE serial_no = ?")
                    .bind(&serial)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            sales::create_sale(
                &pool,
                &p,
                SaleIn {
                    mode: "installment".into(),
                    customer_id: Some(cid),
                    sale_date: None,
                    items: vec![SaleLineIn {
                        variant_id: fridge,
                        qty: 1,
                        discount: 0,
                        serial_ids: vec![sid],
                    }],
                    payments: vec![PayLineIn {
                        method: "cash".into(),
                        amount: 1_000_000,
                        reference: "".into(),
                    }],
                    notes: "".into(),
                    client_key: key,
                    credit_override: false,
                    installment: Some(t),
                },
            )
            .await
        }
    };
    // a sales executive may sign agreements on default terms but cannot set a markup or add fees
    let mut t = terms(5, "monthly", &plus_days(30));
    t.markup_bp = Some(1200);
    assert!(forbidden(
        sale_as(se.clone(), "F-1".into(), t.clone(), "k1").await
    ));
    let mut t2 = terms(5, "monthly", &plus_days(30));
    t2.fees = vec![FeeIn {
        label: "Documentation".into(),
        amount: 50_000,
    }];
    assert!(forbidden(sale_as(se.clone(), "F-1".into(), t2, "k2").await));
    let sid: String =
        sqlx::query_scalar("SELECT status FROM product_serials WHERE serial_no = 'F-1'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(
        sid, "available",
        "a refused agreement leaves nothing behind"
    );
    assert_eq!(
        sale_as(
            se.clone(),
            "F-1".into(),
            terms(5, "monthly", &plus_days(30)),
            "k3"
        )
        .await
        .unwrap()
        .due,
        5_000_000,
        "default terms are fine"
    );
    // markup 12% on 50,000 = 6,000 + 500 fee -> total payable 56,500, with the parts shown separately
    t.markup_bp = Some(1200);
    t.fees = vec![FeeIn {
        label: "Documentation".into(),
        amount: 50_000,
    }];
    let r = sale_as(mgr.clone(), "F-2".into(), t, "k4").await.unwrap();
    let ag = agreement_of(&e, &r).await;
    let d = detail(&e, &ag).await;
    assert_eq!(
        (
            d.principal,
            d.markup_bp,
            d.markup,
            d.fees.iter().map(|f| f.amount).sum::<i64>()
        ),
        (5_000_000, 1200, 600_000, 50_000)
    );
    assert_eq!(d.agreement.total_payable, 5_650_000);
    assert_eq!(d.items.iter().map(|i| i.amount).sum::<i64>(), 5_650_000);
    // markup is a liability until collected: nothing recognised, receivable carries the full amount
    assert_eq!(bal(&e, code::INSTALLMENT_AR).await, 5_000_000 + 5_650_000);
    assert_eq!(bal(&e, code::UNEARNED_MARKUP).await, 650_000);
    assert_eq!(bal(&e, code::MARKUP_INCOME).await, 0);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        5_000_000 + 5_650_000
    );
    assert_eq!(audit_count(&e.pool, "installment.markup_set").await, 1);
    assert_books(&e).await;
    // changing the shop default changes the default; an explicit equal value needs no permission
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("installment.default_markup_bp".into(), "500".into())]),
    )
    .await
    .unwrap();
    let r5 = sale_as(
        se.clone(),
        "F-3".into(),
        terms(4, "weekly", &plus_days(7)),
        "k5",
    )
    .await
    .unwrap();
    let d5 = detail(&e, &agreement_of(&e, &r5).await).await;
    assert_eq!(
        (d5.markup_bp, d5.markup),
        (500, 250_000),
        "5% of the 50,000 financed"
    );
    let mut explicit = terms(4, "weekly", &plus_days(7));
    explicit.markup_bp = Some(500);
    assert!(sale_as(se.clone(), "F-4".into(), explicit, "k6")
        .await
        .is_ok());
    assert_books(&e).await;
}

#[tokio::test]
async fn markup_can_be_recognised_at_signing_by_policy() {
    let e = env().await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([
            (
                "installment.markup_recognition".into(),
                "on_agreement".into(),
            ),
            ("installment.default_markup_bp".into(), "1000".into()),
        ]),
    )
    .await
    .unwrap();
    let c = new_customer(&e, "Karim").await;
    isale(
        &e,
        c,
        "F-1",
        1_000_000,
        None,
        terms(5, "monthly", &plus_days(30)),
        "a",
    )
    .await
    .unwrap();
    assert_eq!(
        (
            bal(&e, code::MARKUP_INCOME).await,
            bal(&e, code::UNEARNED_MARKUP).await
        ),
        (500_000, 0)
    );
    assert_books(&e).await;
}

#[tokio::test]
async fn agreement_validation_rules() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let ok = || terms(6, "monthly", &plus_days(30));
    let go = |dp: i64, t: InstallmentIn, key: &str, date: Option<&str>| {
        let key = key.to_string();
        let date = date.map(str::to_string);
        let e = &e;
        async move { isale(e, c, "F-1", dp, date.as_deref(), t, &key).await }
    };
    assert!(
        go(6_000_000, ok(), "a", None).await.is_err(),
        "down payment = price leaves nothing to finance"
    );
    assert!(
        go(1_000_000, terms(0, "monthly", &plus_days(30)), "b", None)
            .await
            .is_err()
    );
    assert!(
        go(1_000_000, terms(121, "monthly", &plus_days(30)), "c", None)
            .await
            .is_err()
    );
    assert!(go(1_000_000, terms(6, "daily", &plus_days(30)), "d", None)
        .await
        .is_err());
    assert!(
        go(1_000_000, terms(6, "monthly", "2026-02-30"), "e", None)
            .await
            .is_err(),
        "invalid date"
    );
    assert!(
        go(
            1_000_000,
            terms(6, "monthly", "2026-01-01"),
            "f",
            Some("2026-03-01")
        )
        .await
        .is_err(),
        "first due before the sale"
    );
    let mut m = ok();
    m.markup_bp = Some(10_001);
    assert!(go(1_000_000, m, "g", None).await.is_err());
    // minimum down payment is a configurable policy
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("installment.min_down_payment_bp".into(), "2000".into())]),
    )
    .await
    .unwrap(); // 20% of 60,000 = 12,000
    assert!(go(1_199_999, ok(), "h", None).await.is_err());
    assert!(go(1_200_000, ok(), "i", None).await.is_ok());
    // a sale needs a customer and the financing terms
    let sid = serial_id(&e, "F-2").await;
    let mk = |customer: Option<String>, inst: Option<InstallmentIn>, key: &str| SaleIn {
        mode: "installment".into(),
        customer_id: customer,
        sale_date: None,
        items: vec![SaleLineIn {
            variant_id: e.fridge.clone(),
            qty: 1,
            discount: 0,
            serial_ids: vec![sid.clone()],
        }],
        payments: vec![PayLineIn {
            method: "cash".into(),
            amount: 1_200_000,
            reference: "".into(),
        }],
        notes: "".into(),
        client_key: key.into(),
        credit_override: false,
        installment: inst,
    };
    assert!(
        sales::create_sale(&e.pool, &e.owner, mk(None, Some(ok()), "j"))
            .await
            .is_err()
    );
    assert!(
        sales::create_sale(&e.pool, &e.owner, mk(Some(c.to_string()), None, "k"))
            .await
            .is_err()
    );
    // only users with the installments permission may create one
    let cashier = user(&e, "cash", "Cashier").await;
    assert!(forbidden(
        sales::create_sale(&e.pool, &cashier, mk(Some(c.to_string()), Some(ok()), "l")).await
    ));
    // idempotent submit
    let a = sales::create_sale(
        &e.pool,
        &e.owner,
        mk(Some(c.to_string()), Some(ok()), "same"),
    )
    .await
    .unwrap();
    let b = sales::create_sale(
        &e.pool,
        &e.owner,
        mk(Some(c.to_string()), Some(ok()), "same"),
    )
    .await
    .unwrap();
    assert_eq!(a.id, b.id);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM installment_agreements")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert_books(&e).await;
}

#[tokio::test]
async fn a_customer_may_hold_several_agreements_that_never_merge() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let a = isale(
        &e,
        c,
        "F-1",
        1_000_000,
        None,
        terms(5, "monthly", &plus_days(30)),
        "a",
    )
    .await
    .unwrap();
    let b = isale(
        &e,
        c,
        "F-2",
        2_000_000,
        None,
        terms(8, "weekly", &plus_days(7)),
        "b",
    )
    .await
    .unwrap();
    let (ia, ib) = (agreement_of(&e, &a).await, agreement_of(&e, &b).await);
    assert_ne!(ia, ib);
    installments::collect(&e.pool, &e.owner, collect(&ia, 700_000, "c1"))
        .await
        .unwrap();
    let (da, db_) = (detail(&e, &ia).await, detail(&e, &ib).await);
    assert_eq!(
        (da.agreement.outstanding, db_.agreement.outstanding),
        (4_300_000, 4_000_000),
        "each agreement keeps its own balance"
    );
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        8_300_000
    );
    let list = installments::list_agreements(
        &e.pool,
        &e.owner,
        AgreementFilter {
            customer_id: Some(c.to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!((list.total, list.outstanding_total), (2, 8_300_000));
    assert_books(&e).await;
}

// ------------------------------------------------------------------ collections

#[tokio::test]
async fn collections_allocate_transparently_partial_advance_and_complete_exactly() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let r = isale(
        &e,
        c,
        "F-1",
        1_000_000,
        None,
        terms(4, "monthly", &plus_days(30)),
        "a",
    )
    .await
    .unwrap(); // 4 x 1,250,000
    let ag = agreement_of(&e, &r).await;
    let items = detail(&e, &ag).await.items;
    assert_eq!(
        items.iter().map(|i| i.amount).collect::<Vec<_>>(),
        vec![1_250_000; 4]
    );
    // partial payment must not mark the installment paid
    let r1 = installments::collect(&e.pool, &e.owner, collect(&ag, 500_000, "p1"))
        .await
        .unwrap();
    assert_eq!(r1.allocations, vec![(1, 500_000)]);
    let d = detail(&e, &ag).await;
    assert_eq!(
        (
            d.items[0].status.as_str(),
            d.items[0].paid,
            d.items[0].remaining
        ),
        ("partial", 500_000, 750_000)
    );
    assert_eq!(d.items[1].status, "pending");
    // a payment larger than the due installment flows to the next ones (advance payment), oldest first
    let r2 = installments::collect(&e.pool, &e.owner, collect(&ag, 2_000_000, "p2"))
        .await
        .unwrap();
    assert_eq!(r2.allocations, vec![(1, 750_000), (2, 1_250_000)]);
    // naming an installment pays it first, then the rest oldest-first
    let target = detail(&e, &ag).await.items[3].id.clone();
    let mut named = collect(&ag, 1_300_000, "p3");
    named.target_schedule_id = Some(target.clone());
    let r3 = installments::collect(&e.pool, &e.owner, named)
        .await
        .unwrap();
    assert_eq!(
        r3.allocations,
        vec![(4, 1_250_000), (3, 50_000)],
        "the named installment is paid first"
    );
    let d = detail(&e, &ag).await;
    assert_eq!(
        d.agreement.outstanding,
        5_000_000 - 500_000 - 2_000_000 - 1_300_000
    );
    assert_eq!(
        d.items
            .iter()
            .map(|i| i.status.as_str())
            .collect::<Vec<_>>(),
        vec!["paid", "paid", "partial", "paid"]
    );
    // over-payment is refused; the exact balance completes the agreement
    assert!(
        installments::collect(&e.pool, &e.owner, collect(&ag, 1_200_001, "p4"))
            .await
            .is_err()
    );
    assert!(
        installments::collect(&e.pool, &e.owner, collect(&ag, 0, "p5"))
            .await
            .is_err()
    );
    assert!(
        installments::collect(&e.pool, &e.owner, collect(&ag, 10, ""))
            .await
            .is_err()
    );
    let mut bad = collect(&ag, 10, "p6");
    bad.method = "barter".into();
    assert!(installments::collect(&e.pool, &e.owner, bad).await.is_err());
    let fin = installments::collect(&e.pool, &e.owner, collect(&ag, 1_200_000, "p7"))
        .await
        .unwrap();
    assert!(fin.completed && fin.outstanding == 0);
    assert_eq!(detail(&e, &ag).await.agreement.status, "completed");
    assert!(matches!(
        installments::collect(&e.pool, &e.owner, collect(&ag, 1, "p8")).await,
        Err(CoreError::Conflict(_))
    ));
    // receipts, customer ledger, cash and receivable all agree
    let d = detail(&e, &ag).await;
    assert_eq!(d.collections.len(), 4);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        0
    );
    assert_eq!(
        (
            bal(&e, code::INSTALLMENT_AR).await,
            bal(&e, code::CASH).await
        ),
        (0, 6_000_000)
    );
    assert_books(&e).await;
    assert_eq!(audit_count(&e.pool, "installment.collected").await, 4);
}

#[tokio::test]
async fn collection_submissions_are_idempotent_even_when_concurrent() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let ag = agreement_of(
        &e,
        &isale(
            &e,
            c,
            "F-1",
            1_000_000,
            None,
            terms(4, "monthly", &plus_days(30)),
            "a",
        )
        .await
        .unwrap(),
    )
    .await;
    let mut hs = vec![];
    for _ in 0..8 {
        let (pool, owner, ag) = (e.pool.clone(), e.owner.clone(), ag.clone());
        hs.push(tokio::spawn(async move {
            installments::collect(&pool, &owner, collect(&ag, 400_000, "same-form"))
                .await
                .map(|r| r.receipt)
        }));
    }
    let mut receipts = std::collections::HashSet::new();
    for h in hs {
        receipts.insert(h.await.unwrap().unwrap());
    }
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        detail(&e, &ag).await.agreement.outstanding,
        4_600_000,
        "taken once"
    );
    assert_books(&e).await;
}

#[tokio::test]
async fn markup_is_earned_in_proportion_to_collections_and_exactly_in_total() {
    let e = env().await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("installment.default_markup_bp".into(), "1111".into())]),
    )
    .await
    .unwrap();
    let c = new_customer(&e, "Karim").await;
    let r = isale(
        &e,
        c,
        "F-1",
        999_999,
        None,
        terms(7, "monthly", &plus_days(30)),
        "a",
    )
    .await
    .unwrap(); // odd numbers on purpose
    let ag = agreement_of(&e, &r).await;
    let d = detail(&e, &ag).await;
    let (u, t) = (
        d.markup + d.fees.iter().map(|f| f.amount).sum::<i64>(),
        d.agreement.total_payable,
    );
    assert_eq!(bal(&e, code::UNEARNED_MARKUP).await, u);
    let mut paid = 0;
    let mut earned_prev = 0;
    for (i, amt) in [333_333i64, 1, 777_777, 1_000_001].into_iter().enumerate() {
        installments::collect(&e.pool, &e.owner, collect(&ag, amt, &format!("k{i}")))
            .await
            .unwrap();
        paid += amt;
        let earned = bal(&e, code::MARKUP_INCOME).await;
        assert_eq!(earned, installments::earned_markup(u, t, paid, 0));
        assert!(earned >= earned_prev && earned <= u);
        assert_eq!(bal(&e, code::UNEARNED_MARKUP).await, u - earned);
        earned_prev = earned;
        assert_books(&e).await;
    }
    let rest = detail(&e, &ag).await.agreement.outstanding;
    installments::collect(&e.pool, &e.owner, collect(&ag, rest, "last"))
        .await
        .unwrap();
    assert_eq!(
        bal(&e, code::MARKUP_INCOME).await,
        u,
        "the whole markup is earned once everything is collected - to the paisa"
    );
    assert_eq!(bal(&e, code::UNEARNED_MARKUP).await, 0);
    assert_books(&e).await;
}

#[tokio::test]
async fn overdue_tracking_grace_followups_and_late_fees() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    let c = new_customer(&e, "Late Payer").await;
    let c2 = new_customer(&e, "On Time").await;
    // agreement signed in January: installments due 15 Jan, 15 Feb, ... all long past
    let late = isale(
        &e,
        c,
        "F-1",
        1_000_000,
        Some("2026-01-01"),
        terms(3, "monthly", "2026-01-15"),
        "a",
    )
    .await
    .unwrap();
    let fine = isale(
        &e,
        c2,
        "F-2",
        1_000_000,
        None,
        terms(3, "monthly", &plus_days(10)),
        "b",
    )
    .await
    .unwrap();
    let (la, fa) = (agreement_of(&e, &late).await, agreement_of(&e, &fine).await);
    let d = detail(&e, &la).await;
    assert!(
        d.items.iter().all(|i| i.status == "overdue")
            && d.agreement.overdue_amount == 5_000_000
            && d.agreement.max_days_overdue > 100
    );
    assert_eq!(detail(&e, &fa).await.agreement.overdue_amount, 0);
    // partial payment on the first installment: still overdue for the unpaid part
    installments::collect(&e.pool, &e.owner, collect(&la, 100_000, "c1"))
        .await
        .unwrap();
    let d = detail(&e, &la).await;
    assert_eq!(d.agreement.overdue_amount, 4_900_000);
    // worklist contains only the late agreement
    let w = installments::overdue_worklist(&e.pool, &e.owner)
        .await
        .unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].customer, "Late Payer");
    let page = installments::list_agreements(
        &e.pool,
        &e.owner,
        AgreementFilter {
            overdue_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!((page.total, page.overdue_total), (1, 4_900_000));
    // follow-up notes
    installments::add_followup(
        &e.pool,
        &e.owner,
        Id::parse(&la).unwrap(),
        "Called; promises Friday",
        Some("2026-12-04"),
    )
    .await
    .unwrap();
    assert!(
        installments::add_followup(&e.pool, &e.owner, Id::parse(&la).unwrap(), "  ", None)
            .await
            .is_err()
    );
    assert!(installments::add_followup(
        &e.pool,
        &e.owner,
        Id::parse(&la).unwrap(),
        "x",
        Some("not-a-date")
    )
    .await
    .is_err());
    assert_eq!(detail(&e, &la).await.followups.len(), 1);
    // grace period delays "overdue"
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("installment.grace_days".into(), "365".into())]),
    )
    .await
    .unwrap();
    assert_eq!(
        detail(&e, &la).await.agreement.overdue_amount,
        0,
        "within the grace period nothing is overdue"
    );
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("installment.grace_days".into(), "0".into())]),
    )
    .await
    .unwrap();
    // late fees: off by default, explicit when on, once per installment, permissioned and audited
    let item = detail(&e, &la).await.items[1].id.clone();
    let item_id = Id::parse(&item).unwrap();
    assert!(
        matches!(
            installments::apply_late_fee(&e.pool, &mgr, Id::parse(&la).unwrap(), item_id, "late")
                .await,
            Err(CoreError::Conflict(_))
        ),
        "late fees must be enabled by an authorised user first"
    );
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([
            ("installment.late_fee_enabled".into(), "true".into()),
            ("installment.late_fee_bp".into(), "250".into()),
        ]),
    )
    .await
    .unwrap();
    let se = user(&e, "sexec", "Sales Executive").await;
    assert!(forbidden(
        installments::apply_late_fee(&e.pool, &se, Id::parse(&la).unwrap(), item_id, "late").await
    ));
    assert!(
        installments::apply_late_fee(&e.pool, &mgr, Id::parse(&la).unwrap(), item_id, " ")
            .await
            .is_err()
    );
    let fee = installments::apply_late_fee(
        &e.pool,
        &mgr,
        Id::parse(&la).unwrap(),
        item_id,
        "15 days late",
    )
    .await
    .unwrap();
    assert_eq!(
        fee, 41_667,
        "2.5% of the 1,666,667 still unpaid... see below"
    );
    let d = detail(&e, &la).await;
    assert_eq!(d.late_fees_total, fee);
    assert_eq!(d.agreement.outstanding, 5_000_000 - 100_000 + fee);
    assert_eq!(d.items[1].late_fee, fee);
    assert_eq!(d.items[1].effective, d.items[1].amount + fee);
    assert!(matches!(
        installments::apply_late_fee(&e.pool, &mgr, Id::parse(&la).unwrap(), item_id, "again")
            .await,
        Err(CoreError::Conflict(_))
    ));
    let on_time_item = Id::parse(&detail(&e, &fa).await.items[0].id).unwrap();
    assert!(
        installments::apply_late_fee(&e.pool, &mgr, Id::parse(&fa).unwrap(), on_time_item, "x")
            .await
            .is_err(),
        "not overdue"
    );
    assert_eq!(bal(&e, code::OTHER_INCOME).await, fee);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        5_000_000 - 100_000 + fee
    );
    assert_eq!(audit_count(&e.pool, "installment.late_fee").await, 1);
    assert_books(&e).await;
}

// ------------------------------------------------------------------ reversals, settlement, rescheduling

#[tokio::test]
async fn reversing_a_collection_restores_balances_and_markup_and_cannot_be_repeated() {
    let e = env().await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("installment.default_markup_bp".into(), "1000".into())]),
    )
    .await
    .unwrap();
    let c = new_customer(&e, "Karim").await;
    let ag = agreement_of(
        &e,
        &isale(
            &e,
            c,
            "F-1",
            1_000_000,
            None,
            terms(4, "monthly", &plus_days(30)),
            "a",
        )
        .await
        .unwrap(),
    )
    .await;
    installments::collect(&e.pool, &e.owner, collect(&ag, 2_750_000, "c1"))
        .await
        .unwrap();
    let col = detail(&e, &ag).await.collections[0].id.clone();
    let earned_before = bal(&e, code::MARKUP_INCOME).await;
    assert!(earned_before > 0);
    let cashier = user(&e, "cash", "Cashier").await;
    assert!(forbidden(
        installments::reverse_collection(&e.pool, &cashier, Id::parse(&col).unwrap(), "x").await
    ));
    assert!(
        installments::reverse_collection(&e.pool, &e.owner, Id::parse(&col).unwrap(), " ")
            .await
            .is_err()
    );
    let rev = installments::reverse_collection(
        &e.pool,
        &e.owner,
        Id::parse(&col).unwrap(),
        "cheque bounced",
    )
    .await
    .unwrap();
    let d = detail(&e, &ag).await;
    assert_eq!(d.agreement.outstanding, 5_500_000, "full balance restored");
    assert!(d.items.iter().all(|i| i.paid == 0));
    assert_eq!(
        (
            bal(&e, code::MARKUP_INCOME).await,
            bal(&e, code::CASH).await
        ),
        (0, 1_000_000),
        "earned markup and cash roll back"
    );
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        5_500_000
    );
    assert!(matches!(
        installments::reverse_collection(&e.pool, &e.owner, Id::parse(&col).unwrap(), "again")
            .await,
        Err(CoreError::Conflict(_) | CoreError::Database(_))
    ));
    assert!(
        installments::reverse_collection(&e.pool, &e.owner, rev, "undo")
            .await
            .is_err()
    );
    assert_eq!(
        d.collections.len(),
        2,
        "original kept; a reversal row added"
    );
    assert_eq!(
        audit_count(&e.pool, "installment.collection_reversed").await,
        1
    );
    // reversing a payment that completed the agreement reopens it
    installments::collect(&e.pool, &e.owner, collect(&ag, 5_500_000, "c2"))
        .await
        .unwrap();
    assert_eq!(detail(&e, &ag).await.agreement.status, "completed");
    let c2 = detail(&e, &ag).await.collections.last().unwrap().id.clone();
    installments::reverse_collection(&e.pool, &e.owner, Id::parse(&c2).unwrap(), "error")
        .await
        .unwrap();
    assert_eq!(detail(&e, &ag).await.agreement.status, "active");
    assert_books(&e).await;
    // history is immutable
    for t in ["installment_collections", "collection_allocations"] {
        assert!(sqlx::query(&format!("DELETE FROM {t}"))
            .execute(&e.pool)
            .await
            .is_err());
        assert!(sqlx::query(&format!("UPDATE {t} SET amount = 1"))
            .execute(&e.pool)
            .await
            .is_err());
    }
    assert!(
        sqlx::query("UPDATE installment_schedule SET amount = amount + 1")
            .execute(&e.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE installment_schedule SET due_date = '2030-01-01'")
            .execute(&e.pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("DELETE FROM installment_schedule")
        .execute(&e.pool)
        .await
        .is_err());
}

#[tokio::test]
async fn early_settlement_closes_only_at_exact_zero_with_authorised_rebate() {
    let e = env().await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("installment.default_markup_bp".into(), "1000".into())]),
    )
    .await
    .unwrap();
    let mgr = user(&e, "mgr", "Manager").await;
    let se = user(&e, "sexec", "Sales Executive").await;
    let c = new_customer(&e, "Karim").await;
    let ag = agreement_of(
        &e,
        &isale(
            &e,
            c,
            "F-1",
            1_000_000,
            None,
            terms(5, "monthly", &plus_days(30)),
            "a",
        )
        .await
        .unwrap(),
    )
    .await; // 50,000 + 5,000 markup
    let aid = Id::parse(&ag).unwrap();
    installments::collect(&e.pool, &e.owner, collect(&ag, 1_100_000, "c1"))
        .await
        .unwrap(); // 1 of 5 paid -> 20% of markup earned (100,000)
    let q = installments::settle_quote(&e.pool, &e.owner, aid)
        .await
        .unwrap();
    assert_eq!(
        (q.outstanding, q.max_rebate),
        (4_400_000, 400_000),
        "only unearned markup can be waived (500,000 - 100,000 earned)"
    );
    let settle = |rebate: i64, reason: &str, key: &str, who: Principal| {
        let (pool, ag) = (e.pool.clone(), ag.clone());
        let (reason, key) = (reason.to_string(), key.to_string());
        async move {
            installments::settle_early(
                &pool,
                &who,
                SettleIn {
                    agreement_id: ag,
                    rebate,
                    reason,
                    method: "cash".into(),
                    reference: "".into(),
                    client_key: key,
                },
            )
            .await
        }
    };
    // a rebate needs permission and a reason, and is capped
    assert!(forbidden(
        settle(100_000, "goodwill", "s1", se.clone()).await
    ));
    assert!(settle(100_000, " ", "s2", mgr.clone()).await.is_err());
    assert!(settle(400_001, "too much", "s3", mgr.clone())
        .await
        .is_err());
    assert!(settle(-1, "x", "s4", mgr.clone()).await.is_err());
    assert_eq!(
        detail(&e, &ag).await.agreement.outstanding,
        4_400_000,
        "refused attempts change nothing"
    );
    let done = settle(300_000, "early settlement discount", "s5", mgr.clone())
        .await
        .unwrap();
    assert!(done.completed && done.outstanding == 0);
    let d = detail(&e, &ag).await;
    assert_eq!(
        (
            d.agreement.status.as_str(),
            d.agreement.outstanding,
            d.rebates
        ),
        ("completed", 0, 300_000)
    );
    assert!(d
        .items
        .iter()
        .all(|i| i.remaining == 0 && i.status == "paid"));
    // total markup income = markup - rebate; nothing left unearned
    assert_eq!(
        (
            bal(&e, code::MARKUP_INCOME).await,
            bal(&e, code::UNEARNED_MARKUP).await
        ),
        (500_000 - 300_000, 0)
    );
    assert_eq!(bal(&e, code::INSTALLMENT_AR).await, 0);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        0
    );
    assert_eq!(
        bal(&e, code::CASH).await,
        1_000_000 + 1_100_000 + (4_400_000 - 300_000)
    );
    // double submit does not pay twice; settled agreements refuse more
    assert!(
        settle(300_000, "again", "s5", mgr.clone())
            .await
            .unwrap()
            .completed
    );
    assert!(matches!(
        settle(0, "", "s6", mgr.clone()).await,
        Err(CoreError::Conflict(_))
    ));
    assert_eq!(audit_count(&e.pool, "installment.settled_early").await, 1);
    assert_eq!(audit_count(&e.pool, "installment.rebate").await, 1);
    assert_books(&e).await;
    // settlement without any rebate needs no special permission
    let c2 = new_customer(&e, "Other").await;
    let ag2 = agreement_of(
        &e,
        &isale(
            &e,
            c2,
            "F-2",
            1_000_000,
            None,
            terms(5, "monthly", &plus_days(30)),
            "b",
        )
        .await
        .unwrap(),
    )
    .await;
    let r = installments::settle_early(
        &e.pool,
        &se,
        SettleIn {
            agreement_id: ag2.clone(),
            rebate: 0,
            reason: "".into(),
            method: "bank".into(),
            reference: "TT".into(),
            client_key: "z".into(),
        },
    )
    .await
    .unwrap();
    assert!(r.completed);
    assert_eq!(bal(&e, code::MARKUP_INCOME).await, 200_000 + 500_000);
    assert_books(&e).await;
}

#[tokio::test]
async fn rescheduling_replaces_only_the_unpaid_part_and_is_authorised_and_audited() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    let se = user(&e, "sexec", "Sales Executive").await;
    let c = new_customer(&e, "Karim").await;
    let ag = agreement_of(
        &e,
        &isale(
            &e,
            c,
            "F-1",
            1_000_000,
            Some("2026-01-01"),
            terms(5, "monthly", "2026-01-15"),
            "a",
        )
        .await
        .unwrap(),
    )
    .await;
    let aid = Id::parse(&ag).unwrap();
    installments::collect(&e.pool, &e.owner, collect(&ag, 1_500_000, "c1"))
        .await
        .unwrap(); // installment 1 paid, 2 partial (500,000)
    assert!(detail(&e, &ag).await.agreement.overdue_amount > 0);
    let rs = |count: i64, freq: &str, first: String, reason: &str, who: Principal| {
        let ag = ag.clone();
        let (freq, reason) = (freq.to_string(), reason.to_string());
        let pool = e.pool.clone();
        async move {
            installments::reschedule(
                &pool,
                &who,
                RescheduleIn {
                    agreement_id: ag,
                    count,
                    frequency: freq,
                    first_due_date: first,
                    custom: vec![],
                    reason,
                },
            )
            .await
        }
    };
    assert!(forbidden(
        rs(4, "monthly", plus_days(10), "hardship", se.clone()).await
    ));
    assert!(rs(4, "monthly", plus_days(10), "  ", mgr.clone())
        .await
        .is_err());
    assert!(
        rs(4, "monthly", "2026-01-01".into(), "past", mgr.clone())
            .await
            .is_err(),
        "new schedule cannot start in the past"
    );
    assert!(rs(0, "monthly", plus_days(10), "x", mgr.clone())
        .await
        .is_err());
    assert_eq!(
        detail(&e, &ag).await.items.len(),
        5,
        "refused attempts change nothing"
    );
    rs(
        4,
        "monthly",
        plus_days(10),
        "customer hardship - agreed with owner",
        mgr.clone(),
    )
    .await
    .unwrap();
    let d = detail(&e, &ag).await;
    assert_eq!(d.agreement.outstanding, 3_500_000);
    assert_eq!(d.reschedules.len(), 1);
    assert_eq!(
        (d.reschedules[0].outstanding, d.reschedules[0].new_count),
        (3_500_000, 4)
    );
    let active: Vec<_> = d.items.iter().filter(|i| !i.superseded).collect();
    // paid installment 1 stays; installment 2 (partially paid) and 3-5 are superseded; 4 new rows cover the outstanding exactly
    assert_eq!(d.items.iter().filter(|i| i.superseded).count(), 4);
    assert_eq!(d.items[0].status, "paid");
    assert_eq!(d.items[1].status, "superseded");
    assert_eq!(
        d.items[1].effective, 500_000,
        "a superseded row keeps only what was actually paid on it"
    );
    assert_eq!(
        d.items[5..].iter().map(|i| i.amount).sum::<i64>(),
        3_500_000
    );
    assert_eq!(active.len(), 5);
    assert_eq!(
        d.agreement.overdue_amount, 0,
        "the new schedule is not yet due"
    );
    assert!(installments::schedule_mismatches(&e.pool)
        .await
        .unwrap()
        .is_empty());
    // a collection recorded before the reschedule can no longer be reversed (history is locked in)
    let c1 = d.collections[0].id.clone();
    assert!(matches!(
        installments::reverse_collection(&e.pool, &e.owner, Id::parse(&c1).unwrap(), "x").await,
        Err(CoreError::Conflict(_))
    ));
    // payments continue against the new schedule and complete it exactly
    let r = installments::collect(&e.pool, &e.owner, collect(&ag, 875_000, "c2"))
        .await
        .unwrap();
    assert_eq!(r.allocations, vec![(6, 875_000)]);
    installments::collect(&e.pool, &e.owner, collect(&ag, 2_625_000, "c3"))
        .await
        .unwrap();
    assert_eq!(detail(&e, &ag).await.agreement.status, "completed");
    assert!(
        installments::reschedule(
            &e.pool,
            &mgr,
            RescheduleIn {
                agreement_id: ag.clone(),
                count: 3,
                frequency: "weekly".into(),
                first_due_date: plus_days(7),
                custom: vec![],
                reason: "x".into()
            }
        )
        .await
        .is_err(),
        "nothing left to reschedule"
    );
    assert_eq!(audit_count(&e.pool, "installment.rescheduled").await, 1);
    assert!(installments::schedule_mismatches(&e.pool)
        .await
        .unwrap()
        .is_empty());
    assert_books(&e).await;
    let _ = aid;
}

#[tokio::test]
async fn custom_schedules_and_weekly_schedules_work_end_to_end() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let mut t = terms(0, "custom", &plus_days(5));
    t.custom = vec![
        CustomDue {
            due_date: plus_days(5),
            amount: 2_000_000,
        },
        CustomDue {
            due_date: plus_days(60),
            amount: 1_500_000,
        },
        CustomDue {
            due_date: plus_days(200),
            amount: 1_500_000,
        },
    ];
    let ag = agreement_of(
        &e,
        &isale(&e, c, "F-1", 1_000_000, None, t, "a").await.unwrap(),
    )
    .await;
    let d = detail(&e, &ag).await;
    assert_eq!(
        (
            d.frequency.as_str(),
            d.items.len(),
            d.agreement.installment_count
        ),
        ("custom", 3, 3)
    );
    assert_eq!(
        d.items.iter().map(|i| i.amount).collect::<Vec<_>>(),
        vec![2_000_000, 1_500_000, 1_500_000]
    );
    let mut bad = terms(0, "custom", &plus_days(5));
    bad.custom = vec![CustomDue {
        due_date: plus_days(5),
        amount: 1,
    }];
    assert!(isale(&e, c, "F-2", 1_000_000, None, bad, "b")
        .await
        .is_err());
    let wk = agreement_of(
        &e,
        &isale(
            &e,
            c,
            "F-3",
            1_000_000,
            None,
            terms(10, "weekly", &plus_days(7)),
            "c",
        )
        .await
        .unwrap(),
    )
    .await;
    let w = detail(&e, &wk).await;
    assert_eq!(w.items.iter().map(|i| i.amount).sum::<i64>(), 5_000_000);
    let d0 = NaiveDate::parse_from_str(&w.items[0].due_date, "%Y-%m-%d").unwrap();
    let d1 = NaiveDate::parse_from_str(&w.items[1].due_date, "%Y-%m-%d").unwrap();
    assert_eq!((d1 - d0).num_days(), 7);
    assert_books(&e).await;
}

#[tokio::test]
async fn installment_services_enforce_permissions() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let ag = agreement_of(
        &e,
        &isale(
            &e,
            c,
            "F-1",
            1_000_000,
            None,
            terms(4, "monthly", &plus_days(30)),
            "a",
        )
        .await
        .unwrap(),
    )
    .await;
    let aid = Id::parse(&ag).unwrap();
    let acct = user(&e, "acct", "Accountant").await; // sees installments, cannot collect/create
    let inv = user(&e, "inv", "Inventory Manager").await; // no installment permissions at all
    assert!(installments::get_agreement(&e.pool, &acct, aid)
        .await
        .is_ok());
    assert!(forbidden(
        installments::get_agreement(&e.pool, &inv, aid).await
    ));
    assert!(forbidden(
        installments::list_agreements(&e.pool, &inv, AgreementFilter::default()).await
    ));
    assert!(forbidden(
        installments::overdue_worklist(&e.pool, &inv).await
    ));
    assert!(forbidden(
        installments::settle_quote(&e.pool, &inv, aid).await
    ));
    assert!(forbidden(
        installments::collect(&e.pool, &acct, collect(&ag, 1, "k")).await
    ));
    assert!(forbidden(
        installments::settle_early(
            &e.pool,
            &acct,
            SettleIn {
                agreement_id: ag.clone(),
                rebate: 0,
                reason: "".into(),
                method: "cash".into(),
                reference: "".into(),
                client_key: "k".into()
            }
        )
        .await
    ));
    assert!(forbidden(
        installments::reschedule(
            &e.pool,
            &acct,
            RescheduleIn {
                agreement_id: ag.clone(),
                count: 2,
                frequency: "monthly".into(),
                first_due_date: plus_days(9),
                custom: vec![],
                reason: "x".into()
            }
        )
        .await
    ));
    assert!(forbidden(
        installments::add_followup(&e.pool, &acct, aid, "x", None).await
    ));
    assert!(forbidden(
        installments::reverse_collection(&e.pool, &acct, Id::new(), "x").await
    ));
    assert!(forbidden(
        sales::preview_sale(
            &e.pool,
            &acct,
            &SaleIn {
                mode: "installment".into(),
                customer_id: None,
                sale_date: None,
                items: vec![],
                payments: vec![],
                notes: "".into(),
                client_key: "k".into(),
                credit_override: false,
                installment: None
            }
        )
        .await
    ));
    assert_eq!(detail(&e, &ag).await.agreement.outstanding, 5_000_000);
}

#[tokio::test]
async fn books_detect_tampering_with_installment_balances() {
    let e = env().await;
    let c = new_customer(&e, "Karim").await;
    let ag = agreement_of(
        &e,
        &isale(
            &e,
            c,
            "F-1",
            1_000_000,
            None,
            terms(4, "monthly", &plus_days(30)),
            "a",
        )
        .await
        .unwrap(),
    )
    .await;
    installments::collect(&e.pool, &e.owner, collect(&ag, 500_000, "c"))
        .await
        .unwrap();
    assert_books(&e).await;
    // someone inserts an allocation-less collection straight into the table (bypassing the service)
    sqlx::query("INSERT INTO installment_collections (id, agreement_id, receipt_no, kind, amount, method, paid_on, created_at) VALUES ('rogue', ?, 'RCT-999999', 'payment', 100000, 'cash', '2026-01-01', '2026-01-01T00:00:00.000Z')").bind(&ag).execute(&e.pool).await.unwrap();
    let m = accounting::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(
        m.iter()
            .any(|x| x.check == "installment_receivable_vs_agreements"),
        "{m:?}"
    );
    assert!(m.iter().any(|x| x.check == "unposted_documents"
        || x.check == "installment_schedule_total"
        || x.check == "unearned_markup"
        || x.check == "installment_receivable_vs_agreements"));
    // the back-fill posts the orphan so the books agree with the sub-ledger again, and the remaining gap is the missing customer ledger row
    accounting::ensure_posted(&e.pool).await.unwrap();
    let m = accounting::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(
        m.iter()
            .any(|x| x.check == "customer_ledger_vs_receivables"),
        "{m:?}"
    );
}
