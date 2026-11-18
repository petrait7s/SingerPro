//! CSV import of products and customers. Import only ever ADDS records: an SKU or phone number that
//! already exists is reported as an error and nothing is overwritten. The whole file is validated before
//! the first record is written, so a file with mistakes changes nothing.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::catalogue::{self, NewProduct, NewVariant};
use crate::customers::{self, CustomerIn};
use crate::money::Money;
use crate::{CoreError, CoreResult};
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashSet};

const PRODUCT_HEADER: &[&str] = &[
    "brand",
    "category",
    "name",
    "model",
    "sku",
    "barcode",
    "colour",
    "capacity",
    "size",
    "purchase_cost",
    "retail_price",
    "wholesale_price",
    "installment_price",
    "warranty_months",
    "warranty_terms",
    "min_stock",
    "serialized",
];
const CUSTOMER_HEADER: &[&str] = &[
    "name",
    "phone",
    "alt_phone",
    "address",
    "type",
    "credit_limit",
    "notes",
];

pub fn template(kind: &str) -> CoreResult<String> {
    let (header, sample) = match kind {
        "products" => (
            PRODUCT_HEADER,
            "Singer,Refrigerators,Frost-free fridge,SR-250,FRG-250-SIL,,Silver,250L,,40000,52000,48000,,12,Parts and labour,2,yes",
        ),
        "customers" => (
            CUSTOMER_HEADER,
            "Karim Uddin,01711000000,,\"House 5, Road 3, Sylhet\",retail,0,",
        ),
        _ => return Err(CoreError::Validation("import kind must be products or customers".into())),
    };
    Ok(format!("\u{feff}{}\r\n{}\r\n", header.join(","), sample))
}

/// RFC 4180 style parser: quoted fields, doubled quotes, commas and newlines inside quotes.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = vec![];
    let mut row: Vec<String> = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.into_iter()
        .filter(|r| r.iter().any(|f| !f.trim().is_empty()))
        .collect()
}

#[derive(Debug, Serialize)]
pub struct RowError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ImportReport {
    pub kind: String,
    pub rows: usize,
    pub errors: Vec<RowError>,
    /// True only when `commit` was requested, the file had no errors, and everything was written.
    pub imported: bool,
    pub created: usize,
}

fn cols(header: &[String], want: &[&str]) -> Result<Vec<usize>, String> {
    let lower: Vec<String> = header.iter().map(|h| h.trim().to_lowercase()).collect();
    want.iter()
        .map(|w| {
            lower.iter().position(|h| h == w).ok_or_else(|| {
                format!("missing column '{w}'; download the template for the exact headers")
            })
        })
        .collect()
}

fn money(v: &str, what: &str, required: bool) -> Result<Option<i64>, String> {
    let v = v.trim();
    if v.is_empty() {
        return if required {
            Err(format!("{what} is required"))
        } else {
            Ok(None)
        };
    }
    let m = Money::parse(v).map_err(|_| format!("{what}: '{v}' is not an amount"))?;
    if m.paisa() < 0 {
        return Err(format!("{what} cannot be negative"));
    }
    Ok(Some(m.paisa()))
}

fn whole(v: &str, what: &str) -> Result<i64, String> {
    let v = v.trim();
    if v.is_empty() {
        return Ok(0);
    }
    v.parse::<i64>()
        .ok()
        .filter(|n| *n >= 0)
        .ok_or_else(|| format!("{what}: '{v}' must be a whole number, 0 or more"))
}

struct ProductRow {
    line: usize,
    brand: String,
    category: String,
    name: String,
    model: String,
    warranty_months: i64,
    warranty_terms: String,
    min_stock: i64,
    serialized: bool,
    variant: NewVariant,
}

pub async fn import_csv(
    pool: &SqlitePool,
    p: &Principal,
    kind: &str,
    text: &str,
    commit: bool,
) -> CoreResult<ImportReport> {
    match kind {
        "products" => p.require(perm::PRODUCTS_MANAGE)?,
        "customers" => p.require(perm::CUSTOMERS_MANAGE)?,
        _ => {
            return Err(CoreError::Validation(
                "import kind must be products or customers".into(),
            ))
        }
    }
    if text.len() > 5_000_000 {
        return Err(CoreError::Validation(
            "file is too large (limit 5 MB)".into(),
        ));
    }
    let rows = parse_csv(text);
    let mut report = ImportReport {
        kind: kind.into(),
        rows: 0,
        errors: vec![],
        imported: false,
        created: 0,
    };
    let Some((header, body)) = rows.split_first() else {
        return Err(CoreError::Validation("the file is empty".into()));
    };
    report.rows = body.len();
    if body.is_empty() {
        return Err(CoreError::Validation(
            "the file has a header but no rows".into(),
        ));
    }
    if body.len() > 5000 {
        return Err(CoreError::Validation("at most 5,000 rows per file".into()));
    }
    let want = if kind == "products" {
        PRODUCT_HEADER
    } else {
        CUSTOMER_HEADER
    };
    let idx = match cols(header, want) {
        Ok(i) => i,
        Err(m) => {
            report.errors.push(RowError {
                line: 1,
                message: m,
            });
            return Ok(report);
        }
    };
    let get = |r: &Vec<String>, name: &str| -> String {
        let n = want.iter().position(|w| *w == name).expect("known column");
        r.get(idx[n])
            // Line breaks inside a cell become spaces: these are single-line fields.
            .map(|s| s.replace(['\r', '\n'], " ").trim().to_string())
            .unwrap_or_default()
    };
    if kind == "products" {
        let mut parsed: Vec<ProductRow> = vec![];
        let mut seen_sku: HashSet<String> = HashSet::new();
        let mut seen_barcode: HashSet<String> = HashSet::new();
        for (i, r) in body.iter().enumerate() {
            let line = i + 2;
            let mut errs: Vec<String> = vec![];
            let mut need = |name: &str, label: &str| {
                let v = get(r, name);
                if v.is_empty() {
                    errs.push(format!("{label} is required"));
                }
                v
            };
            let (brand, category, name, sku) = (
                need("brand", "brand"),
                need("category", "category"),
                need("name", "name"),
                need("sku", "sku"),
            );
            let sku_key = sku.to_lowercase();
            if !sku.is_empty() && !seen_sku.insert(sku_key.clone()) {
                errs.push(format!("sku '{sku}' appears twice in this file"));
            }
            let barcode = get(r, "barcode");
            if !barcode.is_empty() && !seen_barcode.insert(barcode.to_lowercase()) {
                errs.push(format!("barcode '{barcode}' appears twice in this file"));
            }
            let mut m =
                |col: &str, label: &str, required: bool| match money(&get(r, col), label, required)
                {
                    Ok(v) => v,
                    Err(e) => {
                        errs.push(e);
                        None
                    }
                };
            let cost = m("purchase_cost", "purchase cost", true).unwrap_or(0);
            let retail = m("retail_price", "retail price", true).unwrap_or(0);
            let wholesale = m("wholesale_price", "wholesale price", false).unwrap_or(retail);
            let installment = m("installment_price", "installment price", false);
            let mut w = |col: &str, label: &str| match whole(&get(r, col), label) {
                Ok(v) => v,
                Err(e) => {
                    errs.push(e);
                    0
                }
            };
            let warranty_months = w("warranty_months", "warranty months");
            let min_stock = w("min_stock", "minimum stock");
            let serialized = match get(r, "serialized").to_lowercase().as_str() {
                "" | "no" | "n" | "false" | "0" => false,
                "yes" | "y" | "true" | "1" => true,
                other => {
                    errs.push(format!("serialized: '{other}' must be yes or no"));
                    false
                }
            };
            if !sku.is_empty() {
                let exists: Option<String> =
                    sqlx::query_scalar("SELECT id FROM product_variants WHERE sku = ?")
                        .bind(&sku)
                        .fetch_optional(pool)
                        .await?;
                if exists.is_some() {
                    errs.push(format!(
                        "sku '{sku}' already exists; existing products are never overwritten"
                    ));
                }
            }
            if !barcode.is_empty() {
                let exists: Option<String> =
                    sqlx::query_scalar("SELECT id FROM product_variants WHERE barcode = ?")
                        .bind(&barcode)
                        .fetch_optional(pool)
                        .await?;
                if exists.is_some() {
                    errs.push(format!("barcode '{barcode}' already exists"));
                }
            }
            if !errs.is_empty() {
                report.errors.push(RowError {
                    line,
                    message: errs.join("; "),
                });
                continue;
            }
            parsed.push(ProductRow {
                line,
                brand,
                category,
                name,
                model: get(r, "model"),
                warranty_months,
                warranty_terms: get(r, "warranty_terms"),
                min_stock,
                serialized,
                variant: NewVariant {
                    sku,
                    barcode: if barcode.is_empty() {
                        None
                    } else {
                        Some(barcode)
                    },
                    colour: get(r, "colour"),
                    capacity: get(r, "capacity"),
                    size: get(r, "size"),
                    purchase_cost: cost,
                    retail_price: retail,
                    wholesale_price: wholesale,
                    installment_price: installment,
                },
            });
        }
        // Rows of one product must agree on the product-level fields.
        type Key = (String, String, String, String);
        let mut groups: BTreeMap<Key, Vec<ProductRow>> = BTreeMap::new();
        for r in parsed {
            groups
                .entry((
                    r.brand.to_lowercase(),
                    r.category.to_lowercase(),
                    r.name.to_lowercase(),
                    r.model.to_lowercase(),
                ))
                .or_default()
                .push(r);
        }
        for g in groups.values() {
            let first = &g[0];
            for r in &g[1..] {
                if r.serialized != first.serialized
                    || r.warranty_months != first.warranty_months
                    || r.min_stock != first.min_stock
                {
                    report.errors.push(RowError {
                        line: r.line,
                        message: format!("'{}' has variants with different serialized / warranty / minimum stock values", first.name),
                    });
                }
            }
        }
        report.errors.sort_by_key(|e| e.line);
        if !commit || !report.errors.is_empty() {
            return Ok(report);
        }
        for g in groups.into_values() {
            let first = &g[0];
            let brand_id = match find_named(pool, "brands", &first.brand).await? {
                Some(id) => id,
                None => catalogue::create_brand(pool, p, &first.brand)
                    .await?
                    .to_string(),
            };
            let category_id = match find_named(pool, "categories", &first.category).await? {
                Some(id) => id,
                None => catalogue::create_category(pool, p, &first.category, None)
                    .await?
                    .to_string(),
            };
            let np = NewProduct {
                brand_id,
                category_id,
                name: first.name.clone(),
                model: first.model.clone(),
                description: String::new(),
                warranty_months: first.warranty_months,
                warranty_terms: first.warranty_terms.clone(),
                min_stock: first.min_stock,
                is_serialized: first.serialized,
                variants: g.iter().map(|r| r.variant.clone()).collect(),
            };
            let line = first.line;
            catalogue::create_product(pool, p, np).await.map_err(|e| {
                CoreError::Validation(format!(
                    "line {line}: {e}. {} product(s) were already added before this one.",
                    report.created
                ))
            })?;
            report.created += 1;
        }
        report.imported = true;
        record(pool, p, kind, report.created).await?;
        return Ok(report);
    }

    // customers
    let mut parsed: Vec<CustomerIn> = vec![];
    let mut seen_phone: HashSet<String> = HashSet::new();
    for (i, r) in body.iter().enumerate() {
        let line = i + 2;
        let mut errs = vec![];
        let name = get(r, "name");
        if name.is_empty() {
            errs.push("name is required".to_string());
        }
        let phone = get(r, "phone");
        if !phone.is_empty() {
            if !seen_phone.insert(phone.clone()) {
                errs.push(format!("phone '{phone}' appears twice in this file"));
            }
            let exists: Option<String> =
                sqlx::query_scalar("SELECT id FROM customers WHERE phone = ?")
                    .bind(&phone)
                    .fetch_optional(pool)
                    .await?;
            if exists.is_some() {
                errs.push(format!("phone '{phone}' already belongs to a customer"));
            }
        }
        let ty = match get(r, "type").to_lowercase().as_str() {
            "" => "retail".to_string(),
            t @ ("retail" | "wholesale" | "dealer") => t.to_string(),
            other => {
                errs.push(format!(
                    "type '{other}' must be retail, wholesale or dealer"
                ));
                "retail".into()
            }
        };
        let credit = match money(&get(r, "credit_limit"), "credit limit", false) {
            Ok(v) => v.unwrap_or(0),
            Err(e) => {
                errs.push(e);
                0
            }
        };
        if !errs.is_empty() {
            report.errors.push(RowError {
                line,
                message: errs.join("; "),
            });
            continue;
        }
        parsed.push(CustomerIn {
            name,
            phone,
            alt_phone: get(r, "alt_phone"),
            address: get(r, "address"),
            customer_type: ty,
            credit_limit: credit,
            nid: String::new(),
            notes: get(r, "notes"),
        });
    }
    if !commit || !report.errors.is_empty() {
        return Ok(report);
    }
    for c in parsed {
        customers::create_customer(pool, p, c).await.map_err(|e| {
            CoreError::Validation(format!(
                "{e}. {} customer(s) were already added before this one.",
                report.created
            ))
        })?;
        report.created += 1;
    }
    report.imported = true;
    record(pool, p, kind, report.created).await?;
    Ok(report)
}

async fn record(pool: &SqlitePool, p: &Principal, kind: &str, created: usize) -> CoreResult<()> {
    let mut tx = pool.begin().await?;
    audit::log(
        &mut tx,
        p,
        "import.completed",
        "import",
        kind,
        serde_json::json!({ "created": created }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn find_named(pool: &SqlitePool, table: &str, name: &str) -> CoreResult<Option<String>> {
    // Tables are fixed internal names, never user input.
    let sql = match table {
        "brands" => "SELECT id FROM brands WHERE name = ?",
        _ => "SELECT id FROM categories WHERE name = ? AND parent_id IS NULL",
    };
    Ok(sqlx::query_scalar(sql)
        .bind(name)
        .fetch_optional(pool)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::parse_csv;
    #[test]
    fn parses_quotes_and_embedded_newlines() {
        let rows = parse_csv("a,b\r\nx,\"House 5,\r\nRoad 3, \"\"S\"\"\",z\r\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], vec!["x", "House 5,\r\nRoad 3, \"S\"", "z"]);
    }
}
