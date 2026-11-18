//! Reports and dashboard figures. Every number comes from the same ledgers the rest of the system posts to;
//! nothing is stored separately, so reports can never drift from the books.
//!
//! A report is a plain table (`Report`) so one renderer serves the screen, print, CSV and XLSX.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::time;
use crate::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Clone)]
pub struct Column {
    pub key: &'static str,
    pub label: &'static str,
    /// text | money | int | date
    pub kind: &'static str,
}

#[derive(Debug, Serialize, Clone)]
pub struct Report {
    pub id: String,
    pub title: String,
    pub period: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    pub totals: Option<Vec<Value>>,
}

#[derive(Debug, Serialize)]
pub struct ReportDef {
    pub id: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    /// range | as_of | none
    pub period: &'static str,
    pub financial: bool,
}

const DEFS: &[ReportDef] = &[
    ReportDef {
        id: "sales_by_day",
        title: "Sales by day",
        group: "Sales",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "sales_by_month",
        title: "Sales by month",
        group: "Sales",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "sales_by_mode",
        title: "Sales by mode (retail / wholesale / installment)",
        group: "Sales",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "sales_by_product",
        title: "Sales by product",
        group: "Sales",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "sales_by_brand",
        title: "Sales by brand",
        group: "Sales",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "sales_by_category",
        title: "Sales by category",
        group: "Sales",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "purchases_by_supplier",
        title: "Purchases by supplier",
        group: "Purchasing",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "stock_valuation",
        title: "Stock valuation",
        group: "Inventory",
        period: "none",
        financial: false,
    },
    ReportDef {
        id: "low_stock",
        title: "Low stock",
        group: "Inventory",
        period: "none",
        financial: false,
    },
    ReportDef {
        id: "customer_dues",
        title: "Customer dues",
        group: "Receivables and payables",
        period: "none",
        financial: false,
    },
    ReportDef {
        id: "supplier_dues",
        title: "Supplier dues",
        group: "Receivables and payables",
        period: "none",
        financial: false,
    },
    ReportDef {
        id: "installments_due",
        title: "Installments due or overdue",
        group: "Installments",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "installment_collections",
        title: "Installment collections",
        group: "Installments",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "expenses_by_category",
        title: "Expenses by category",
        group: "Expenses",
        period: "range",
        financial: false,
    },
    ReportDef {
        id: "cashbook",
        title: "Cash and bank book",
        group: "Financial",
        period: "range",
        financial: true,
    },
    ReportDef {
        id: "profit_loss",
        title: "Profit and loss",
        group: "Financial",
        period: "range",
        financial: true,
    },
    ReportDef {
        id: "balance_sheet",
        title: "Balance sheet",
        group: "Financial",
        period: "as_of",
        financial: true,
    },
];

pub fn catalogue(p: &Principal) -> Vec<&'static ReportDef> {
    DEFS.iter()
        .filter(|d| {
            if d.financial {
                p.has(perm::REPORTS_FINANCIAL)
            } else {
                p.has(perm::REPORTS_OPERATIONAL) || p.has(perm::REPORTS_FINANCIAL)
            }
        })
        .collect()
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct Params {
    pub from: Option<String>,
    pub to: Option<String>,
    pub as_of: Option<String>,
}

fn money(c: &'static str, l: &'static str) -> Column {
    Column {
        key: c,
        label: l,
        kind: "money",
    }
}
fn int(c: &'static str, l: &'static str) -> Column {
    Column {
        key: c,
        label: l,
        kind: "int",
    }
}
fn text(c: &'static str, l: &'static str) -> Column {
    Column {
        key: c,
        label: l,
        kind: "text",
    }
}
fn date(c: &'static str, l: &'static str) -> Column {
    Column {
        key: c,
        label: l,
        kind: "date",
    }
}

fn valid_date(s: &str) -> CoreResult<()> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| CoreError::Validation(format!("'{s}' is not a date (use YYYY-MM-DD)")))
}

/// Resolved inclusive period. Defaults to the current month so far.
fn range(p: &Params) -> CoreResult<(String, String)> {
    use chrono::Datelike;
    let today = time::today_dhaka();
    let from = p
        .from
        .clone()
        .unwrap_or_else(|| today.with_day(1).expect("day 1").to_string());
    let to = p.to.clone().unwrap_or_else(|| today.to_string());
    valid_date(&from)?;
    valid_date(&to)?;
    if from > to {
        return Err(CoreError::Validation(
            "the start date is after the end date".into(),
        ));
    }
    Ok((from, to))
}

fn sum_col(rows: &[Vec<Value>], i: usize) -> i64 {
    rows.iter().filter_map(|r| r[i].as_i64()).sum()
}

fn totals_for(columns: &[Column], rows: &[Vec<Value>]) -> Vec<Value> {
    columns
        .iter()
        .enumerate()
        .map(|(i, c)| match c.kind {
            "money" | "int" => json!(sum_col(rows, i)),
            _ => {
                if i == 0 {
                    json!("Total")
                } else {
                    Value::Null
                }
            }
        })
        .collect()
}

fn finish(
    id: &str,
    title: &str,
    period: String,
    columns: Vec<Column>,
    rows: Vec<Vec<Value>>,
    totals: bool,
) -> Report {
    let t = totals.then(|| totals_for(&columns, &rows));
    Report {
        id: id.into(),
        title: title.into(),
        period,
        columns,
        rows,
        totals: t,
    }
}

pub async fn run(pool: &SqlitePool, p: &Principal, id: &str, params: Params) -> CoreResult<Report> {
    let def = DEFS
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| CoreError::NotFound("report".into()))?;
    // Operational reports also open to financial-report holders; financial reports only to them.
    if def.financial || !p.has(perm::REPORTS_OPERATIONAL) {
        p.require(perm::REPORTS_FINANCIAL)?;
    }
    let mut r = match id {
        "sales_by_day" => sales_by_period(pool, &params, false).await?,
        "sales_by_month" => sales_by_period(pool, &params, true).await?,
        "sales_by_mode" => sales_by_mode(pool, &params).await?,
        "sales_by_product" => sales_by_group(pool, &params, "product").await?,
        "sales_by_brand" => sales_by_group(pool, &params, "brand").await?,
        "sales_by_category" => sales_by_group(pool, &params, "category").await?,
        "purchases_by_supplier" => purchases_by_supplier(pool, &params).await?,
        "stock_valuation" => stock_valuation(pool).await?,
        "low_stock" => low_stock(pool).await?,
        "customer_dues" => customer_dues(pool).await?,
        "supplier_dues" => supplier_dues(pool).await?,
        "installments_due" => installments_due(pool, &params).await?,
        "installment_collections" => installment_collections(pool, &params).await?,
        "expenses_by_category" => expenses_by_category(pool, &params).await?,
        "cashbook" => cashbook(pool, &params).await?,
        "profit_loss" => profit_loss(pool, &params).await?,
        "balance_sheet" => balance_sheet(pool, &params).await?,
        _ => return Err(CoreError::NotFound("report".into())),
    };
    r.title = def.title.into();
    // Cost and margin are financial information.
    if !p.has(perm::REPORTS_FINANCIAL) {
        let hide: Vec<usize> = r
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.key == "cost" || c.key == "margin")
            .map(|(i, _)| i)
            .collect();
        for i in hide.into_iter().rev() {
            r.columns.remove(i);
            for row in &mut r.rows {
                row.remove(i);
            }
            if let Some(t) = &mut r.totals {
                t.remove(i);
            }
        }
    }
    Ok(r)
}

/// Average cost per unit, rounded half away from zero (display only; values stay exact in the ledger).
fn avg_cost(value: i64, qty: i64) -> i64 {
    if qty == 0 {
        return 0;
    }
    let (v, q) = (value as i128, qty as i128);
    let n = v.abs() * 2 + q.abs();
    let r = n / (q.abs() * 2);
    (r * if (v < 0) != (q < 0) { -1 } else { 1 }) as i64
}

fn period_text(from: &str, to: &str) -> String {
    format!("{from} to {to}")
}

// ------------------------------------------------------------------ sales

async fn sales_by_period(pool: &SqlitePool, params: &Params, monthly: bool) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let cut = if monthly { 7 } else { 10 };
    let gross: Vec<(String, i64, i64)> = sqlx::query_as(&format!("SELECT substr(sale_date, 1, {cut}), COUNT(*), COALESCE(SUM(grand_total), 0) FROM sales WHERE sale_date BETWEEN ? AND ? GROUP BY 1"))
        .bind(&from).bind(&to).fetch_all(pool).await?;
    let ret: Vec<(String, i64)> = sqlx::query_as(&format!("SELECT substr(r.return_date, 1, {cut}), COALESCE(SUM(ri.value), 0) FROM sale_return_items ri JOIN sale_returns r ON r.id = ri.return_id WHERE r.status = 'completed' AND r.return_date BETWEEN ? AND ? GROUP BY 1"))
        .bind(&from).bind(&to).fetch_all(pool).await?;
    let mut by: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();
    for (d, n, t) in gross {
        let e = by.entry(d).or_default();
        e.0 = n;
        e.1 = t;
    }
    for (d, v) in ret {
        by.entry(d).or_default().2 = v;
    }
    let rows = by
        .into_iter()
        .map(|(d, (n, s, r))| vec![json!(d), json!(n), json!(s), json!(r), json!(s - r)])
        .collect();
    let first = if monthly {
        text("period", "Month")
    } else {
        date("period", "Date")
    };
    Ok(finish(
        if monthly {
            "sales_by_month"
        } else {
            "sales_by_day"
        },
        "",
        period_text(&from, &to),
        vec![
            first,
            int("invoices", "Invoices"),
            money("sales", "Sales (incl. tax)"),
            money("returns", "Returns"),
            money("net", "Net sales"),
        ],
        rows,
        true,
    ))
}

async fn sales_by_mode(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let gross: Vec<(String, i64, i64)> = sqlx::query_as("SELECT mode, COUNT(*), COALESCE(SUM(grand_total), 0) FROM sales WHERE sale_date BETWEEN ? AND ? GROUP BY mode")
        .bind(&from).bind(&to).fetch_all(pool).await?;
    let ret: Vec<(String, i64)> = sqlx::query_as("SELECT s.mode, COALESCE(SUM(ri.value), 0) FROM sale_return_items ri JOIN sale_returns r ON r.id = ri.return_id JOIN sales s ON s.id = r.sale_id WHERE r.status = 'completed' AND r.return_date BETWEEN ? AND ? GROUP BY s.mode")
        .bind(&from).bind(&to).fetch_all(pool).await?;
    let mut by: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();
    for (m, n, t) in gross {
        let e = by.entry(m).or_default();
        e.0 = n;
        e.1 = t;
    }
    for (m, v) in ret {
        by.entry(m).or_default().2 = v;
    }
    let rows = by
        .into_iter()
        .map(|(m, (n, s, r))| vec![json!(m), json!(n), json!(s), json!(r), json!(s - r)])
        .collect();
    Ok(finish(
        "sales_by_mode",
        "",
        period_text(&from, &to),
        vec![
            text("mode", "Mode"),
            int("invoices", "Invoices"),
            money("sales", "Sales (incl. tax)"),
            money("returns", "Returns"),
            money("net", "Net sales"),
        ],
        rows,
        true,
    ))
}

async fn sales_by_group(pool: &SqlitePool, params: &Params, by: &str) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let (label, join, group) = match by {
        "brand" => ("b.name", "", "b.id"),
        "category" => ("c.name", "", "c.id"),
        _ => (
            "TRIM(b.name || ' ' || pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END || CASE WHEN v.capacity <> '' THEN ' · ' || v.capacity ELSE '' END) || ' [' || v.sku || ']'",
            "",
            "v.id",
        ),
    };
    let _ = join;
    // Sold side and returned side are aggregated separately and merged so a return in one period
    // is never silently lost when its sale is in another.
    let sold_sql = format!(
        "SELECT {group}, {label}, COALESCE(SUM(i.qty), 0), COALESCE(SUM(i.line_total - i.tax), 0), COALESCE(SUM(i.cost_total), 0)
         FROM sale_items i JOIN sales s ON s.id = i.sale_id JOIN product_variants v ON v.id = i.variant_id
         JOIN products pr ON pr.id = v.product_id JOIN brands b ON b.id = pr.brand_id JOIN categories c ON c.id = pr.category_id
         WHERE s.sale_date BETWEEN ? AND ? GROUP BY {group}"
    );
    let ret_sql = format!(
        "SELECT {group}, {label}, COALESCE(SUM(ri.qty), 0), COALESCE(SUM(ri.value - ri.tax), 0), COALESCE(SUM(ri.cost_total), 0)
         FROM sale_return_items ri JOIN sale_returns r ON r.id = ri.return_id JOIN product_variants v ON v.id = ri.variant_id
         JOIN products pr ON pr.id = v.product_id JOIN brands b ON b.id = pr.brand_id JOIN categories c ON c.id = pr.category_id
         WHERE r.status = 'completed' AND r.return_date BETWEEN ? AND ? GROUP BY {group}"
    );
    type Agg = (String, String, i64, i64, i64);
    let sold: Vec<Agg> = sqlx::query_as(&sold_sql)
        .bind(&from)
        .bind(&to)
        .fetch_all(pool)
        .await?;
    let ret: Vec<Agg> = sqlx::query_as(&ret_sql)
        .bind(&from)
        .bind(&to)
        .fetch_all(pool)
        .await?;
    let mut m: BTreeMap<String, (String, i64, i64, i64)> = BTreeMap::new();
    for (k, l, q, rev, cost) in sold {
        m.insert(k, (l, q, rev, cost));
    }
    for (k, l, q, rev, cost) in ret {
        let e = m.entry(k).or_insert((l, 0, 0, 0));
        e.1 -= q;
        e.2 -= rev;
        e.3 -= cost;
    }
    let mut rows: Vec<Vec<Value>> = m
        .into_values()
        .map(|(l, q, rev, cost)| {
            vec![
                json!(l),
                json!(q),
                json!(rev),
                json!(cost),
                json!(rev - cost),
            ]
        })
        .collect();
    rows.sort_by(|a, b| {
        b[2].as_i64()
            .cmp(&a[2].as_i64())
            .then(a[0].as_str().cmp(&b[0].as_str()))
    });
    let first = match by {
        "brand" => text("name", "Brand"),
        "category" => text("name", "Category"),
        _ => text("name", "Product"),
    };
    Ok(finish(
        match by {
            "brand" => "sales_by_brand",
            "category" => "sales_by_category",
            _ => "sales_by_product",
        },
        "",
        period_text(&from, &to),
        vec![
            first,
            int("qty", "Net qty"),
            money("revenue", "Net revenue (excl. tax)"),
            money("cost", "Cost of goods"),
            money("margin", "Gross margin"),
        ],
        rows,
        true,
    ))
}

// ------------------------------------------------------------------ purchasing, stock, dues

async fn purchases_by_supplier(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT s.name, COUNT(p.id), COALESCE(SUM(p.grand_total), 0),
                COALESCE((SELECT SUM(amount) FROM supplier_payments sp WHERE sp.supplier_id = s.id AND sp.paid_on BETWEEN ?1 AND ?2), 0)
         FROM suppliers s JOIN purchases p ON p.supplier_id = s.id AND p.status = 'confirmed' AND p.order_date BETWEEN ?1 AND ?2
         GROUP BY s.id ORDER BY 3 DESC, s.name",
    )
    .bind(&from).bind(&to).fetch_all(pool).await?;
    let rows = rows
        .into_iter()
        .map(|(n, c, t, paid)| vec![json!(n), json!(c), json!(t), json!(paid)])
        .collect();
    Ok(finish(
        "purchases_by_supplier",
        "",
        period_text(&from, &to),
        vec![
            text("supplier", "Supplier"),
            int("orders", "Purchase orders"),
            money("purchased", "Purchased"),
            money("paid", "Paid in period"),
        ],
        rows,
        true,
    ))
}

async fn stock_valuation(pool: &SqlitePool) -> CoreResult<Report> {
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT TRIM(b.name || ' ' || pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END), v.sku, sb.bucket, sb.qty, sb.value
         FROM stock_balances sb JOIN product_variants v ON v.id = sb.variant_id JOIN products pr ON pr.id = v.product_id JOIN brands b ON b.id = pr.brand_id
         WHERE sb.qty <> 0 OR sb.value <> 0 ORDER BY 1, sb.bucket",
    ).fetch_all(pool).await?;
    let rows = rows
        .into_iter()
        .map(|(n, sku, bucket, q, v)| {
            vec![
                json!(n),
                json!(sku),
                json!(bucket),
                json!(q),
                json!(avg_cost(v, q)),
                json!(v),
            ]
        })
        .collect();
    Ok(finish(
        "stock_valuation",
        "",
        format!("as of {}", time::today_dhaka()),
        vec![
            text("product", "Product"),
            text("sku", "SKU"),
            text("bucket", "Bucket"),
            int("qty", "Qty"),
            money("avg", "Average cost"),
            money("value", "Value"),
        ],
        rows,
        true,
    ))
}

async fn low_stock(pool: &SqlitePool) -> CoreResult<Report> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT TRIM(b.name || ' ' || pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END), v.sku, pr.min_stock,
                COALESCE((SELECT qty FROM stock_balances WHERE variant_id = v.id AND bucket = 'available'), 0)
         FROM product_variants v JOIN products pr ON pr.id = v.product_id JOIN brands b ON b.id = pr.brand_id
         WHERE v.is_active = 1 AND pr.is_active = 1 AND pr.min_stock > 0
           AND COALESCE((SELECT qty FROM stock_balances WHERE variant_id = v.id AND bucket = 'available'), 0) <= pr.min_stock
         ORDER BY 4 - 3, 1",
    ).fetch_all(pool).await?;
    let rows = rows
        .into_iter()
        .map(|(n, sku, min, have)| {
            vec![
                json!(n),
                json!(sku),
                json!(min),
                json!(have),
                json!((min - have).max(0)),
            ]
        })
        .collect();
    Ok(finish(
        "low_stock",
        "",
        format!("as of {}", time::today_dhaka()),
        vec![
            text("product", "Product"),
            text("sku", "SKU"),
            int("min", "Minimum"),
            int("available", "Available"),
            int("short", "Short by"),
        ],
        rows,
        false,
    ))
}

async fn customer_dues(pool: &SqlitePool) -> CoreResult<Report> {
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT c.code, c.name, c.phone, SUM(l.amount) AS bal FROM customers c JOIN customer_ledger_entries l ON l.customer_id = c.id GROUP BY c.id HAVING bal <> 0 ORDER BY bal DESC, c.name",
    ).fetch_all(pool).await?;
    let rows = rows
        .into_iter()
        .map(|(c, n, ph, b)| vec![json!(c), json!(n), json!(ph), json!(b)])
        .collect();
    Ok(finish(
        "customer_dues",
        "",
        format!("as of {}", time::today_dhaka()),
        vec![
            text("code", "Code"),
            text("name", "Customer"),
            text("phone", "Phone"),
            money("due", "Owes us (negative = credit)"),
        ],
        rows,
        true,
    ))
}

async fn supplier_dues(pool: &SqlitePool) -> CoreResult<Report> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT s.name, s.phone, SUM(l.amount) AS bal FROM suppliers s JOIN supplier_ledger_entries l ON l.supplier_id = s.id GROUP BY s.id HAVING bal <> 0 ORDER BY bal DESC, s.name",
    ).fetch_all(pool).await?;
    let rows = rows
        .into_iter()
        .map(|(n, ph, b)| vec![json!(n), json!(ph), json!(b)])
        .collect();
    Ok(finish(
        "supplier_dues",
        "",
        format!("as of {}", time::today_dhaka()),
        vec![
            text("name", "Supplier"),
            text("phone", "Phone"),
            money("due", "We owe (negative = advance)"),
        ],
        rows,
        true,
    ))
}

// ------------------------------------------------------------------ installments, expenses

async fn installments_due(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let today = time::today_dhaka().to_string();
    let rows: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT a.number, c.name, c.phone, s.due_date, s.amount + COALESCE((SELECT amount FROM installment_late_fees lf WHERE lf.schedule_id = s.id), 0),
                COALESCE((SELECT SUM(al.amount) FROM collection_allocations al WHERE al.schedule_id = s.id), 0)
         FROM installment_schedule s JOIN installment_agreements a ON a.id = s.agreement_id JOIN customers c ON c.id = a.customer_id
         WHERE s.superseded = 0 AND a.status = 'active' AND s.due_date BETWEEN ? AND ? ORDER BY s.due_date, a.number",
    ).bind(&from).bind(&to).fetch_all(pool).await?;
    let rows: Vec<Vec<Value>> = rows
        .into_iter()
        .filter(|r| r.4 - r.5 > 0)
        .map(|(a, c, ph, d, due, paid)| {
            let days = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
                .ok()
                .zip(chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok())
                .map(|(t, d)| (t - d).num_days().max(0))
                .unwrap_or(0);
            vec![
                json!(a),
                json!(c),
                json!(ph),
                json!(d),
                json!(due),
                json!(paid),
                json!(due - paid),
                json!(days),
            ]
        })
        .collect();
    Ok(finish(
        "installments_due",
        "",
        period_text(&from, &to),
        vec![
            text("agreement", "Agreement"),
            text("customer", "Customer"),
            text("phone", "Phone"),
            date("due_date", "Due date"),
            money("due", "Amount due"),
            money("paid", "Paid"),
            money("remaining", "Remaining"),
            int("days", "Days overdue"),
        ],
        rows,
        false,
    ))
    .map(|mut r| {
        // totals only make sense for the money columns; the days column is not additive
        let mut t = totals_for(&r.columns, &r.rows);
        if let Some(last) = t.last_mut() {
            *last = Value::Null;
        }
        r.totals = Some(t);
        r
    })
}

async fn installment_collections(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let rows: Vec<(String, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT col.paid_on, col.receipt_no, a.number, c.name, col.method, col.amount
         FROM installment_collections col JOIN installment_agreements a ON a.id = col.agreement_id JOIN customers c ON c.id = a.customer_id
         WHERE col.kind = 'payment' AND col.paid_on BETWEEN ? AND ? ORDER BY col.paid_on, col.receipt_no",
    ).bind(&from).bind(&to).fetch_all(pool).await?;
    let rows = rows
        .into_iter()
        .map(|(d, r, a, c, m, amt)| {
            vec![json!(d), json!(r), json!(a), json!(c), json!(m), json!(amt)]
        })
        .collect();
    Ok(finish(
        "installment_collections",
        "",
        period_text(&from, &to),
        vec![
            date("date", "Date"),
            text("receipt", "Receipt"),
            text("agreement", "Agreement"),
            text("customer", "Customer"),
            text("method", "Method"),
            money("amount", "Amount (reversals negative)"),
        ],
        rows,
        true,
    ))
}

async fn expenses_by_category(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT c.name, COUNT(e.id), COALESCE(SUM(e.amount), 0) FROM expenses e JOIN expense_categories c ON c.id = e.category_id
         WHERE e.status = 'approved' AND e.expense_date BETWEEN ? AND ? GROUP BY c.id ORDER BY 3 DESC, c.name",
    ).bind(&from).bind(&to).fetch_all(pool).await?;
    let rows = rows
        .into_iter()
        .map(|(n, c, t)| vec![json!(n), json!(c), json!(t)])
        .collect();
    Ok(finish(
        "expenses_by_category",
        "",
        period_text(&from, &to),
        vec![
            text("category", "Category"),
            int("count", "Entries"),
            money("total", "Total (approved, not voided)"),
        ],
        rows,
        true,
    ))
}

// ------------------------------------------------------------------ financial

async fn cashbook(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let opening: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(l.debit - l.credit), 0) FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id JOIN accounts a ON a.id = l.account_id WHERE a.code IN ('1000', '1010', '1020') AND e.entry_date < ?",
    ).bind(&from).fetch_one(pool).await?;
    let lines: Vec<(String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT e.entry_date, e.number, a.name, e.memo, l.debit, l.credit FROM journal_lines l JOIN journal_entries e ON e.id = l.entry_id JOIN accounts a ON a.id = l.account_id
         WHERE a.code IN ('1000', '1010', '1020') AND e.entry_date BETWEEN ? AND ? ORDER BY e.entry_date, e.number, l.line_no",
    ).bind(&from).bind(&to).fetch_all(pool).await?;
    let mut bal = opening;
    let mut rows = vec![vec![
        json!(from),
        json!(""),
        json!("Opening balance"),
        json!(""),
        json!(0),
        json!(0),
        json!(opening),
    ]];
    for (d, n, acct, memo, dr, cr) in lines {
        bal += dr - cr;
        rows.push(vec![
            json!(d),
            json!(n),
            json!(acct),
            json!(memo),
            json!(dr),
            json!(cr),
            json!(bal),
        ]);
    }
    let td = sum_col(&rows, 4);
    let tc = sum_col(&rows, 5);
    let mut r = finish(
        "cashbook",
        "",
        period_text(&from, &to),
        vec![
            date("date", "Date"),
            text("entry", "Entry"),
            text("account", "Account"),
            text("memo", "Description"),
            money("in", "Received"),
            money("out", "Paid"),
            money("balance", "Balance"),
        ],
        rows,
        false,
    );
    r.totals = Some(vec![
        json!("Total"),
        Value::Null,
        Value::Null,
        Value::Null,
        json!(td),
        json!(tc),
        json!(bal),
    ]);
    Ok(r)
}

async fn profit_loss(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let (from, to) = range(params)?;
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT a.code, a.name, a.type, COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0)
         FROM accounts a JOIN journal_lines l ON l.account_id = a.id JOIN journal_entries e ON e.id = l.entry_id
         WHERE a.type IN ('income', 'expense') AND e.entry_date BETWEEN ? AND ? GROUP BY a.id ORDER BY a.code",
    ).bind(&from).bind(&to).fetch_all(pool).await?;
    let mut out: Vec<Vec<Value>> = vec![];
    let mut income = 0i64;
    let mut cogs = 0i64;
    let mut expense = 0i64;
    out.push(vec![json!(""), json!("INCOME"), json!(null)]);
    for r in rows.iter().filter(|r| r.2 == "income") {
        let v = r.4 - r.3;
        income += v;
        out.push(vec![json!(r.0), json!(r.1), json!(v)]);
    }
    out.push(vec![json!(""), json!("Total income"), json!(income)]);
    out.push(vec![json!(""), json!("COST OF GOODS"), json!(null)]);
    for r in rows
        .iter()
        .filter(|r| r.2 == "expense" && r.0.starts_with('5'))
    {
        let v = r.3 - r.4;
        cogs += v;
        out.push(vec![json!(r.0), json!(r.1), json!(v)]);
    }
    out.push(vec![json!(""), json!("Total cost of goods"), json!(cogs)]);
    out.push(vec![json!(""), json!("GROSS PROFIT"), json!(income - cogs)]);
    out.push(vec![json!(""), json!("OPERATING EXPENSES"), json!(null)]);
    for r in rows
        .iter()
        .filter(|r| r.2 == "expense" && !r.0.starts_with('5'))
    {
        let v = r.3 - r.4;
        expense += v;
        out.push(vec![json!(r.0), json!(r.1), json!(v)]);
    }
    out.push(vec![
        json!(""),
        json!("Total operating expenses"),
        json!(expense),
    ]);
    out.push(vec![
        json!(""),
        json!("NET PROFIT"),
        json!(income - cogs - expense),
    ]);
    Ok(finish(
        "profit_loss",
        "",
        period_text(&from, &to),
        vec![
            text("code", "Account"),
            text("name", "Description"),
            money("amount", "Amount"),
        ],
        out,
        false,
    ))
}

async fn balance_sheet(pool: &SqlitePool, params: &Params) -> CoreResult<Report> {
    let as_of = params
        .as_of
        .clone()
        .unwrap_or_else(|| time::today_dhaka().to_string());
    valid_date(&as_of)?;
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT a.code, a.name, a.type, COALESCE(SUM(l.debit), 0), COALESCE(SUM(l.credit), 0)
         FROM accounts a JOIN journal_lines l ON l.account_id = a.id JOIN journal_entries e ON e.id = l.entry_id
         WHERE e.entry_date <= ? GROUP BY a.id ORDER BY a.code",
    ).bind(&as_of).fetch_all(pool).await?;
    let mut out: Vec<Vec<Value>> = vec![];
    let section = |out: &mut Vec<Vec<Value>>, title: &str, ty: &str, debit_normal: bool| -> i64 {
        out.push(vec![json!(""), json!(title), json!(null)]);
        let mut total = 0;
        for r in rows.iter().filter(|r| r.2 == ty && r.3 != r.4) {
            let v = if debit_normal { r.3 - r.4 } else { r.4 - r.3 };
            total += v;
            out.push(vec![json!(r.0), json!(r.1), json!(v)]);
        }
        total
    };
    let assets = section(&mut out, "ASSETS", "asset", true);
    out.push(vec![json!(""), json!("Total assets"), json!(assets)]);
    let liabilities = section(&mut out, "LIABILITIES", "liability", false);
    out.push(vec![
        json!(""),
        json!("Total liabilities"),
        json!(liabilities),
    ]);
    let equity = section(&mut out, "EQUITY", "equity", false);
    let earnings: i64 = rows
        .iter()
        .map(|r| match r.2.as_str() {
            "income" => r.4 - r.3,
            "expense" => -(r.3 - r.4),
            _ => 0,
        })
        .sum();
    out.push(vec![
        json!(""),
        json!("Profit retained (all periods to date)"),
        json!(earnings),
    ]);
    out.push(vec![
        json!(""),
        json!("Total equity"),
        json!(equity + earnings),
    ]);
    out.push(vec![
        json!(""),
        json!("Liabilities + equity"),
        json!(liabilities + equity + earnings),
    ]);
    out.push(vec![
        json!(""),
        json!(if assets == liabilities + equity + earnings {
            "Balanced"
        } else {
            "OUT OF BALANCE - run reconciliation"
        }),
        json!(assets - liabilities - equity - earnings),
    ]);
    Ok(finish(
        "balance_sheet",
        "",
        format!("as of {as_of}"),
        vec![
            text("code", "Account"),
            text("name", "Description"),
            money("amount", "Amount"),
        ],
        out,
        false,
    ))
}

// ------------------------------------------------------------------ export

/// Plain decimal taka with exactly two places, no grouping: 125000.50, -0.05.
pub fn plain_taka(paisa: i64) -> String {
    let neg = paisa < 0;
    let a = paisa.unsigned_abs();
    format!("{}{}.{:02}", if neg { "-" } else { "" }, a / 100, a % 100)
}

fn cell_text(kind: &str, v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Number(n) if kind == "money" => plain_taka(n.as_i64().unwrap_or(0)),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn csv_field(s: &str) -> String {
    // Defuse spreadsheet formulas (CSV injection) before quoting.
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) && s.parse::<f64>().is_err() {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

pub fn to_csv(r: &Report) -> String {
    let mut out = String::from("\u{feff}"); // BOM so Excel reads Bangla text as UTF-8
    out.push_str(&format!(
        "{},{}\r\n",
        csv_field(&r.title),
        csv_field(&r.period)
    ));
    out.push_str(
        &r.columns
            .iter()
            .map(|c| csv_field(c.label))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push_str("\r\n");
    for row in r.rows.iter().chain(r.totals.iter()) {
        let line: Vec<String> = row
            .iter()
            .zip(&r.columns)
            .map(|(v, c)| csv_field(&cell_text(c.kind, v)))
            .collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

pub fn to_xlsx(r: &Report) -> CoreResult<Vec<u8>> {
    use rust_xlsxwriter::{Format, Workbook};
    let x = |e: rust_xlsxwriter::XlsxError| {
        CoreError::Validation(format!("could not build the spreadsheet: {e}"))
    };
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    let name: String = r
        .title
        .chars()
        .filter(|c| !"[]:*?/\\".contains(*c))
        .take(31)
        .collect();
    ws.set_name(if name.is_empty() { "Report" } else { &name })
        .map_err(x)?;
    let bold = Format::new().set_bold();
    let money_fmt = Format::new().set_num_format("#,##0.00");
    ws.write_with_format(0, 0, &r.title, &bold).map_err(x)?;
    ws.write(0, 1, &r.period).map_err(x)?;
    for (i, c) in r.columns.iter().enumerate() {
        ws.write_with_format(2, i as u16, c.label, &bold)
            .map_err(x)?;
        ws.set_column_width(i as u16, if c.kind == "text" { 34.0 } else { 16.0 })
            .map_err(x)?;
    }
    for (ri, row) in r.rows.iter().chain(r.totals.iter()).enumerate() {
        let is_total = ri >= r.rows.len();
        for (ci, (v, c)) in row.iter().zip(&r.columns).enumerate() {
            let (rr, cc) = (3 + ri as u32, ci as u16);
            match (v, c.kind) {
                (Value::Null, _) => {}
                // Money is written from exact paisa; the cell holds a number, not text.
                (Value::Number(n), "money") => {
                    let p = n.as_i64().unwrap_or(0);
                    let f = (p as f64) / 100.0;
                    if is_total {
                        ws.write_with_format(
                            rr,
                            cc,
                            f,
                            &Format::new().set_bold().set_num_format("#,##0.00"),
                        )
                        .map_err(x)?;
                    } else {
                        ws.write_with_format(rr, cc, f, &money_fmt).map_err(x)?;
                    }
                }
                (Value::Number(n), _) => {
                    ws.write(rr, cc, n.as_i64().unwrap_or(0) as f64)
                        .map_err(x)?;
                }
                (other, _) => {
                    let s = cell_text(c.kind, other);
                    if is_total {
                        ws.write_with_format(rr, cc, &s, &bold).map_err(x)?;
                    } else {
                        ws.write_string(rr, cc, &s).map_err(x)?;
                    }
                }
            }
        }
    }
    wb.save_to_buffer().map_err(x)
}

/// Audited export. Callers write the returned bytes where the user chose.
pub async fn export(
    pool: &SqlitePool,
    p: &Principal,
    id: &str,
    params: Params,
    format: &str,
) -> CoreResult<(String, Vec<u8>)> {
    let report = run(pool, p, id, params).await?;
    let (ext, bytes) = match format {
        "csv" => ("csv", to_csv(&report).into_bytes()),
        "xlsx" => ("xlsx", to_xlsx(&report)?),
        _ => return Err(CoreError::Validation("format must be csv or xlsx".into())),
    };
    let mut tx = pool.begin().await?;
    audit::log(
        &mut tx,
        p,
        "report.exported",
        "report",
        id,
        json!({"format": ext, "period": report.period, "rows": report.rows.len()}),
    )
    .await?;
    tx.commit().await?;
    let safe_period: String = report
        .period
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Ok((format!("{id}_{safe_period}.{ext}"), bytes))
}

// ------------------------------------------------------------------ dashboard

#[derive(Debug, Serialize, Default)]
pub struct Dashboard {
    pub today: String,
    pub sales: Option<SalesKpi>,
    pub money: Option<MoneyKpi>,
    pub stock: Option<StockKpi>,
    pub installments: Option<InstKpi>,
    pub pending: Pending,
    /// (date, net sales) for the last 14 days, oldest first.
    pub last_days: Vec<(String, i64)>,
}

#[derive(Debug, Serialize)]
pub struct SalesKpi {
    pub today_net: i64,
    pub today_invoices: i64,
    pub month_net: i64,
    pub month_invoices: i64,
}

#[derive(Debug, Serialize)]
pub struct MoneyKpi {
    pub cash_and_bank: i64,
    pub receivables: i64,
    pub payables: i64,
    pub month_profit: i64,
}

#[derive(Debug, Serialize)]
pub struct StockKpi {
    pub value: i64,
    pub low_items: i64,
}

#[derive(Debug, Serialize)]
pub struct InstKpi {
    pub overdue_count: i64,
    pub overdue_amount: i64,
    pub due_this_week: i64,
}

#[derive(Debug, Serialize, Default)]
pub struct Pending {
    pub returns_awaiting_inspection: Option<i64>,
    pub expenses_awaiting_approval: Option<i64>,
    pub open_warranty_cases: Option<i64>,
}

pub async fn dashboard(pool: &SqlitePool, p: &Principal) -> CoreResult<Dashboard> {
    use chrono::Datelike;
    let today = time::today_dhaka();
    let month_start = today.with_day(1).expect("day 1").to_string();
    let today_s = today.to_string();
    let mut d = Dashboard {
        today: today_s.clone(),
        ..Default::default()
    };
    let op = p.has(perm::REPORTS_OPERATIONAL) || p.has(perm::REPORTS_FINANCIAL);
    let fin = p.has(perm::REPORTS_FINANCIAL) || p.has(perm::ACCOUNTS_VIEW);
    let net = |from: String, to: String| async move {
        let s: (i64, i64) = sqlx::query_as("SELECT COUNT(*), COALESCE(SUM(grand_total), 0) FROM sales WHERE sale_date BETWEEN ? AND ?").bind(&from).bind(&to).fetch_one(pool).await?;
        let r: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(ri.value), 0) FROM sale_return_items ri JOIN sale_returns r ON r.id = ri.return_id WHERE r.status = 'completed' AND r.return_date BETWEEN ? AND ?").bind(&from).bind(&to).fetch_one(pool).await?;
        Ok::<_, CoreError>((s.0, s.1 - r))
    };
    if op || p.has(perm::SALES_VIEW) {
        let (ti, tn) = net(today_s.clone(), today_s.clone()).await?;
        let (mi, mn) = net(month_start.clone(), today_s.clone()).await?;
        d.sales = Some(SalesKpi {
            today_net: tn,
            today_invoices: ti,
            month_net: mn,
            month_invoices: mi,
        });
        let from = (today - chrono::Duration::days(13)).to_string();
        let rep = sales_by_period(
            pool,
            &Params {
                from: Some(from.clone()),
                to: Some(today_s.clone()),
                as_of: None,
            },
            false,
        )
        .await?;
        let mut by: BTreeMap<String, i64> = BTreeMap::new();
        for i in 0..14 {
            by.insert((today - chrono::Duration::days(13 - i)).to_string(), 0);
        }
        for row in rep.rows {
            if let (Some(k), Some(v)) = (row[0].as_str(), row[4].as_i64()) {
                by.insert(k.to_string(), v);
            }
        }
        d.last_days = by.into_iter().collect();
    }
    if fin {
        let bal = |codes: &'static str, debit: bool| async move {
            let v: i64 = sqlx::query_scalar(&format!("SELECT COALESCE(SUM({}), 0) FROM journal_lines l JOIN accounts a ON a.id = l.account_id WHERE a.code IN ({codes})", if debit { "l.debit - l.credit" } else { "l.credit - l.debit" })).fetch_one(pool).await?;
            Ok::<_, CoreError>(v)
        };
        let pl = profit_loss(
            pool,
            &Params {
                from: Some(month_start.clone()),
                to: Some(today_s.clone()),
                as_of: None,
            },
        )
        .await?;
        let profit = pl
            .rows
            .iter()
            .find(|r| r[1] == "NET PROFIT")
            .and_then(|r| r[2].as_i64())
            .unwrap_or(0);
        d.money = Some(MoneyKpi {
            cash_and_bank: bal("'1000','1010','1020'", true).await?,
            receivables: bal("'1100','1110'", true).await?,
            payables: bal("'2000'", false).await?,
            month_profit: profit,
        });
    }
    if op || p.has(perm::INVENTORY_VIEW) {
        let value: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(value), 0) FROM stock_balances")
            .fetch_one(pool)
            .await?;
        let low = low_stock(pool).await?.rows.len() as i64;
        d.stock = Some(StockKpi {
            value,
            low_items: low,
        });
    }
    if op || p.has(perm::INSTALLMENTS_VIEW) {
        let rows: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(DISTINCT s.agreement_id), COALESCE(SUM(s.amount + COALESCE((SELECT amount FROM installment_late_fees lf WHERE lf.schedule_id = s.id), 0) - COALESCE((SELECT SUM(al.amount) FROM collection_allocations al WHERE al.schedule_id = s.id), 0)), 0)
             FROM installment_schedule s JOIN installment_agreements a ON a.id = s.agreement_id
             WHERE s.superseded = 0 AND a.status = 'active' AND s.due_date < ?1
               AND s.amount + COALESCE((SELECT amount FROM installment_late_fees lf WHERE lf.schedule_id = s.id), 0) > COALESCE((SELECT SUM(al.amount) FROM collection_allocations al WHERE al.schedule_id = s.id), 0)",
        ).bind(&today_s).fetch_one(pool).await?;
        let week_end = (today + chrono::Duration::days(7)).to_string();
        let upcoming: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(s.amount - COALESCE((SELECT SUM(al.amount) FROM collection_allocations al WHERE al.schedule_id = s.id), 0)), 0)
             FROM installment_schedule s JOIN installment_agreements a ON a.id = s.agreement_id
             WHERE s.superseded = 0 AND a.status = 'active' AND s.due_date BETWEEN ?1 AND ?2",
        ).bind(&today_s).bind(&week_end).fetch_one(pool).await?;
        d.installments = Some(InstKpi {
            overdue_count: rows.0,
            overdue_amount: rows.1,
            due_this_week: upcoming.max(0),
        });
    }
    if p.has(perm::RETURNS_APPROVE) {
        d.pending.returns_awaiting_inspection = Some(
            sqlx::query_scalar("SELECT COUNT(*) FROM sale_returns WHERE status = 'requested'")
                .fetch_one(pool)
                .await?,
        );
    }
    if p.has(perm::EXPENSES_APPROVE) {
        d.pending.expenses_awaiting_approval = Some(
            sqlx::query_scalar("SELECT COUNT(*) FROM expenses WHERE status = 'pending'")
                .fetch_one(pool)
                .await?,
        );
    }
    if p.has(perm::WARRANTY_MANAGE) {
        d.pending.open_warranty_cases = Some(
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM warranty_cases WHERE status IN ('open', 'in_service')",
            )
            .fetch_one(pool)
            .await?,
        );
    }
    Ok(d)
}
