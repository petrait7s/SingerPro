//! P3 test-suite: double-entry accounting, customers, retail/wholesale sales, receipts, reconciliation.

use crate::accounting::{self, code, cr, dr, ManualLine, PostIn};
use crate::auth;
use crate::authz::{self, Principal};
use crate::catalogue::{self, NewProduct, NewVariant};
use crate::customers::{self, CustomerIn};
use crate::inventory::{self, AdjustIn, OpeningStockIn};
use crate::purchases::{self, LineIn, PaymentIn, PurchaseIn, ReceiveIn, ReceiveLine};
use crate::recovery::DataPaths;
use crate::sales::{self, CollectIn, PayLineIn, SaleFilter, SaleIn, SaleLineIn};
use crate::suppliers::{self, SupplierIn};
use crate::users::{self, NewUser};
use crate::{db, settings, CoreError, Id};
use sqlx::SqlitePool;
use std::collections::HashMap;

const PW: &str = "Correct-horse-battery-9";

struct Env {
    pool: SqlitePool,
    owner: Principal,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let pool = db::open(&DataPaths::new(dir.path()).db).await.unwrap();
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
    let rows = accounting::list_accounts(&e.pool, &e.owner, None)
        .await
        .unwrap();
    rows.into_iter()
        .find(|a| a.code == account)
        .unwrap()
        .balance
}

async fn assert_books_balance(e: &Env) {
    let m = accounting::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(m.is_empty(), "books must reconcile: {m:?}");
    let s = crate::stock::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(s.is_empty(), "stock must reconcile: {s:?}");
    let tb = accounting::trial_balance(&e.pool, &e.owner, None)
        .await
        .unwrap();
    assert!(
        tb.balanced,
        "trial balance: {} vs {}",
        tb.total_debit, tb.total_credit
    );
}

fn vin(sku: &str, cost: i64, retail: i64, wholesale: i64) -> NewVariant {
    NewVariant {
        sku: sku.into(),
        barcode: None,
        colour: "".into(),
        capacity: "".into(),
        size: "".into(),
        purchase_cost: cost,
        retail_price: retail,
        wholesale_price: wholesale,
        installment_price: None,
    }
}

struct Shop {
    fan: String,    // non-serialized variant
    fridge: String, // serialized variant
}

/// Catalogue with a fan (qty) and a fridge (serial) plus opening stock.
async fn shop(e: &Env) -> Shop {
    let brand = catalogue::create_brand(&e.pool, &e.owner, "Singer")
        .await
        .unwrap();
    let cat = catalogue::create_category(&e.pool, &e.owner, "Appliances", None)
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
    let fan = catalogue::create_product(
        &e.pool,
        &e.owner,
        mk("Fan", false, vin("FAN", 100_000, 150_000, 130_000)),
    )
    .await
    .unwrap();
    let fridge = catalogue::create_product(
        &e.pool,
        &e.owner,
        mk(
            "Fridge",
            true,
            vin("FRIDGE", 4_000_000, 5_200_000, 4_800_000),
        ),
    )
    .await
    .unwrap();
    let vid = |p: Id| {
        let pool = e.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT id FROM product_variants WHERE product_id = ?")
                .bind(p.to_string())
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let (fan, fridge) = (vid(fan).await, vid(fridge).await);
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: fan.clone(),
            qty: 10,
            unit_cost: 100_000,
            serials: vec![],
            reason: "open".into(),
        },
    )
    .await
    .unwrap();
    inventory::record_opening_stock(
        &e.pool,
        &e.owner,
        OpeningStockIn {
            variant_id: fridge.clone(),
            qty: 0,
            unit_cost: 4_000_000,
            serials: vec!["F-1".into(), "F-2".into(), "F-3".into()],
            reason: "open".into(),
        },
    )
    .await
    .unwrap();
    Shop { fan, fridge }
}

async fn serial_id(e: &Env, sn: &str) -> String {
    sqlx::query_scalar("SELECT id FROM product_serials WHERE serial_no = ?")
        .bind(sn)
        .fetch_one(&e.pool)
        .await
        .unwrap()
}

fn cust_in(name: &str, kind: &str, limit: i64) -> CustomerIn {
    CustomerIn {
        name: name.into(),
        phone: "01711111111".into(),
        alt_phone: "".into(),
        address: "Dhaka".into(),
        customer_type: kind.into(),
        credit_limit: limit,
        nid: "".into(),
        notes: "".into(),
    }
}

fn line(v: &str, qty: i64) -> SaleLineIn {
    SaleLineIn {
        variant_id: v.into(),
        qty,
        discount: 0,
        serial_ids: vec![],
    }
}

fn pay(m: &str, a: i64) -> PayLineIn {
    PayLineIn {
        method: m.into(),
        amount: a,
        reference: "".into(),
    }
}

fn sale(mode: &str, items: Vec<SaleLineIn>, payments: Vec<PayLineIn>, key: &str) -> SaleIn {
    SaleIn {
        mode: mode.into(),
        customer_id: None,
        sale_date: None,
        items,
        payments,
        notes: "".into(),
        client_key: key.into(),
        credit_override: false,
        installment: None,
    }
}

// ------------------------------------------------------------------ accounting core

#[tokio::test]
async fn chart_is_seeded_and_journals_are_balanced_idempotent_and_immutable() {
    let e = env().await;
    let accts = accounting::list_accounts(&e.pool, &e.owner, None)
        .await
        .unwrap();
    assert!(
        accts.len() >= 30
            && accts
                .iter()
                .any(|a| a.code == "1200" && a.normal_side == "debit")
    );
    assert!(accts
        .iter()
        .any(|a| a.code == "2000" && a.normal_side == "credit"));
    let mut tx = e.pool.begin().await.unwrap();
    // unbalanced entries are refused
    let bad = accounting::post(
        &mut tx,
        &PostIn {
            key: "t1",
            date: "2026-01-01",
            memo: "x",
            source_type: "test",
            source_id: "1",
            lines: vec![dr("1000", 100, ""), cr("4000", 99, "")],
            reverses: None,
            actor: None,
        },
    )
    .await;
    assert!(matches!(bad, Err(CoreError::Integrity(_))));
    // a line cannot be both debit and credit; unknown accounts and bad dates refused
    let both = accounting::Line {
        account: "1000",
        debit: 5,
        credit: 5,
        memo: "",
    };
    assert!(accounting::post(
        &mut tx,
        &PostIn {
            key: "t2",
            date: "2026-01-01",
            memo: "x",
            source_type: "test",
            source_id: "2",
            lines: vec![both, cr("4000", 5, "")],
            reverses: None,
            actor: None
        }
    )
    .await
    .is_err());
    assert!(accounting::post(
        &mut tx,
        &PostIn {
            key: "t3",
            date: "2026-01-01",
            memo: "x",
            source_type: "test",
            source_id: "3",
            lines: vec![dr("9999", 5, ""), cr("4000", 5, "")],
            reverses: None,
            actor: None
        }
    )
    .await
    .is_err());
    assert!(accounting::post(
        &mut tx,
        &PostIn {
            key: "t4",
            date: "01/01/2026",
            memo: "x",
            source_type: "test",
            source_id: "4",
            lines: vec![dr("1000", 5, ""), cr("4000", 5, "")],
            reverses: None,
            actor: None
        }
    )
    .await
    .is_err());
    // all-zero is skipped; good entry posts; same key is a no-op
    assert!(accounting::post(
        &mut tx,
        &PostIn {
            key: "t5",
            date: "2026-01-01",
            memo: "x",
            source_type: "test",
            source_id: "5",
            lines: vec![dr("1000", 0, ""), cr("4000", 0, "")],
            reverses: None,
            actor: None
        }
    )
    .await
    .unwrap()
    .is_none());
    let first = accounting::post(
        &mut tx,
        &PostIn {
            key: "t6",
            date: "2026-01-01",
            memo: "x",
            source_type: "test",
            source_id: "6",
            lines: vec![dr("1000", 100, ""), cr("4000", 100, "")],
            reverses: None,
            actor: None,
        },
    )
    .await
    .unwrap();
    let again = accounting::post(
        &mut tx,
        &PostIn {
            key: "t6",
            date: "2026-01-01",
            memo: "x",
            source_type: "test",
            source_id: "6",
            lines: vec![dr("1000", 100, ""), cr("4000", 100, "")],
            reverses: None,
            actor: None,
        },
    )
    .await
    .unwrap();
    assert!(first.is_some() && again.is_none());
    tx.commit().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "failed postings leave nothing");
    for t in ["journal_entries", "journal_lines"] {
        assert!(sqlx::query(&format!("UPDATE {t} SET memo = 'x'"))
            .execute(&e.pool)
            .await
            .is_err());
        assert!(sqlx::query(&format!("DELETE FROM {t}"))
            .execute(&e.pool)
            .await
            .is_err());
    }
    // DB-level: a line may not carry both sides or be negative
    assert!(sqlx::query("INSERT INTO journal_lines (id, entry_id, line_no, account_id, debit, credit) SELECT 'x', id, 99, (SELECT id FROM accounts LIMIT 1), 1, 1 FROM journal_entries LIMIT 1").execute(&e.pool).await.is_err());
}

#[tokio::test]
async fn purchase_lifecycle_posts_the_documented_journals_and_books_reconcile() {
    let e = env().await;
    let s = shop(&e).await;
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
    // opening stock was valued 10x1000 + 3x40000 = 130,000.00 -> Dr Inventory / Cr Opening Equity
    assert_eq!(bal(&e, code::INVENTORY).await, 10 * 100_000 + 3 * 4_000_000);
    assert_eq!(
        bal(&e, code::OPENING_EQUITY).await,
        10 * 100_000 + 3 * 4_000_000
    );
    let po = purchases::create_draft(
        &e.pool,
        &e.owner,
        PurchaseIn {
            supplier_id: sup.to_string(),
            supplier_invoice_no: Some("I1".into()),
            order_date: "2026-03-01".into(),
            discount: 10_000,
            notes: "".into(),
            items: vec![LineIn {
                variant_id: s.fan.clone(),
                qty: 5,
                unit_cost: 100_000,
            }],
            costs: vec![crate::purchases::CostIn {
                label: "Freight".into(),
                amount: 25_000,
            }],
        },
    )
    .await
    .unwrap();
    assert_eq!(bal(&e, code::AP).await, 0, "drafts post nothing");
    purchases::confirm(&e.pool, &e.owner, po).await.unwrap();
    assert_eq!(
        bal(&e, code::AP).await,
        490_000,
        "payable = 5 x 1000 - 100 discount"
    );
    assert_eq!(bal(&e, code::ACCRUED_LANDED).await, 25_000);
    assert_eq!(bal(&e, code::IN_TRANSIT).await, 515_000);
    let item = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    purchases::receive(
        &e.pool,
        &e.owner,
        po,
        ReceiveIn {
            received_on: "2026-03-05".into(),
            note: "".into(),
            lines: vec![ReceiveLine {
                item_id: item.clone(),
                qty: 2,
                serials: vec![],
            }],
        },
    )
    .await
    .unwrap();
    assert_eq!(
        bal(&e, code::IN_TRANSIT).await,
        515_000 - 206_000,
        "2 of 5 units moved from transit to inventory at allocated cost"
    );
    purchases::receive(
        &e.pool,
        &e.owner,
        po,
        ReceiveIn {
            received_on: "2026-03-06".into(),
            note: "".into(),
            lines: vec![ReceiveLine {
                item_id: item,
                qty: 3,
                serials: vec![],
            }],
        },
    )
    .await
    .unwrap();
    assert_eq!(
        bal(&e, code::IN_TRANSIT).await,
        0,
        "fully received -> nothing left in transit"
    );
    assert_eq!(
        bal(&e, code::INVENTORY).await,
        10 * 100_000 + 3 * 4_000_000 + 515_000
    );
    assert_books_balance(&e).await;
    // payment and reversal
    let p1 = purchases::pay_supplier(
        &e.pool,
        &e.owner,
        PaymentIn {
            supplier_id: sup.to_string(),
            purchase_id: Some(po.to_string()),
            amount: 200_000,
            method: "bank".into(),
            paid_on: "2026-03-10".into(),
            reference: "".into(),
            note: "".into(),
            client_key: "k".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        (bal(&e, code::AP).await, bal(&e, code::BANK).await),
        (290_000, -200_000)
    );
    purchases::reverse_payment(&e.pool, &e.owner, p1, "bounced")
        .await
        .unwrap();
    assert_eq!(
        (bal(&e, code::AP).await, bal(&e, code::BANK).await),
        (490_000, 0)
    );
    // return 1 unit: stock out at cost, credit at invoice price; variance to 5110
    let det = purchases::get_purchase(&e.pool, &e.owner, po)
        .await
        .unwrap();
    purchases::return_to_supplier(
        &e.pool,
        &e.owner,
        po,
        crate::purchases::ReturnIn {
            returned_on: "2026-03-12".into(),
            reason: "faulty".into(),
            lines: vec![crate::purchases::ReturnLine {
                item_id: det.items[0].id.clone(),
                qty: 1,
                serials: vec![],
            }],
        },
    )
    .await
    .unwrap();
    assert_eq!(bal(&e, code::AP).await, 490_000 - 100_000);
    assert_books_balance(&e).await;
    // adjustments: loss goes to 5100, gain comes from 5100
    inventory::adjust_quantity(
        &e.pool,
        &e.owner,
        AdjustIn {
            variant_id: s.fan.clone(),
            bucket: "available".into(),
            qty_delta: -2,
            unit_cost: None,
            reason: "stolen".into(),
        },
    )
    .await
    .unwrap();
    assert!(bal(&e, code::INVENTORY_ADJUSTMENTS).await > 0);
    inventory::change_serial_status(
        &e.pool,
        &e.owner,
        Id::parse(&serial_id(&e, "F-1").await).unwrap(),
        "written_off",
        "irreparable",
    )
    .await
    .unwrap();
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn backfill_posts_unposted_documents_and_reconcile_notices_them() {
    let e = env().await;
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
    // a payment row + ledger entry that never got a journal (e.g. imported from an older version)
    sqlx::query("INSERT INTO supplier_payments (id, supplier_id, amount, method, paid_on, created_at, client_key) VALUES ('legacy-pay', ?, 5000, 'cash', '2026-01-05', '2026-01-05T04:00:00.000Z', 'legacy')").bind(sup.to_string()).execute(&e.pool).await.unwrap();
    sqlx::query("INSERT INTO supplier_ledger_entries (id, supplier_id, at, kind, amount, doc_type, doc_id) VALUES ('l1', ?, '2026-01-05T04:00:00.000Z', 'payment', -5000, 'supplier_payment', 'legacy-pay')").bind(sup.to_string()).execute(&e.pool).await.unwrap();
    let before = accounting::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(
        before.iter().any(|m| m.check == "unposted_documents"),
        "{before:?}"
    );
    assert_eq!(accounting::ensure_posted(&e.pool).await.unwrap(), 1);
    assert_eq!(
        accounting::ensure_posted(&e.pool).await.unwrap(),
        0,
        "idempotent"
    );
    assert_books_balance(&e).await;
    assert_eq!(bal(&e, code::CASH).await, -5000);
}

#[tokio::test]
async fn manual_journals_openings_and_reversals_follow_the_rules() {
    let e = env().await;
    let acct = user(&e, "acct", "Accountant").await;
    let cashier = user(&e, "cash", "Cashier").await;
    let ml = |c: &str, d: i64, cr_: i64| ManualLine {
        account_code: c.into(),
        debit: d,
        credit: cr_,
        memo: "".into(),
    };
    assert!(forbidden(
        accounting::manual_journal(
            &e.pool,
            &cashier,
            "2026-02-01",
            "x",
            vec![ml("1000", 5, 0), ml("4900", 0, 5)]
        )
        .await
    ));
    assert!(forbidden(
        accounting::opening_balance(&e.pool, &cashier, "1000", 5, "2026-01-01").await
    ));
    assert!(forbidden(
        accounting::list_accounts(&e.pool, &cashier, None).await
    ));
    // validation
    for bad in [
        vec![ml("1000", 5, 0)],
        vec![ml("1000", 5, 0), ml("4900", 0, 4)],
        vec![ml("1000", 5, 5), ml("4900", 0, 5)],
        vec![ml("1000", 5, 0), ml("0000", 0, 5)],
        vec![ml("1000", 0, 0), ml("4900", 0, 0)],
    ] {
        assert!(
            accounting::manual_journal(&e.pool, &acct, "2026-02-01", "x", bad)
                .await
                .is_err()
        );
    }
    assert!(accounting::manual_journal(
        &e.pool,
        &acct,
        "2026-02-01",
        "  ",
        vec![ml("1000", 5, 0), ml("4900", 0, 5)]
    )
    .await
    .is_err());
    let j = accounting::manual_journal(
        &e.pool,
        &acct,
        "2026-02-01",
        "Interest received",
        vec![ml("1010", 5000, 0), ml("4900", 0, 5000)],
    )
    .await
    .unwrap();
    assert_eq!(
        (bal(&e, code::BANK).await, bal(&e, code::OTHER_INCOME).await),
        (5000, 5000)
    );
    // reversal: reason required, once only, reversal not reversible
    assert!(accounting::reverse_entry(&e.pool, &acct, j, " ")
        .await
        .is_err());
    let r = accounting::reverse_entry(&e.pool, &acct, j, "wrong account")
        .await
        .unwrap();
    assert_eq!(bal(&e, code::BANK).await, 0);
    assert!(accounting::reverse_entry(&e.pool, &acct, j, "again")
        .await
        .is_err());
    assert!(accounting::reverse_entry(&e.pool, &acct, r, "undo")
        .await
        .is_err());
    let d = accounting::get_entry(&e.pool, &acct, j).await.unwrap();
    assert!(d.entry.reversed && d.lines.len() == 2);
    // openings
    assert!(
        accounting::opening_balance(&e.pool, &acct, "1200", 5, "2026-01-01")
            .await
            .is_err(),
        "only cash/bank/wallet"
    );
    assert!(
        accounting::opening_balance(&e.pool, &acct, "1000", 0, "2026-01-01")
            .await
            .is_err()
    );
    accounting::opening_balance(&e.pool, &acct, "1000", 250_000, "2026-01-01")
        .await
        .unwrap();
    assert!(
        matches!(
            accounting::opening_balance(&e.pool, &acct, "1000", 1, "2026-01-01").await,
            Err(CoreError::Conflict(_))
        ),
        "only once"
    );
    assert_eq!(bal(&e, code::CASH).await, 250_000);
    // document-generated entries cannot be reversed by hand
    let s = shop(&e).await;
    let out = sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 1)],
            vec![pay("cash", 150_000)],
            "k1",
        ),
    )
    .await
    .unwrap();
    let ents = accounting::list_entries(
        &e.pool,
        &acct,
        accounting::EntryFilter {
            source_type: Some("sale".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        accounting::reverse_entry(&e.pool, &acct, Id::parse(&ents.rows[0].id).unwrap(), "x").await,
        Err(CoreError::Conflict(_))
    ));
    let _ = out;
    assert_books_balance(&e).await;
    assert_eq!(audit_count(&e.pool, "journal.manual").await, 1);
    assert_eq!(audit_count(&e.pool, "journal.reversed").await, 1);
    assert_eq!(audit_count(&e.pool, "journal.opening_balance").await, 1);
}

#[tokio::test]
async fn ledger_running_balances_and_trial_balance() {
    let e = env().await;
    accounting::opening_balance(&e.pool, &e.owner, "1000", 100_000, "2026-01-01")
        .await
        .unwrap();
    let ml = |c: &str, d: i64, c2: i64| ManualLine {
        account_code: c.into(),
        debit: d,
        credit: c2,
        memo: "m".into(),
    };
    accounting::manual_journal(
        &e.pool,
        &e.owner,
        "2026-02-10",
        "a",
        vec![ml("1000", 50_000, 0), ml("4900", 0, 50_000)],
    )
    .await
    .unwrap();
    accounting::manual_journal(
        &e.pool,
        &e.owner,
        "2026-03-10",
        "b",
        vec![ml("6900", 30_000, 0), ml("1000", 0, 30_000)],
    )
    .await
    .unwrap();
    let l = accounting::ledger(&e.pool, &e.owner, "1000", None, None)
        .await
        .unwrap();
    assert_eq!(
        l.lines.iter().map(|x| x.balance).collect::<Vec<_>>(),
        vec![100_000, 150_000, 120_000]
    );
    assert_eq!(l.closing, 120_000);
    let l = accounting::ledger(
        &e.pool,
        &e.owner,
        "1000",
        Some("2026-03-01"),
        Some("2026-03-31"),
    )
    .await
    .unwrap();
    assert_eq!(
        (l.opening, l.lines.len(), l.closing),
        (150_000, 1, 120_000),
        "period ledger starts from the prior balance"
    );
    let tb = accounting::trial_balance(&e.pool, &e.owner, Some("2026-02-28"))
        .await
        .unwrap();
    assert!(tb.balanced);
    assert_eq!(
        tb.rows.iter().find(|r| r.code == "1000").unwrap().balance,
        150_000
    );
    assert!(accounting::ledger(&e.pool, &e.owner, "0000", None, None)
        .await
        .is_err());
}

// ------------------------------------------------------------------ customers

#[tokio::test]
async fn customer_profiles_codes_nid_and_credit_limit_controls() {
    let e = env().await;
    let mgr = user(&e, "mgr", "Manager").await;
    let sales_exec = user(&e, "sexec", "Sales Executive").await;
    let c1 = customers::create_customer(&e.pool, &sales_exec, cust_in("Karim", "retail", 0))
        .await
        .unwrap();
    let c2 = customers::create_customer(
        &e.pool,
        &sales_exec,
        cust_in("Dealer Co", "dealer", 5_000_000),
    )
    .await
    .unwrap();
    let a = customers::get_customer(&e.pool, &sales_exec, c1)
        .await
        .unwrap();
    let b = customers::get_customer(&e.pool, &sales_exec, c2)
        .await
        .unwrap();
    assert_eq!(
        (a.code.as_str(), b.code.as_str()),
        ("CUS-00001", "CUS-00002")
    );
    // validation
    assert!(
        customers::create_customer(&e.pool, &sales_exec, cust_in("", "retail", 0))
            .await
            .is_err()
    );
    assert!(
        customers::create_customer(&e.pool, &sales_exec, cust_in("X", "vip", 0))
            .await
            .is_err()
    );
    assert!(
        customers::create_customer(&e.pool, &sales_exec, cust_in("X", "retail", -1))
            .await
            .is_err()
    );
    // NID: disabled by default, accepted once the shop enables it, and hidden from non-managers
    let mut with_nid = cust_in("Rahim", "retail", 0);
    with_nid.nid = "1234567890".into();
    assert!(matches!(
        customers::create_customer(&e.pool, &sales_exec, with_nid.clone()).await,
        Err(CoreError::Validation(_))
    ));
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("customers.collect_nid".into(), "true".into())]),
    )
    .await
    .unwrap();
    let c3 = customers::create_customer(&e.pool, &sales_exec, with_nid)
        .await
        .unwrap();
    assert_eq!(
        customers::get_customer(&e.pool, &sales_exec, c3)
            .await
            .unwrap()
            .nid,
        "1234567890",
        "managers of customers see it"
    );
    let cashier = user(&e, "cash", "Cashier").await; // customers.view only
    assert_eq!(
        customers::get_customer(&e.pool, &cashier, c3)
            .await
            .unwrap()
            .nid,
        "",
        "view-only roles never see NID"
    );
    // credit limit is a control: needs sales.credit_override
    let mut edit = cust_in("Dealer Co", "dealer", 9_000_000);
    assert!(forbidden(
        customers::update_customer(&e.pool, &sales_exec, c2, edit.clone()).await
    ));
    customers::update_customer(&e.pool, &mgr, c2, edit.clone())
        .await
        .unwrap();
    assert_eq!(
        audit_count(&e.pool, "customer.credit_limit_changed").await,
        1
    );
    edit.phone = "01999999999".into();
    customers::update_customer(&e.pool, &sales_exec, c2, edit)
        .await
        .unwrap();
    // search & activation
    assert_eq!(
        customers::list_customers(&e.pool, &cashier, Some("dealer"), None, false, false)
            .await
            .unwrap()
            .len(),
        1
    );
    customers::set_customer_active(&e.pool, &sales_exec, c1, false)
        .await
        .unwrap();
    assert_eq!(
        customers::list_customers(&e.pool, &cashier, None, None, false, false)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(forbidden(
        customers::create_customer(&e.pool, &cashier, cust_in("Z", "retail", 0)).await
    ));
}

#[tokio::test]
async fn customer_opening_balance_hits_ledger_and_receivables_once() {
    let e = env().await;
    let c = customers::create_customer(&e.pool, &e.owner, cust_in("Old Debtor", "wholesale", 0))
        .await
        .unwrap();
    customers::opening_balance(&e.pool, &e.owner, c, 750_000, "2026-01-01")
        .await
        .unwrap();
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        750_000
    );
    assert_eq!(bal(&e, code::AR).await, 750_000);
    assert!(
        customers::opening_balance(&e.pool, &e.owner, c, 1, "2026-01-01")
            .await
            .is_err(),
        "once per customer"
    );
    assert!(
        customers::opening_balance(&e.pool, &e.owner, c, 0, "2026-01-01")
            .await
            .is_err()
    );
    let cashier = user(&e, "cash", "Cashier").await;
    assert!(forbidden(
        customers::opening_balance(&e.pool, &cashier, c, 5, "2026-01-01").await
    ));
    assert_books_balance(&e).await;
}

// ------------------------------------------------------------------ retail sales

#[tokio::test]
async fn retail_sale_moves_stock_money_and_journals_atomically() {
    let e = env().await;
    let s = shop(&e).await;
    let cashier = user(&e, "cash", "Cashier").await;
    let r = sales::create_sale(
        &e.pool,
        &cashier,
        sale(
            "retail",
            vec![line(&s.fan, 3)],
            vec![
                pay("cash", 100_000),
                pay("bank", 200_000),
                pay("mobile_wallet", 150_000),
            ],
            "form-1",
        ),
    )
    .await
    .unwrap();
    assert_eq!((r.grand_total, r.paid, r.due), (450_000, 450_000, 0));
    assert_eq!(r.number, "INV-000001");
    assert_eq!(r.receipts, vec!["RCT-000001", "RCT-000002", "RCT-000003"]);
    // stock: 10 - 3, cost out at average (3 x 1000)
    let lv = inventory::stock_levels(&e.pool, &e.owner, inventory::StockFilter::default())
        .await
        .unwrap();
    assert_eq!(
        lv.rows.iter().find(|x| x.sku == "FAN").unwrap().available,
        7
    );
    // journals: split tender to three accounts, revenue, COGS
    assert_eq!(
        (
            bal(&e, code::CASH).await,
            bal(&e, code::BANK).await,
            bal(&e, code::WALLET).await
        ),
        (100_000, 200_000, 150_000)
    );
    assert_eq!(bal(&e, code::SALES).await, 450_000);
    assert_eq!(bal(&e, code::COGS).await, 300_000);
    assert_eq!(
        bal(&e, code::AR).await,
        0,
        "walk-in retail leaves no receivable"
    );
    assert_books_balance(&e).await;
    // the audit trail names the cashier
    assert_eq!(audit_count(&e.pool, "sale.created").await, 1);
    let d = sales::get_sale(&e.pool, &cashier, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (d.items.len(), d.payments.len(), d.cashier.as_deref()),
        (1, 3, Some("cash"))
    );
    assert_eq!(d.items[0].line_total, 450_000);
}

#[tokio::test]
async fn serial_sales_track_the_exact_unit_and_cost() {
    let e = env().await;
    let s = shop(&e).await;
    let f1 = serial_id(&e, "F-1").await;
    let mut l = line(&s.fridge, 1);
    l.serial_ids = vec![f1.clone()];
    let r = sales::create_sale(
        &e.pool,
        &e.owner,
        sale("retail", vec![l.clone()], vec![pay("cash", 5_200_000)], "a"),
    )
    .await
    .unwrap();
    let st: String = sqlx::query_scalar("SELECT status FROM product_serials WHERE id = ?")
        .bind(&f1)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(st, "sold");
    assert_eq!(
        bal(&e, code::COGS).await,
        4_000_000,
        "COGS uses the unit's own cost"
    );
    let d = sales::get_sale(&e.pool, &e.owner, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!(d.items[0].serial_no.as_deref(), Some("F-1"));
    // the same unit cannot be sold twice
    assert!(matches!(
        sales::create_sale(
            &e.pool,
            &e.owner,
            sale("retail", vec![l], vec![pay("cash", 5_200_000)], "b")
        )
        .await,
        Err(CoreError::Conflict(_))
    ));
    // validation: wrong count, wrong product, duplicates, serials on non-serialized, unavailable unit
    let f2 = serial_id(&e, "F-2").await;
    let mut two = line(&s.fridge, 2);
    two.serial_ids = vec![f2.clone()];
    assert!(sales::create_sale(
        &e.pool,
        &e.owner,
        sale("retail", vec![two], vec![pay("cash", 1)], "c")
    )
    .await
    .is_err());
    let mut dup = line(&s.fridge, 2);
    dup.serial_ids = vec![f2.clone(), f2.clone()];
    assert!(sales::create_sale(
        &e.pool,
        &e.owner,
        sale("retail", vec![dup], vec![pay("cash", 1)], "d")
    )
    .await
    .is_err());
    let mut wrong = line(&s.fan, 1);
    wrong.serial_ids = vec![f2.clone()];
    assert!(sales::create_sale(
        &e.pool,
        &e.owner,
        sale("retail", vec![wrong], vec![pay("cash", 1)], "e")
    )
    .await
    .is_err());
    inventory::change_serial_status(
        &e.pool,
        &e.owner,
        Id::parse(&f2).unwrap(),
        "damaged",
        "dropped",
    )
    .await
    .unwrap();
    let mut dmg = line(&s.fridge, 1);
    dmg.serial_ids = vec![f2];
    assert!(
        sales::create_sale(
            &e.pool,
            &e.owner,
            sale("retail", vec![dmg], vec![pay("cash", 5_200_000)], "f")
        )
        .await
        .is_err(),
        "damaged units are not sellable"
    );
    let picks = sales::available_serials(&e.pool, &e.owner, Id::parse(&s.fridge).unwrap())
        .await
        .unwrap();
    assert_eq!(
        picks
            .iter()
            .map(|x| x.serial_no.as_str())
            .collect::<Vec<_>>(),
        vec!["F-3"]
    );
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn insufficient_stock_blocks_the_sale_without_side_effects_unless_policy_allows() {
    let e = env().await;
    let s = shop(&e).await;
    let r = sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 11)],
            vec![pay("cash", 1_650_000)],
            "k",
        ),
    )
    .await;
    assert!(matches!(r, Err(CoreError::Conflict(_))), "{r:?}");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // a failing LATER line must roll back the earlier line's stock movement
    let mut bad_serial = line(&s.fridge, 1);
    bad_serial.serial_ids = vec!["not-a-serial".into()];
    assert!(sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 2), bad_serial],
            vec![pay("cash", 1)],
            "k2"
        )
    )
    .await
    .is_err());
    let lv = inventory::stock_levels(&e.pool, &e.owner, inventory::StockFilter::default())
        .await
        .unwrap();
    assert_eq!(
        lv.rows.iter().find(|x| x.sku == "FAN").unwrap().available,
        10,
        "atomic: nothing was deducted"
    );
    assert_books_balance(&e).await;
    // policy: allow negative stock -> sells, valuing the shortfall at the variant's list cost
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("stock.allow_negative".into(), "true".into())]),
    )
    .await
    .unwrap();
    sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 12)],
            vec![pay("cash", 1_800_000)],
            "k3",
        ),
    )
    .await
    .unwrap();
    let lv = inventory::stock_levels(&e.pool, &e.owner, inventory::StockFilter::default())
        .await
        .unwrap();
    assert_eq!(
        lv.rows.iter().find(|x| x.sku == "FAN").unwrap().available,
        -2
    );
    assert_eq!(bal(&e, code::COGS).await, 12 * 100_000);
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn discounts_need_permission_and_are_audited_and_bounded() {
    let e = env().await;
    let s = shop(&e).await;
    let cashier = user(&e, "cash", "Cashier").await;
    let mgr = user(&e, "mgr", "Manager").await;
    let mut l = line(&s.fan, 2);
    l.discount = 20_000;
    assert!(forbidden(
        sales::create_sale(
            &e.pool,
            &cashier,
            sale("retail", vec![l.clone()], vec![pay("cash", 280_000)], "a")
        )
        .await
    ));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    let r = sales::create_sale(
        &e.pool,
        &mgr,
        sale("retail", vec![l.clone()], vec![pay("cash", 280_000)], "b"),
    )
    .await
    .unwrap();
    assert_eq!(r.grand_total, 280_000);
    assert_eq!(
        bal(&e, code::SALES).await,
        280_000,
        "revenue is booked net of discount"
    );
    assert_eq!(audit_count(&e.pool, "sale.discount_applied").await, 1);
    l.discount = 300_001;
    assert!(
        sales::create_sale(
            &e.pool,
            &mgr,
            sale("retail", vec![l.clone()], vec![pay("cash", 1)], "c")
        )
        .await
        .is_err(),
        "cannot exceed the line"
    );
    l.discount = -1;
    assert!(sales::create_sale(
        &e.pool,
        &mgr,
        sale("retail", vec![l], vec![pay("cash", 1)], "d")
    )
    .await
    .is_err());
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn tax_is_exact_and_posted_to_tax_payable() {
    let e = env().await;
    let s = shop(&e).await;
    // disabled by default
    let r0 = sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 1)],
            vec![pay("cash", 150_000)],
            "t0",
        ),
    )
    .await
    .unwrap();
    assert_eq!(r0.grand_total, 150_000);
    // exclusive 7.5%
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
    let r1 = sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 3)],
            vec![pay("cash", 483_750)],
            "t1",
        ),
    )
    .await
    .unwrap();
    assert_eq!(r1.grand_total, 450_000 + 33_750);
    assert_eq!(bal(&e, code::TAX_PAYABLE).await, 33_750);
    assert_eq!(bal(&e, code::SALES).await, 150_000 + 450_000);
    // inclusive 7.5%: price already contains tax, total unchanged, tax portion = net*7.5/107.5
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("tax.prices_include_tax".into(), "true".into())]),
    )
    .await
    .unwrap();
    let r2 = sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 1)],
            vec![pay("cash", 150_000)],
            "t2",
        ),
    )
    .await
    .unwrap();
    assert_eq!(r2.grand_total, 150_000);
    let d = sales::get_sale(&e.pool, &e.owner, Id::parse(&r2.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        d.tax_total, 10_465,
        "150000 * 750 / 10750 = 10465.1 -> 10465"
    );
    assert_eq!(bal(&e, code::TAX_PAYABLE).await, 33_750 + 10_465);
    assert_books_balance(&e).await;
    // odd-paisa prices stay exact
    catalogue::update_variant(
        &e.pool,
        &e.owner,
        catalogue::UpdateVariant {
            id: s.fan.clone(),
            sku: "FAN".into(),
            barcode: None,
            colour: "".into(),
            capacity: "".into(),
            size: "".into(),
            purchase_cost: 100_000,
            retail_price: 99_999,
            wholesale_price: 90_001,
            installment_price: None,
        },
    )
    .await
    .unwrap();
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("tax.prices_include_tax".into(), "false".into())]),
    )
    .await
    .unwrap();
    let r3 = sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 3)],
            vec![pay("cash", 322_497)],
            "t3",
        ),
    )
    .await
    .unwrap();
    // 3 x 999.99 = 2999.97 -> tax 7.5% = 224.99775 -> 22499.775 paisa -> rounds (half away from zero) to 22500
    let d = sales::get_sale(&e.pool, &e.owner, Id::parse(&r3.id).unwrap())
        .await
        .unwrap();
    assert_eq!(d.sale.grand_total, 299_997 + d.tax_total);
    assert_eq!(d.tax_total, 22_500);
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn payment_rules_for_retail() {
    let e = env().await;
    let s = shop(&e).await;
    let bad = |pays: Vec<PayLineIn>, k: &str| sale("retail", vec![line(&s.fan, 1)], pays, k);
    assert!(
        sales::create_sale(&e.pool, &e.owner, bad(vec![pay("cash", 149_999)], "a"))
            .await
            .is_err(),
        "retail must be paid in full"
    );
    assert!(sales::create_sale(&e.pool, &e.owner, bad(vec![], "b"))
        .await
        .is_err());
    assert!(
        sales::create_sale(&e.pool, &e.owner, bad(vec![pay("cash", 150_001)], "c"))
            .await
            .is_err(),
        "no overpayment"
    );
    assert!(sales::create_sale(
        &e.pool,
        &e.owner,
        bad(vec![pay("cash", 0), pay("bank", 150_000)], "d")
    )
    .await
    .is_err());
    assert!(
        sales::create_sale(&e.pool, &e.owner, bad(vec![pay("barter", 150_000)], "e"))
            .await
            .is_err()
    );
    assert!(
        sales::create_sale(&e.pool, &e.owner, sale("retail", vec![], vec![], "f"))
            .await
            .is_err()
    );
    assert!(
        sales::create_sale(&e.pool, &e.owner, bad(vec![pay("cash", 150_000)], ""))
            .await
            .is_err(),
        "submission key required"
    );
    assert!(
        sales::create_sale(
            &e.pool,
            &e.owner,
            sale("installment", vec![line(&s.fan, 1)], vec![], "g")
        )
        .await
        .is_err(),
        "installments live in their own module"
    );
    assert!(sales::create_sale(
        &e.pool,
        &e.owner,
        sale("barter", vec![line(&s.fan, 1)], vec![], "h")
    )
    .await
    .is_err());
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn duplicate_submissions_create_one_sale_even_when_concurrent() {
    let e = env().await;
    let s = shop(&e).await;
    let mut hs = vec![];
    for _ in 0..8 {
        let (pool, owner, fan) = (e.pool.clone(), e.owner.clone(), s.fan.clone());
        hs.push(tokio::spawn(async move {
            sales::create_sale(
                &pool,
                &owner,
                sale(
                    "retail",
                    vec![line(&fan, 2)],
                    vec![pay("cash", 300_000)],
                    "same-form",
                ),
            )
            .await
            .map(|r| r.id)
        }));
    }
    let mut ids = std::collections::HashSet::new();
    for h in hs {
        ids.insert(
            h.await
                .unwrap()
                .expect("duplicates return the original sale"),
        );
    }
    assert_eq!(ids.len(), 1);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let lv = inventory::stock_levels(&e.pool, &e.owner, inventory::StockFilter::default())
        .await
        .unwrap();
    assert_eq!(
        lv.rows.iter().find(|x| x.sku == "FAN").unwrap().available,
        8,
        "stock deducted exactly once"
    );
    let pays: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sale_payments")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(pays, 1);
    assert_books_balance(&e).await;
}

// ------------------------------------------------------------------ wholesale credit, receipts, reversals

#[tokio::test]
async fn wholesale_credit_limits_overrides_and_customer_ledger() {
    let e = env().await;
    let s = shop(&e).await;
    let mgr = user(&e, "mgr", "Manager").await;
    let se = user(&e, "sexec", "Sales Executive").await;
    let dealer =
        customers::create_customer(&e.pool, &e.owner, cust_in("Dealer", "dealer", 500_000))
            .await
            .unwrap();
    let mut w = sale(
        "wholesale",
        vec![line(&s.fan, 4)],
        vec![pay("cash", 100_000)],
        "w1",
    );
    // wholesale price 1300 x 4 = 5200, paid 1000, due 4200 (within 5000 limit)
    assert!(
        sales::create_sale(&e.pool, &se, w.clone()).await.is_err(),
        "wholesale needs a customer"
    );
    w.customer_id = Some(dealer.to_string());
    let r = sales::create_sale(&e.pool, &se, w.clone()).await.unwrap();
    assert_eq!((r.grand_total, r.paid, r.due), (520_000, 100_000, 420_000));
    assert_eq!(
        customers::get_customer(&e.pool, &se, dealer)
            .await
            .unwrap()
            .balance,
        420_000
    );
    assert_eq!(bal(&e, code::AR).await, 420_000);
    // second credit sale pushes past the limit -> blocked with a clear message
    let mut w2 = sale("wholesale", vec![line(&s.fan, 2)], vec![], "w2");
    w2.customer_id = Some(dealer.to_string());
    let err = sales::create_sale(&e.pool, &se, w2.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(err, CoreError::Conflict(_)) && err.to_string().contains("credit limit exceeded"),
        "{err}"
    );
    // override needs the permission and is audited
    w2.credit_override = true;
    assert!(forbidden(
        sales::create_sale(&e.pool, &se, w2.clone()).await
    ));
    let r2 = sales::create_sale(&e.pool, &mgr, w2.clone()).await.unwrap();
    assert_eq!(r2.due, 260_000);
    assert_eq!(audit_count(&e.pool, "sale.credit_override").await, 1);
    assert_eq!(
        customers::get_customer(&e.pool, &se, dealer)
            .await
            .unwrap()
            .balance,
        680_000
    );
    // enforcement can be turned off by an authorised admin
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("sales.credit_limit_enforced".into(), "false".into())]),
    )
    .await
    .unwrap();
    let mut w3 = sale("wholesale", vec![line(&s.fan, 1)], vec![], "w3");
    w3.customer_id = Some(dealer.to_string());
    sales::create_sale(&e.pool, &se, w3).await.unwrap();
    // statement lines reconcile to the balance
    let st = customers::statement(&e.pool, &se, dealer).await.unwrap();
    assert_eq!(
        st.lines.iter().map(|l| l.amount).sum::<i64>(),
        st.closing_balance
    );
    assert_eq!(
        st.lines.iter().map(|l| l.kind.as_str()).collect::<Vec<_>>(),
        vec!["sale", "payment", "sale", "sale"]
    );
    // inactive customers cannot buy
    customers::set_customer_active(&e.pool, &e.owner, dealer, false)
        .await
        .unwrap();
    let mut w4 = sale(
        "wholesale",
        vec![line(&s.fan, 1)],
        vec![pay("cash", 130_000)],
        "w4",
    );
    w4.customer_id = Some(dealer.to_string());
    assert!(sales::create_sale(&e.pool, &se, w4).await.is_err());
    // a retail cashier cannot create wholesale sales
    let cashier = user(&e, "cash", "Cashier").await;
    let mut w5 = sale(
        "wholesale",
        vec![line(&s.fan, 1)],
        vec![pay("cash", 130_000)],
        "w5",
    );
    w5.customer_id = Some(dealer.to_string());
    assert!(forbidden(sales::create_sale(&e.pool, &cashier, w5).await));
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn customer_payments_allocate_oldest_first_and_are_idempotent() {
    let e = env().await;
    let s = shop(&e).await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("sales.credit_limit_enforced".into(), "false".into())]),
    )
    .await
    .unwrap();
    let c = customers::create_customer(&e.pool, &e.owner, cust_in("Dealer", "dealer", 0))
        .await
        .unwrap();
    let mk = |k: &str, qty: i64, date: &str| {
        let mut w = sale("wholesale", vec![line(&s.fan, qty)], vec![], k);
        w.customer_id = Some(c.to_string());
        w.sale_date = Some(date.into());
        w
    };
    let a = sales::create_sale(&e.pool, &e.owner, mk("a", 2, "2026-03-01"))
        .await
        .unwrap(); // 2600
    let b = sales::create_sale(&e.pool, &e.owner, mk("b", 3, "2026-03-02"))
        .await
        .unwrap(); // 3900
    let cin = |amount: i64, sale_id: Option<&str>, key: &str| CollectIn {
        customer_id: c.to_string(),
        sale_id: sale_id.map(str::to_string),
        amount,
        method: "cash".into(),
        reference: "".into(),
        paid_on: None,
        client_key: key.into(),
    };
    // 3000 clears invoice A (2600) and puts 400 on B
    let r = sales::collect_payment(&e.pool, &e.owner, cin(300_000, None, "p1"))
        .await
        .unwrap();
    assert_eq!(
        r.allocations,
        vec![(a.id.clone(), 260_000), (b.id.clone(), 40_000)]
    );
    assert_eq!(r.receipts.len(), 2);
    let da = sales::get_sale(&e.pool, &e.owner, Id::parse(&a.id).unwrap())
        .await
        .unwrap();
    let db_ = sales::get_sale(&e.pool, &e.owner, Id::parse(&b.id).unwrap())
        .await
        .unwrap();
    assert_eq!((da.sale.due, db_.sale.due), (0, 350_000));
    // double submit does nothing more
    let again = sales::collect_payment(&e.pool, &e.owner, cin(300_000, None, "p1"))
        .await
        .unwrap();
    assert_eq!(again.receipts, r.receipts);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        350_000
    );
    // named invoice, over-collection, validation
    assert!(
        sales::collect_payment(&e.pool, &e.owner, cin(350_001, Some(&b.id), "p2"))
            .await
            .is_err(),
        "cannot exceed what is owed"
    );
    assert!(
        sales::collect_payment(&e.pool, &e.owner, cin(1, Some(&a.id), "p3"))
            .await
            .is_err(),
        "paid invoice has nothing due"
    );
    assert!(
        sales::collect_payment(&e.pool, &e.owner, cin(0, None, "p4"))
            .await
            .is_err()
    );
    assert!(sales::collect_payment(&e.pool, &e.owner, cin(10, None, ""))
        .await
        .is_err());
    let r2 = sales::collect_payment(&e.pool, &e.owner, cin(350_000, Some(&b.id), "p5"))
        .await
        .unwrap();
    assert_eq!(r2.receipts.len(), 1);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        0
    );
    assert_eq!(bal(&e, code::AR).await, 0);
    assert_eq!(bal(&e, code::CASH).await, 650_000);
    assert_books_balance(&e).await;
    // concurrent duplicate collection
    let mut w3 = sale("wholesale", vec![line(&s.fan, 4)], vec![], "c3");
    w3.customer_id = Some(c.to_string());
    sales::create_sale(&e.pool, &e.owner, w3).await.unwrap(); // 5200 due
    let mut hs = vec![];
    for _ in 0..6 {
        let (pool, owner, cc) = (e.pool.clone(), e.owner.clone(), c.to_string());
        hs.push(tokio::spawn(async move {
            sales::collect_payment(
                &pool,
                &owner,
                CollectIn {
                    customer_id: cc,
                    sale_id: None,
                    amount: 100_000,
                    method: "cash".into(),
                    reference: "".into(),
                    paid_on: None,
                    client_key: "dup".into(),
                },
            )
            .await
            .map(|r| r.receipts)
        }));
    }
    let mut got = std::collections::HashSet::new();
    for h in hs {
        got.insert(format!("{:?}", h.await.unwrap().unwrap()));
    }
    assert_eq!(got.len(), 1);
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        520_000 - 100_000
    );
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn payment_reversal_restores_the_receivable_and_needs_permission() {
    let e = env().await;
    let s = shop(&e).await;
    let cashier = user(&e, "cash", "Cashier").await;
    settings::update(
        &e.pool,
        &e.owner,
        HashMap::from([("sales.credit_limit_enforced".into(), "false".into())]),
    )
    .await
    .unwrap();
    let c = customers::create_customer(&e.pool, &e.owner, cust_in("Dealer", "dealer", 0))
        .await
        .unwrap();
    let mut w = sale(
        "wholesale",
        vec![line(&s.fan, 2)],
        vec![pay("bank", 100_000)],
        "w",
    );
    w.customer_id = Some(c.to_string());
    let r = sales::create_sale(&e.pool, &e.owner, w).await.unwrap();
    let det = sales::get_sale(&e.pool, &e.owner, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    let pid = Id::parse(&det.payments[0].id).unwrap();
    assert!(forbidden(
        sales::reverse_payment(&e.pool, &cashier, pid, "oops").await
    ));
    assert!(sales::reverse_payment(&e.pool, &e.owner, pid, " ")
        .await
        .is_err());
    sales::reverse_payment(&e.pool, &e.owner, pid, "cheque bounced")
        .await
        .unwrap();
    assert_eq!(
        customers::get_customer(&e.pool, &e.owner, c)
            .await
            .unwrap()
            .balance,
        260_000
    );
    assert_eq!(bal(&e, code::BANK).await, 0);
    assert_eq!(bal(&e, code::AR).await, 260_000);
    assert!(sales::reverse_payment(&e.pool, &e.owner, pid, "again")
        .await
        .is_err());
    let det = sales::get_sale(&e.pool, &e.owner, Id::parse(&r.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (det.sale.paid, det.sale.due, det.payments.len()),
        (0, 260_000, 2)
    );
    let rev = Id::parse(&det.payments[1].id).unwrap();
    assert!(
        sales::reverse_payment(&e.pool, &e.owner, rev, "undo")
            .await
            .is_err(),
        "reversals are final"
    );
    // history is never deleted
    assert!(sqlx::query("DELETE FROM sale_payments")
        .execute(&e.pool)
        .await
        .is_err());
    assert!(sqlx::query("UPDATE customer_ledger_entries SET amount = 1")
        .execute(&e.pool)
        .await
        .is_err());
    assert_eq!(audit_count(&e.pool, "customer.payment_reversed").await, 1);
    assert_books_balance(&e).await;
}

#[tokio::test]
async fn sales_list_filters_and_read_permissions() {
    let e = env().await;
    let s = shop(&e).await;
    let acct = user(&e, "acct", "Accountant").await;
    let c =
        customers::create_customer(&e.pool, &e.owner, cust_in("Karim", "wholesale", 10_000_000))
            .await
            .unwrap();
    sales::create_sale(
        &e.pool,
        &e.owner,
        sale(
            "retail",
            vec![line(&s.fan, 1)],
            vec![pay("cash", 150_000)],
            "r1",
        ),
    )
    .await
    .unwrap();
    let mut w = sale(
        "wholesale",
        vec![line(&s.fan, 2)],
        vec![pay("cash", 60_000)],
        "w1",
    );
    w.customer_id = Some(c.to_string());
    sales::create_sale(&e.pool, &e.owner, w).await.unwrap();
    let all = sales::list_sales(&e.pool, &acct, SaleFilter::default())
        .await
        .unwrap();
    assert_eq!(all.total, 2);
    assert_eq!(
        sales::list_sales(
            &e.pool,
            &acct,
            SaleFilter {
                mode: Some("wholesale".into()),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .rows[0]
            .due,
        200_000
    );
    assert_eq!(
        sales::list_sales(
            &e.pool,
            &acct,
            SaleFilter {
                due_only: true,
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .total,
        1
    );
    assert_eq!(
        sales::list_sales(
            &e.pool,
            &acct,
            SaleFilter {
                search: Some("karim".into()),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .total,
        1
    );
    assert_eq!(
        sales::list_sales(
            &e.pool,
            &acct,
            SaleFilter {
                search: Some("%".into()),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .total,
        0
    );
    let noperm = {
        let mut p = acct.clone();
        p.permissions.remove("sales.view");
        p
    };
    assert!(forbidden(
        sales::list_sales(&e.pool, &noperm, SaleFilter::default()).await
    ));
    assert!(forbidden(
        sales::get_sale(&e.pool, &noperm, Id::new()).await
    ));
    assert!(forbidden(
        sales::available_serials(&e.pool, &acct, Id::new()).await
    ));
}

#[tokio::test]
async fn reconciliation_detects_tampering_with_the_ledgers() {
    let e = env().await;
    let s = shop(&e).await;
    let c =
        customers::create_customer(&e.pool, &e.owner, cust_in("Karim", "wholesale", 10_000_000))
            .await
            .unwrap();
    let mut w = sale("wholesale", vec![line(&s.fan, 1)], vec![], "w");
    w.customer_id = Some(c.to_string());
    sales::create_sale(&e.pool, &e.owner, w).await.unwrap();
    assert_books_balance(&e).await;
    // a customer ledger row with no matching journal
    sqlx::query("INSERT INTO customer_ledger_entries (id, customer_id, at, kind, amount, doc_type, doc_id) VALUES ('x', ?, '2026-01-01T00:00:00.000Z', 'adjustment', 999, 'manual', 'm1')").bind(c.to_string()).execute(&e.pool).await.unwrap();
    let m = accounting::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(
        m.iter()
            .any(|x| x.check == "customer_ledger_vs_receivables" && x.expected - x.actual == 999),
        "{m:?}"
    );
    // stock value changed behind the journals' back
    sqlx::query(
        "UPDATE stock_balances SET value = value + 7 WHERE bucket = 'available' AND variant_id = ?",
    )
    .bind(&s.fan)
    .execute(&e.pool)
    .await
    .unwrap();
    let m = accounting::reconcile(&e.pool, &e.owner).await.unwrap();
    assert!(
        m.iter()
            .any(|x| x.check == "stock_value_vs_inventory_account"),
        "{m:?}"
    );
}

#[tokio::test]
async fn p3_services_require_the_right_permissions() {
    let e = env().await;
    let s = shop(&e).await;
    let inv = user(&e, "inv", "Inventory Manager").await; // no sales/customers/accounts permissions
    assert!(forbidden(
        sales::create_sale(
            &e.pool,
            &inv,
            sale(
                "retail",
                vec![line(&s.fan, 1)],
                vec![pay("cash", 150_000)],
                "k"
            )
        )
        .await
    ));
    assert!(forbidden(
        customers::list_customers(&e.pool, &inv, None, None, false, false).await
    ));
    assert!(forbidden(
        customers::create_customer(&e.pool, &inv, cust_in("X", "retail", 0)).await
    ));
    assert!(forbidden(
        accounting::trial_balance(&e.pool, &inv, None).await
    ));
    assert!(forbidden(
        accounting::ledger(&e.pool, &inv, "1000", None, None).await
    ));
    assert!(forbidden(
        accounting::list_entries(&e.pool, &inv, Default::default()).await
    ));
    assert!(forbidden(
        accounting::get_entry(&e.pool, &inv, Id::new()).await
    ));
    assert!(forbidden(accounting::reconcile(&e.pool, &inv).await));
    assert!(forbidden(
        sales::collect_payment(
            &e.pool,
            &inv,
            CollectIn {
                customer_id: "x".into(),
                sale_id: None,
                amount: 1,
                method: "cash".into(),
                reference: "".into(),
                paid_on: None,
                client_key: "k".into()
            }
        )
        .await
    ));
    assert!(forbidden(
        sales::reverse_payment(&e.pool, &inv, Id::new(), "x").await
    ));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn preview_matches_the_posted_sale_exactly_and_has_no_side_effects() {
    let e = env().await;
    let s = shop(&e).await;
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
    let mut l = line(&s.fan, 3);
    l.discount = 12_345;
    let si = sale("retail", vec![l], vec![], "k");
    let pv = sales::preview_sale(&e.pool, &e.owner, &si).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    let mv: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stock_movements WHERE doc_type = 'sale'")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!((n, mv), (0, 0), "preview must not change anything");
    let real = sales::create_sale(
        &e.pool,
        &e.owner,
        SaleIn {
            payments: vec![pay("cash", pv.grand_total)],
            ..si.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(real.grand_total, pv.grand_total);
    let d = sales::get_sale(&e.pool, &e.owner, Id::parse(&real.id).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (d.subtotal, d.discount_total, d.tax_total),
        (pv.subtotal, pv.discount_total, pv.tax_total)
    );
    // permissions: a cashier cannot preview a discounted or wholesale cart
    let cashier = user(&e, "cash", "Cashier").await;
    assert!(forbidden(sales::preview_sale(&e.pool, &cashier, &si).await));
    let w = sale("wholesale", vec![line(&s.fan, 1)], vec![], "k2");
    assert!(forbidden(sales::preview_sale(&e.pool, &cashier, &w).await));
    // wholesale uses the wholesale price
    let pvw = sales::preview_sale(&e.pool, &e.owner, &w).await.unwrap();
    assert_eq!(pvw.lines[0].unit_price, 130_000);
}
