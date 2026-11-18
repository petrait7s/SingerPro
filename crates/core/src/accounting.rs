//! Double-entry accounting. Every business event posts ONE idempotent, balanced, immutable journal
//! entry (keyed by `posting_key`). Corrections are reversal entries, never edits.
//!
//! Chart of accounts and the posting rules are documented in docs/ACCOUNTING_POLICY.md.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::{sequence, time, CoreError, CoreResult, Id};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

pub mod code {
    pub const CASH: &str = "1000";
    pub const BANK: &str = "1010";
    pub const WALLET: &str = "1020";
    pub const AR: &str = "1100";
    pub const INSTALLMENT_AR: &str = "1110";
    pub const INVENTORY: &str = "1200";
    pub const IN_TRANSIT: &str = "1210";
    pub const AP: &str = "2000";
    pub const UNEARNED_MARKUP: &str = "2100";
    pub const ACCRUED_LANDED: &str = "2150";
    pub const TAX_PAYABLE: &str = "2200";
    pub const CUSTOMER_ADVANCES: &str = "2300";
    pub const OPENING_EQUITY: &str = "3000";
    pub const OWNER_CAPITAL: &str = "3100";
    pub const SALES: &str = "4000";
    pub const SALES_RETURNS: &str = "4010";
    pub const MARKUP_INCOME: &str = "4100";
    pub const OTHER_INCOME: &str = "4900";
    pub const COGS: &str = "5000";
    pub const INVENTORY_ADJUSTMENTS: &str = "5100";
    pub const PURCHASE_VARIANCE: &str = "5110";
    pub const MISC_EXPENSE: &str = "6900";
}

/// (code, name, type, normal side)
pub const CHART: &[(&str, &str, &str, &str)] = &[
    ("1000", "Cash in Hand", "asset", "debit"),
    ("1010", "Bank Accounts", "asset", "debit"),
    ("1020", "Mobile Wallets", "asset", "debit"),
    ("1100", "Accounts Receivable - Customers", "asset", "debit"),
    ("1110", "Installment Receivable", "asset", "debit"),
    ("1200", "Inventory", "asset", "debit"),
    (
        "1210",
        "Inventory in Transit (confirmed, not yet received)",
        "asset",
        "debit",
    ),
    (
        "2000",
        "Accounts Payable - Suppliers",
        "liability",
        "credit",
    ),
    ("2100", "Unearned Installment Markup", "liability", "credit"),
    ("2150", "Accrued Landed Costs", "liability", "credit"),
    ("2200", "Tax Payable", "liability", "credit"),
    (
        "2300",
        "Customer Advances and Refunds Payable",
        "liability",
        "credit",
    ),
    ("3000", "Opening Balance Equity", "equity", "credit"),
    ("3100", "Owner's Capital", "equity", "credit"),
    ("4000", "Sales Revenue", "income", "credit"),
    ("4010", "Sales Returns and Allowances", "income", "debit"),
    ("4100", "Installment Markup Income", "income", "credit"),
    ("4900", "Other Income", "income", "credit"),
    ("5000", "Cost of Goods Sold", "expense", "debit"),
    (
        "5100",
        "Inventory Adjustments and Write-offs",
        "expense",
        "debit",
    ),
    ("5110", "Purchase Return Variance", "expense", "debit"),
    ("6000", "Rent", "expense", "debit"),
    ("6010", "Salaries and Wages", "expense", "debit"),
    ("6020", "Utilities", "expense", "debit"),
    ("6030", "Transport and Delivery", "expense", "debit"),
    ("6040", "Marketing and Advertising", "expense", "debit"),
    ("6050", "Repairs and Maintenance", "expense", "debit"),
    ("6060", "Office and Administration", "expense", "debit"),
    ("6070", "Bank Charges", "expense", "debit"),
    ("6900", "Miscellaneous Expense", "expense", "debit"),
];

pub async fn sync_accounts(pool: &SqlitePool) -> CoreResult<()> {
    let now = time::now_utc_string();
    for (c, n, t, s) in CHART {
        sqlx::query("INSERT OR IGNORE INTO accounts (id, code, name, type, normal_side, is_system, created_at) VALUES (?, ?, ?, ?, ?, 1, ?)")
            .bind(Id::new().to_string())
            .bind(c)
            .bind(n)
            .bind(t)
            .bind(s)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub fn method_account(method: &str) -> &'static str {
    match method {
        "bank" | "cheque" => code::BANK,
        "mobile_wallet" => code::WALLET,
        _ => code::CASH,
    }
}

/// Business date (Asia/Dhaka) of a stored UTC timestamp.
pub fn dhaka_date(ts: &str) -> String {
    DateTime::parse_from_rfc3339(ts)
        .map(|t| {
            time::to_dhaka(t.with_timezone(&Utc))
                .date_naive()
                .to_string()
        })
        .unwrap_or_else(|_| time::today_dhaka().to_string())
}

pub struct Line<'a> {
    pub account: &'a str,
    pub debit: i64,
    pub credit: i64,
    pub memo: &'a str,
}

pub fn dr<'a>(account: &'a str, amount: i64, memo: &'a str) -> Line<'a> {
    Line {
        account,
        debit: amount,
        credit: 0,
        memo,
    }
}

pub fn cr<'a>(account: &'a str, amount: i64, memo: &'a str) -> Line<'a> {
    Line {
        account,
        debit: 0,
        credit: amount,
        memo,
    }
}

pub struct PostIn<'a> {
    pub key: &'a str,
    pub date: &'a str,
    pub memo: &'a str,
    pub source_type: &'a str,
    pub source_id: &'a str,
    pub lines: Vec<Line<'a>>,
    pub reverses: Option<Id>,
    pub actor: Option<Id>,
}

/// Posts a balanced entry. Idempotent: an existing `key` is a no-op (`None`); all-zero entries are skipped.
pub async fn post(conn: &mut SqliteConnection, p: &PostIn<'_>) -> CoreResult<Option<Id>> {
    let lines: Vec<&Line> = p
        .lines
        .iter()
        .filter(|l| l.debit != 0 || l.credit != 0)
        .collect();
    if lines.is_empty() {
        return Ok(None);
    }
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE posting_key = ?")
            .bind(p.key)
            .fetch_one(&mut *conn)
            .await?;
    if exists > 0 {
        return Ok(None);
    }
    for l in &lines {
        if l.debit < 0 || l.credit < 0 || (l.debit != 0 && l.credit != 0) {
            return Err(CoreError::Integrity(format!(
                "invalid journal line for account {}",
                l.account
            )));
        }
    }
    let (d, c): (i64, i64) = lines
        .iter()
        .fold((0, 0), |a, l| (a.0 + l.debit, a.1 + l.credit));
    if d != c {
        return Err(CoreError::Integrity(format!(
            "unbalanced journal '{}': debits {d} != credits {c}",
            p.key
        )));
    }
    NaiveDate::parse_from_str(p.date, "%Y-%m-%d")
        .map_err(|_| CoreError::Validation("journal date must be YYYY-MM-DD".into()))?;
    let mut account_ids = Vec::with_capacity(lines.len());
    for l in &lines {
        let acct: Option<String> = sqlx::query_scalar("SELECT id FROM accounts WHERE code = ?")
            .bind(l.account)
            .fetch_optional(&mut *conn)
            .await?;
        account_ids.push(
            acct.ok_or_else(|| CoreError::Validation(format!("unknown account {}", l.account)))?,
        );
    }
    sequence::ensure(&mut *conn, "journal", "JE-", 6).await?;
    let number = sequence::next(&mut *conn, "journal").await?;
    let id = Id::new();
    sqlx::query("INSERT INTO journal_entries (id, number, entry_date, memo, source_type, source_id, posting_key, reverses_id, actor_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&number)
        .bind(p.date)
        .bind(p.memo)
        .bind(p.source_type)
        .bind(p.source_id)
        .bind(p.key)
        .bind(p.reverses.map(|r| r.to_string()))
        .bind(p.actor.map(|a| a.to_string()))
        .bind(time::now_utc_string())
        .execute(&mut *conn)
        .await?;
    for (i, l) in lines.iter().enumerate() {
        sqlx::query("INSERT INTO journal_lines (id, entry_id, line_no, account_id, debit, credit, memo) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(Id::new().to_string())
            .bind(id.to_string())
            .bind(i as i64 + 1)
            .bind(&account_ids[i])
            .bind(l.debit)
            .bind(l.credit)
            .bind(l.memo)
            .execute(&mut *conn)
            .await?;
    }
    Ok(Some(id))
}

// ------------------------------------------------------------------ posting builders (P2 documents)

pub(crate) async fn post_purchase_confirmed(
    conn: &mut SqliteConnection,
    purchase_id: &str,
) -> CoreResult<()> {
    let r: Option<(String, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT number, grand_total, landed_total, confirmed_at FROM purchases WHERE id = ?",
    )
    .bind(purchase_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((number, grand, landed, Some(at))) = r else {
        return Ok(());
    };
    let key = format!("purchase:{purchase_id}:confirm");
    let memo = format!("Purchase {number} confirmed");
    post(
        conn,
        &PostIn {
            key: &key,
            date: &dhaka_date(&at),
            memo: &memo,
            source_type: "purchase",
            source_id: purchase_id,
            lines: vec![
                dr(code::IN_TRANSIT, grand + landed, "Goods ordered"),
                cr(code::AP, grand, "Supplier payable"),
                cr(code::ACCRUED_LANDED, landed, "Landed costs"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_purchase_cancelled(
    conn: &mut SqliteConnection,
    purchase_id: &str,
) -> CoreResult<()> {
    type CancelRow = (String, i64, i64, Option<String>, Option<String>);
    let r: Option<CancelRow> = sqlx::query_as("SELECT number, grand_total, landed_total, confirmed_at, cancelled_at FROM purchases WHERE id = ? AND status = 'cancelled'").bind(purchase_id).fetch_optional(&mut *conn).await?;
    let Some((number, grand, landed, Some(_), Some(at))) = r else {
        return Ok(());
    };
    let key = format!("purchase:{purchase_id}:cancel");
    let memo = format!("Purchase {number} cancelled");
    post(
        conn,
        &PostIn {
            key: &key,
            date: &dhaka_date(&at),
            memo: &memo,
            source_type: "purchase",
            source_id: purchase_id,
            lines: vec![
                dr(code::AP, grand, "Supplier payable reversed"),
                dr(code::ACCRUED_LANDED, landed, "Landed costs reversed"),
                cr(code::IN_TRANSIT, grand + landed, "Order cancelled"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_purchase_receipt(
    conn: &mut SqliteConnection,
    receipt_id: &str,
) -> CoreResult<()> {
    let r: Option<(String, String, i64)> = sqlx::query_as(
        "SELECT p.number, r.received_on, COALESCE((SELECT SUM(cost_total) FROM purchase_receipt_items WHERE receipt_id = r.id), 0)
         FROM purchase_receipts r JOIN purchases p ON p.id = r.purchase_id WHERE r.id = ?",
    )
    .bind(receipt_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((number, date, cost)) = r else {
        return Ok(());
    };
    let key = format!("receipt:{receipt_id}");
    let memo = format!("Goods received on {number}");
    post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &memo,
            source_type: "purchase_receipt",
            source_id: receipt_id,
            lines: vec![
                dr(code::INVENTORY, cost, "Stock received"),
                cr(code::IN_TRANSIT, cost, "Cleared from transit"),
            ],
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_purchase_return(
    conn: &mut SqliteConnection,
    return_id: &str,
) -> CoreResult<()> {
    let r: Option<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT r.number, r.returned_on, r.credit_total, COALESCE((SELECT SUM(cost_total) FROM purchase_return_items WHERE return_id = r.id), 0) FROM purchase_returns r WHERE r.id = ?",
    )
    .bind(return_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((number, date, credit, cost)) = r else {
        return Ok(());
    };
    let key = format!("purchase_return:{return_id}");
    let memo = format!("Return to supplier {number}");
    let diff = credit - cost;
    let mut lines = vec![
        dr(code::AP, credit, "Supplier credit"),
        cr(code::INVENTORY, cost, "Stock returned"),
    ];
    if diff > 0 {
        lines.push(cr(code::PURCHASE_VARIANCE, diff, "Credit above stock cost"));
    } else if diff < 0 {
        lines.push(dr(
            code::PURCHASE_VARIANCE,
            -diff,
            "Stock cost above credit",
        ));
    }
    post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: &memo,
            source_type: "purchase_return",
            source_id: return_id,
            lines,
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_supplier_payment(
    conn: &mut SqliteConnection,
    payment_id: &str,
) -> CoreResult<()> {
    let r: Option<(i64, String, String)> =
        sqlx::query_as("SELECT amount, method, paid_on FROM supplier_payments WHERE id = ?")
            .bind(payment_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((amount, method, date)) = r else {
        return Ok(());
    };
    let key = format!("supplier_payment:{payment_id}");
    let acct = method_account(&method);
    let lines = if amount > 0 {
        vec![
            dr(code::AP, amount, "Supplier paid"),
            cr(acct, amount, "Payment out"),
        ]
    } else {
        vec![
            dr(acct, -amount, "Payment reversed"),
            cr(code::AP, -amount, "Payable restored"),
        ]
    };
    post(
        conn,
        &PostIn {
            key: &key,
            date: &date,
            memo: if amount > 0 {
                "Supplier payment"
            } else {
                "Supplier payment reversal"
            },
            source_type: "supplier_payment",
            source_id: payment_id,
            lines,
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn post_stock_adjustment(
    conn: &mut SqliteConnection,
    adjustment_id: &str,
) -> CoreResult<()> {
    let r: Option<(String, String, String)> =
        sqlx::query_as("SELECT number, kind, at FROM stock_adjustments WHERE id = ?")
            .bind(adjustment_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((number, kind, at)) = r else {
        return Ok(());
    };
    let net: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(cost_total), 0) FROM stock_movements WHERE doc_type = 'stock_adjustment' AND doc_id = ?").bind(adjustment_id).fetch_one(&mut *conn).await?;
    let key = format!("stock_adjustment:{adjustment_id}");
    let memo = format!("Stock adjustment {number} ({kind})");
    let contra = if kind == "opening" {
        code::OPENING_EQUITY
    } else {
        code::INVENTORY_ADJUSTMENTS
    };
    let lines = if net >= 0 {
        vec![dr(code::INVENTORY, net, "Stock in"), cr(contra, net, "")]
    } else {
        vec![dr(contra, -net, ""), cr(code::INVENTORY, -net, "Stock out")]
    };
    post(
        conn,
        &PostIn {
            key: &key,
            date: &dhaka_date(&at),
            memo: &memo,
            source_type: "stock_adjustment",
            source_id: adjustment_id,
            lines,
            reverses: None,
            actor: None,
        },
    )
    .await?;
    Ok(())
}

/// Posts any document that has no journal entry yet (upgrade backfill and repair). Idempotent.
pub async fn ensure_posted(pool: &SqlitePool) -> CoreResult<u64> {
    let mut n = 0u64;
    macro_rules! each {
        ($sql:expr, $f:path) => {{
            let ids: Vec<String> = sqlx::query_scalar($sql).fetch_all(pool).await?;
            for id in ids {
                let mut tx = pool.begin().await?;
                $f(&mut tx, &id).await?;
                tx.commit().await?;
                n += 1;
            }
        }};
    }
    each!("SELECT id FROM purchases WHERE confirmed_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'purchase:' || purchases.id || ':confirm')", post_purchase_confirmed);
    each!("SELECT id FROM purchases WHERE status = 'cancelled' AND confirmed_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'purchase:' || purchases.id || ':cancel')", post_purchase_cancelled);
    each!("SELECT id FROM purchase_receipts WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'receipt:' || purchase_receipts.id)", post_purchase_receipt);
    each!("SELECT id FROM purchase_returns WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'purchase_return:' || purchase_returns.id)", post_purchase_return);
    each!("SELECT id FROM supplier_payments WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'supplier_payment:' || supplier_payments.id)", post_supplier_payment);
    each!("SELECT id FROM stock_adjustments WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'stock_adjustment:' || stock_adjustments.id)", post_stock_adjustment);
    n += crate::sales::ensure_posted(pool).await?;
    Ok(n)
}

// ------------------------------------------------------------------ queries

#[derive(Debug, Serialize)]
pub struct AccountRow {
    pub id: String,
    pub code: String,
    pub name: String,
    pub account_type: String,
    pub normal_side: String,
    pub is_system: bool,
    pub is_active: bool,
    pub debit: i64,
    pub credit: i64,
    /// Signed in the account's normal direction (a negative number means the opposite side).
    pub balance: i64,
}

fn signed(normal: &str, d: i64, c: i64) -> i64 {
    if normal == "debit" {
        d - c
    } else {
        c - d
    }
}

pub async fn list_accounts(
    pool: &SqlitePool,
    p: &Principal,
    as_of: Option<&str>,
) -> CoreResult<Vec<AccountRow>> {
    p.require(perm::ACCOUNTS_VIEW)?;
    type Raw = (String, String, String, String, String, i64, i64, i64, i64);
    let rows: Vec<Raw> = sqlx::query_as(
        "SELECT a.id, a.code, a.name, a.type, a.normal_side, a.is_system, a.is_active,
            COALESCE(SUM(CASE WHEN e.id IS NOT NULL THEN l.debit END), 0), COALESCE(SUM(CASE WHEN e.id IS NOT NULL THEN l.credit END), 0)
         FROM accounts a LEFT JOIN journal_lines l ON l.account_id = a.id LEFT JOIN journal_entries e ON e.id = l.entry_id AND (?1 IS NULL OR e.entry_date <= ?1)
         GROUP BY a.id ORDER BY a.code",
    )
    .bind(as_of)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| AccountRow {
            balance: signed(&r.4, r.7, r.8),
            id: r.0,
            code: r.1,
            name: r.2,
            account_type: r.3,
            normal_side: r.4,
            is_system: r.5 == 1,
            is_active: r.6 == 1,
            debit: r.7,
            credit: r.8,
        })
        .collect())
}

#[derive(Debug, Serialize)]
pub struct TrialBalance {
    pub as_of: Option<String>,
    pub rows: Vec<AccountRow>,
    pub total_debit: i64,
    pub total_credit: i64,
    pub balanced: bool,
}

pub async fn trial_balance(
    pool: &SqlitePool,
    p: &Principal,
    as_of: Option<&str>,
) -> CoreResult<TrialBalance> {
    let rows: Vec<AccountRow> = list_accounts(pool, p, as_of)
        .await?
        .into_iter()
        .filter(|a| a.debit != 0 || a.credit != 0)
        .collect();
    // Trial balance shows each account's NET on its natural side.
    let (mut td, mut tc) = (0i64, 0i64);
    for r in &rows {
        let net = r.debit - r.credit;
        if net >= 0 {
            td += net
        } else {
            tc += -net
        }
    }
    Ok(TrialBalance {
        as_of: as_of.map(str::to_string),
        rows,
        total_debit: td,
        total_credit: tc,
        balanced: td == tc,
    })
}

#[derive(Debug, Serialize)]
pub struct LedgerLine {
    pub entry_id: String,
    pub number: String,
    pub date: String,
    pub memo: String,
    pub line_memo: String,
    pub debit: i64,
    pub credit: i64,
    pub balance: i64,
}

#[derive(Debug, Serialize)]
pub struct Ledger {
    pub account: String,
    pub name: String,
    pub opening: i64,
    pub lines: Vec<LedgerLine>,
    pub closing: i64,
}

pub async fn ledger(
    pool: &SqlitePool,
    p: &Principal,
    account_code: &str,
    from: Option<&str>,
    to: Option<&str>,
) -> CoreResult<Ledger> {
    p.require(perm::ACCOUNTS_VIEW)?;
    let (id, name, normal): (String, String, String) =
        sqlx::query_as("SELECT id, name, normal_side FROM accounts WHERE code = ?")
            .bind(account_code)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| CoreError::NotFound("account".into()))?;
    let (od, oc): (i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id WHERE l.account_id = ? AND ?2 IS NOT NULL AND e.entry_date < ?2")
        .bind(&id)
        .bind(from)
        .fetch_one(pool)
        .await?;
    let opening = signed(&normal, od, oc);
    let rows: Vec<(String, String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT e.id, e.number, e.entry_date, e.memo, l.memo, l.debit, l.credit FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id
         WHERE l.account_id = ?1 AND (?2 IS NULL OR e.entry_date >= ?2) AND (?3 IS NULL OR e.entry_date <= ?3) ORDER BY e.entry_date, e.number, l.line_no",
    )
    .bind(&id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    let mut bal = opening;
    let lines = rows
        .into_iter()
        .map(|(entry_id, number, date, memo, line_memo, debit, credit)| {
            bal += signed(&normal, debit, credit);
            LedgerLine {
                entry_id,
                number,
                date,
                memo,
                line_memo,
                debit,
                credit,
                balance: bal,
            }
        })
        .collect();
    Ok(Ledger {
        account: account_code.to_string(),
        name,
        opening,
        lines,
        closing: bal,
    })
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EntryRow {
    pub id: String,
    pub number: String,
    pub entry_date: String,
    pub memo: String,
    pub source_type: String,
    pub source_id: String,
    pub total: i64,
    pub reverses_id: Option<String>,
    pub reversed: bool,
}

#[derive(Debug, Serialize)]
pub struct EntryPage {
    pub total: i64,
    pub rows: Vec<EntryRow>,
}

#[derive(Debug, Default, Deserialize)]
pub struct EntryFilter {
    pub from: Option<String>,
    pub to: Option<String>,
    pub search: Option<String>,
    pub source_type: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_entries(
    pool: &SqlitePool,
    p: &Principal,
    f: EntryFilter,
) -> CoreResult<EntryPage> {
    p.require(perm::ACCOUNTS_VIEW)?;
    let like = f.search.as_deref().map(crate::catalogue::like_pattern);
    let w = "WHERE (?1 IS NULL OR e.entry_date >= ?1) AND (?2 IS NULL OR e.entry_date <= ?2) AND (?3 IS NULL OR e.memo LIKE ?3 OR e.number LIKE ?3) AND (?4 IS NULL OR e.source_type = ?4)";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM journal_entries e {w}"))
        .bind(&f.from)
        .bind(&f.to)
        .bind(&like)
        .bind(&f.source_type)
        .fetch_one(pool)
        .await?;
    let rows = sqlx::query_as(&format!(
        "SELECT e.id, e.number, e.entry_date, e.memo, e.source_type, e.source_id, COALESCE((SELECT SUM(debit) FROM journal_lines WHERE entry_id = e.id), 0) AS total, e.reverses_id,
            EXISTS (SELECT 1 FROM journal_entries r WHERE r.reverses_id = e.id) AS reversed
         FROM journal_entries e {w} ORDER BY e.entry_date DESC, e.number DESC LIMIT ?5 OFFSET ?6"
    ))
    .bind(&f.from)
    .bind(&f.to)
    .bind(&like)
    .bind(&f.source_type)
    .bind(f.limit.unwrap_or(50).clamp(1, 200))
    .bind(f.offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await?;
    Ok(EntryPage { total, rows })
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EntryLineRow {
    pub code: String,
    pub name: String,
    pub debit: i64,
    pub credit: i64,
    pub memo: String,
}

#[derive(Debug, Serialize)]
pub struct EntryDetail {
    pub entry: EntryRow,
    pub lines: Vec<EntryLineRow>,
}

pub async fn get_entry(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<EntryDetail> {
    p.require(perm::ACCOUNTS_VIEW)?;
    let entry: EntryRow = sqlx::query_as(
        "SELECT e.id, e.number, e.entry_date, e.memo, e.source_type, e.source_id, COALESCE((SELECT SUM(debit) FROM journal_lines WHERE entry_id = e.id), 0) AS total, e.reverses_id,
            EXISTS (SELECT 1 FROM journal_entries r WHERE r.reverses_id = e.id) AS reversed FROM journal_entries e WHERE e.id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| CoreError::NotFound("journal entry".into()))?;
    let lines = sqlx::query_as("SELECT a.code, a.name, l.debit, l.credit, l.memo FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE l.entry_id = ? ORDER BY l.line_no").bind(id.to_string()).fetch_all(pool).await?;
    Ok(EntryDetail { entry, lines })
}

#[derive(Debug, Deserialize)]
pub struct ManualLine {
    pub account_code: String,
    pub debit: i64,
    pub credit: i64,
    pub memo: String,
}

/// Accountant-entered journal (needs `accounts.manage`, a reason, and must balance).
pub async fn manual_journal(
    pool: &SqlitePool,
    p: &Principal,
    date: &str,
    memo: &str,
    lines: Vec<ManualLine>,
) -> CoreResult<Id> {
    p.require(perm::ACCOUNTS_MANAGE)?;
    if memo.trim().is_empty() {
        return Err(CoreError::Validation("a description is required".into()));
    }
    if lines.len() < 2 {
        return Err(CoreError::Validation(
            "a journal needs at least two lines".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    for l in &lines {
        let ok: Option<i64> = sqlx::query_scalar("SELECT is_active FROM accounts WHERE code = ?")
            .bind(&l.account_code)
            .fetch_optional(&mut *tx)
            .await?;
        match ok {
            None => {
                return Err(CoreError::Validation(format!(
                    "unknown account {}",
                    l.account_code
                )))
            }
            Some(0) => {
                return Err(CoreError::Validation(format!(
                    "account {} is inactive",
                    l.account_code
                )))
            }
            _ => {}
        }
        if l.debit < 0 || l.credit < 0 || (l.debit > 0) == (l.credit > 0) {
            return Err(CoreError::Validation(
                "each line needs either a debit or a credit amount".into(),
            ));
        }
    }
    let key = format!("manual:{}", Id::new());
    let pl: Vec<Line> = lines
        .iter()
        .map(|l| Line {
            account: &l.account_code,
            debit: l.debit,
            credit: l.credit,
            memo: &l.memo,
        })
        .collect();
    let id = post(
        &mut tx,
        &PostIn {
            key: &key,
            date,
            memo: memo.trim(),
            source_type: "manual",
            source_id: &key,
            lines: pl,
            reverses: None,
            actor: Some(p.user_id),
        },
    )
    .await?
    .ok_or_else(|| CoreError::Validation("journal has no amounts".into()))?;
    audit::log(
        &mut tx,
        p,
        "journal.manual",
        "journal_entry",
        &id.to_string(),
        serde_json::json!({"memo": memo.trim(), "lines": lines.len()}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Reverses a manual/opening entry with an equal and opposite entry. Document-generated entries are
/// corrected through their own workflows (returns, payment reversals, cancellations).
pub async fn reverse_entry(
    pool: &SqlitePool,
    p: &Principal,
    entry_id: Id,
    reason: &str,
) -> CoreResult<Id> {
    p.require(perm::ACCOUNTS_MANAGE)?;
    if reason.trim().is_empty() {
        return Err(CoreError::Validation("a reason is required".into()));
    }
    let eid = entry_id.to_string();
    let mut tx = pool.begin().await?;
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT number, source_type, memo, reverses_id FROM journal_entries WHERE id = ?",
    )
    .bind(&eid)
    .fetch_optional(&mut *tx)
    .await?;
    let (number, source, memo, reverses) =
        row.ok_or_else(|| CoreError::NotFound("journal entry".into()))?;
    if reverses.is_some() {
        return Err(CoreError::Conflict("a reversal cannot be reversed".into()));
    }
    if !matches!(source.as_str(), "manual" | "opening") {
        return Err(CoreError::Conflict(format!("{number} was generated by a business document; correct it through that document's own workflow")));
    }
    let lines: Vec<(String, i64, i64, String)> = sqlx::query_as("SELECT a.code, l.debit, l.credit, l.memo FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE l.entry_id = ? ORDER BY l.line_no").bind(&eid).fetch_all(&mut *tx).await?;
    let key = format!("reversal:{eid}");
    let m = format!("Reversal of {number}: {}", reason.trim());
    let pl: Vec<Line> = lines
        .iter()
        .map(|(c, d, cr_, mm)| Line {
            account: c,
            debit: *cr_,
            credit: *d,
            memo: mm,
        })
        .collect();
    let id = post(
        &mut tx,
        &PostIn {
            key: &key,
            date: &time::today_dhaka().to_string(),
            memo: &m,
            source_type: "reversal",
            source_id: &eid,
            lines: pl,
            reverses: Some(entry_id),
            actor: Some(p.user_id),
        },
    )
    .await?
    .ok_or_else(|| CoreError::Conflict("already reversed".into()))?;
    audit::record(
        &mut tx,
        audit::AuditEntry {
            actor_id: Some(p.user_id),
            action: "journal.reversed",
            entity_type: "journal_entry",
            entity_id: Some(&eid),
            detail: Some(
                serde_json::json!({"original": number, "memo": memo, "reversal": id.to_string()}),
            ),
            reason: Some(reason.trim()),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// One-time opening balance for a cash/bank/wallet account: Dr account, Cr Opening Balance Equity.
pub async fn opening_balance(
    pool: &SqlitePool,
    p: &Principal,
    account_code: &str,
    amount: i64,
    date: &str,
) -> CoreResult<Id> {
    p.require(perm::ACCOUNTS_MANAGE)?;
    if ![code::CASH, code::BANK, code::WALLET].contains(&account_code) {
        return Err(CoreError::Validation("opening balances here are for cash, bank and wallet accounts; use opening stock / customer / supplier openings for the rest".into()));
    }
    if amount <= 0 {
        return Err(CoreError::Validation(
            "opening balance must be positive".into(),
        ));
    }
    let key = format!("opening:{account_code}");
    let mut tx = pool.begin().await?;
    let id = post(
        &mut tx,
        &PostIn {
            key: &key,
            date,
            memo: &format!("Opening balance {account_code}"),
            source_type: "opening",
            source_id: &key,
            lines: vec![
                dr(account_code, amount, "Opening balance"),
                cr(code::OPENING_EQUITY, amount, "Opening balance"),
            ],
            reverses: None,
            actor: Some(p.user_id),
        },
    )
    .await?
    .ok_or_else(|| {
        CoreError::Conflict(
            "an opening balance for this account already exists; reverse it first if it was wrong"
                .into(),
        )
    })?;
    audit::log(
        &mut tx,
        p,
        "journal.opening_balance",
        "journal_entry",
        &id.to_string(),
        serde_json::json!({"account": account_code, "amount": amount}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

#[derive(Debug, Serialize)]
pub struct Mismatch {
    pub check: String,
    pub detail: String,
    pub expected: i64,
    pub actual: i64,
}

/// Cross-checks the ledgers. Empty = everything agrees.
pub async fn reconcile(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<Mismatch>> {
    p.require(perm::ACCOUNTS_VIEW)?;
    let mut out = vec![];
    let bal = |c: &'static str| async move {
        let r: (String, i64, i64) = sqlx::query_as("SELECT a.normal_side, COALESCE(SUM(l.debit),0), COALESCE(SUM(l.credit),0) FROM accounts a LEFT JOIN journal_lines l ON l.account_id = a.id WHERE a.code = ? GROUP BY a.id").bind(c).fetch_one(pool).await?;
        Ok::<i64, sqlx::Error>(signed(&r.0, r.1, r.2))
    };
    let unbalanced: Vec<(String, i64, i64)> = sqlx::query_as("SELECT e.number, SUM(l.debit), SUM(l.credit) FROM journal_entries e JOIN journal_lines l ON l.entry_id = e.id GROUP BY e.id HAVING SUM(l.debit) <> SUM(l.credit)").fetch_all(pool).await?;
    for (n, d, c) in unbalanced {
        out.push(Mismatch {
            check: "journal_balanced".into(),
            detail: n,
            expected: d,
            actual: c,
        });
    }
    let (td, tc): (i64, i64) =
        sqlx::query_as("SELECT COALESCE(SUM(debit),0), COALESCE(SUM(credit),0) FROM journal_lines")
            .fetch_one(pool)
            .await?;
    if td != tc {
        out.push(Mismatch {
            check: "trial_balance".into(),
            detail: "total debits vs credits".into(),
            expected: td,
            actual: tc,
        });
    }
    let sup: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(amount),0) FROM supplier_ledger_entries")
            .fetch_one(pool)
            .await?;
    let ap = bal(code::AP).await?;
    if sup != ap {
        out.push(Mismatch {
            check: "supplier_ledger_vs_payables".into(),
            detail: "supplier ledger vs account 2000".into(),
            expected: sup,
            actual: ap,
        });
    }
    let cus: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(amount),0) FROM customer_ledger_entries")
            .fetch_one(pool)
            .await?;
    let ar = bal(code::AR).await? + bal(code::INSTALLMENT_AR).await?;
    if cus != ar {
        out.push(Mismatch {
            check: "customer_ledger_vs_receivables".into(),
            detail: "customer ledger vs accounts 1100+1110".into(),
            expected: cus,
            actual: ar,
        });
    }
    let stock: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(value),0) FROM stock_balances")
        .fetch_one(pool)
        .await?;
    let inv = bal(code::INVENTORY).await?;
    if stock != inv {
        out.push(Mismatch {
            check: "stock_value_vs_inventory_account".into(),
            detail: "stock balances vs account 1200".into(),
            expected: stock,
            actual: inv,
        });
    }
    let ordered: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(grand_total + landed_total),0) FROM purchases WHERE status = 'confirmed'").fetch_one(pool).await?;
    let received: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(ri.cost_total),0) FROM purchase_receipt_items ri JOIN purchase_receipts r ON r.id = ri.receipt_id JOIN purchases p ON p.id = r.purchase_id WHERE p.status = 'confirmed'").fetch_one(pool).await?;
    let transit = bal(code::IN_TRANSIT).await?;
    if ordered - received != transit {
        out.push(Mismatch {
            check: "in_transit".into(),
            detail: "ordered - received vs account 1210".into(),
            expected: ordered - received,
            actual: transit,
        });
    }
    let unposted: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM purchases WHERE confirmed_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'purchase:' || purchases.id || ':confirm'))
              + (SELECT COUNT(*) FROM purchase_receipts WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'receipt:' || purchase_receipts.id))
              + (SELECT COUNT(*) FROM supplier_payments WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'supplier_payment:' || supplier_payments.id))
              + (SELECT COUNT(*) FROM sales WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale:' || sales.id))
              + (SELECT COUNT(*) FROM sale_payments WHERE NOT EXISTS (SELECT 1 FROM journal_entries WHERE posting_key = 'sale_payment:' || sale_payments.id))",
    )
    .fetch_one(pool)
    .await?;
    if unposted > 0 {
        out.push(Mismatch {
            check: "unposted_documents".into(),
            detail: "documents without a journal entry".into(),
            expected: 0,
            actual: unposted,
        });
    }
    Ok(out)
}
