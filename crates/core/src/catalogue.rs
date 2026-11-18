//! Brands, categories, products and variants. Prices are integer paisa; price changes are audited.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::{time, CoreError, CoreResult, Id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{SqliteConnection, SqlitePool};

pub(crate) fn map_unique(e: sqlx::Error, what: &str) -> CoreError {
    match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            CoreError::Conflict(format!("{what} already exists"))
        }
        sqlx::Error::Database(d) if d.is_foreign_key_violation() => {
            CoreError::Validation(format!("{what}: referenced record does not exist"))
        }
        other => CoreError::Database(other),
    }
}

fn text(label: &str, v: &str, max: usize, required: bool) -> CoreResult<String> {
    let t = v.trim();
    if required && t.is_empty() {
        return Err(CoreError::Validation(format!("{label} is required")));
    }
    if t.chars().count() > max || t.chars().any(|c| c.is_control()) {
        return Err(CoreError::Validation(format!(
            "{label}: at most {max} characters, no control characters"
        )));
    }
    Ok(t.to_string())
}

fn price(label: &str, v: i64) -> CoreResult<i64> {
    if v < 0 {
        return Err(CoreError::Validation(format!("{label} cannot be negative")));
    }
    Ok(v)
}

// ------------------------------------------------------------------ brands & categories

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Named {
    pub id: String,
    pub name: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub is_active: bool,
}

pub async fn list_brands(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<Named>> {
    p.require(perm::PRODUCTS_VIEW)?;
    Ok(
        sqlx::query_as("SELECT id, name, is_active FROM brands ORDER BY name")
            .fetch_all(pool)
            .await?,
    )
}

pub async fn create_brand(pool: &SqlitePool, p: &Principal, name: &str) -> CoreResult<Id> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let name = text("brand name", name, 60, true)?;
    let id = Id::new();
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO brands (id, name, created_at) VALUES (?, ?, ?)")
        .bind(id.to_string())
        .bind(&name)
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, "brand"))?;
    audit::log(
        &mut tx,
        p,
        "brand.created",
        "brand",
        &id.to_string(),
        serde_json::json!({"name": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn rename_brand(pool: &SqlitePool, p: &Principal, id: Id, name: &str) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let name = text("brand name", name, 60, true)?;
    let mut tx = pool.begin().await?;
    let old: Option<String> = sqlx::query_scalar("SELECT name FROM brands WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
    let old = old.ok_or_else(|| CoreError::NotFound("brand".into()))?;
    sqlx::query("UPDATE brands SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, "brand"))?;
    audit::log(
        &mut tx,
        p,
        "brand.renamed",
        "brand",
        &id.to_string(),
        serde_json::json!({"before": old, "after": name}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_brand_active(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE brands SET is_active = ? WHERE id = ?")
        .bind(active as i64)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("brand".into()));
    }
    audit::log(
        &mut tx,
        p,
        if active {
            "brand.activated"
        } else {
            "brand.deactivated"
        },
        "brand",
        &id.to_string(),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn list_categories(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<CategoryRow>> {
    p.require(perm::PRODUCTS_VIEW)?;
    Ok(sqlx::query_as("SELECT id, name, parent_id, is_active FROM categories ORDER BY COALESCE(parent_id, id), parent_id IS NOT NULL, name")
        .fetch_all(pool)
        .await?)
}

/// Two levels only: category and subcategory.
pub async fn create_category(
    pool: &SqlitePool,
    p: &Principal,
    name: &str,
    parent_id: Option<Id>,
) -> CoreResult<Id> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let name = text("category name", name, 60, true)?;
    let id = Id::new();
    let mut tx = pool.begin().await?;
    if let Some(pid) = parent_id {
        let pp: Option<Option<String>> =
            sqlx::query_scalar("SELECT parent_id FROM categories WHERE id = ?")
                .bind(pid.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        match pp {
            None => return Err(CoreError::NotFound("parent category".into())),
            Some(Some(_)) => {
                return Err(CoreError::Validation(
                    "subcategories cannot have subcategories".into(),
                ))
            }
            Some(None) => {}
        }
    }
    sqlx::query("INSERT INTO categories (id, name, parent_id, created_at) VALUES (?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(&name)
        .bind(parent_id.map(|x| x.to_string()))
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| map_unique(e, "category"))?;
    audit::log(
        &mut tx,
        p,
        "category.created",
        "category",
        &id.to_string(),
        serde_json::json!({"name": name, "parent_id": parent_id.map(|x| x.to_string())}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn set_category_active(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE categories SET is_active = ? WHERE id = ?")
        .bind(active as i64)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("category".into()));
    }
    audit::log(
        &mut tx,
        p,
        if active {
            "category.activated"
        } else {
            "category.deactivated"
        },
        "category",
        &id.to_string(),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ------------------------------------------------------------------ products & variants

#[derive(Debug, Clone, Deserialize)]
pub struct NewVariant {
    pub sku: String,
    pub barcode: Option<String>,
    pub colour: String,
    pub capacity: String,
    pub size: String,
    pub purchase_cost: i64,
    pub retail_price: i64,
    pub wholesale_price: i64,
    pub installment_price: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewProduct {
    pub brand_id: String,
    pub category_id: String,
    pub name: String,
    pub model: String,
    pub description: String,
    pub warranty_months: i64,
    pub warranty_terms: String,
    pub min_stock: i64,
    pub is_serialized: bool,
    pub variants: Vec<NewVariant>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateProduct {
    pub id: String,
    pub brand_id: String,
    pub category_id: String,
    pub name: String,
    pub model: String,
    pub description: String,
    pub warranty_months: i64,
    pub warranty_terms: String,
    pub min_stock: i64,
    pub is_serialized: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateVariant {
    pub id: String,
    pub sku: String,
    pub barcode: Option<String>,
    pub colour: String,
    pub capacity: String,
    pub size: String,
    pub purchase_cost: i64,
    pub retail_price: i64,
    pub wholesale_price: i64,
    pub installment_price: Option<i64>,
}

struct CleanVariant {
    sku: String,
    barcode: Option<String>,
    colour: String,
    capacity: String,
    size: String,
    purchase_cost: i64,
    retail_price: i64,
    wholesale_price: i64,
    installment_price: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
fn clean_variant(
    sku: &str,
    barcode: &Option<String>,
    colour: &str,
    capacity: &str,
    size: &str,
    pc: i64,
    rp: i64,
    wp: i64,
    ip: Option<i64>,
) -> CoreResult<CleanVariant> {
    let barcode = barcode
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(|b| text("barcode", b, 64, true))
        .transpose()?;
    Ok(CleanVariant {
        sku: text("SKU", sku, 40, true)?,
        barcode,
        colour: text("colour", colour, 40, false)?,
        capacity: text("capacity", capacity, 40, false)?,
        size: text("size", size, 40, false)?,
        purchase_cost: price("purchase cost", pc)?,
        retail_price: price("retail price", rp)?,
        wholesale_price: price("wholesale price", wp)?,
        installment_price: ip.map(|v| price("installment price", v)).transpose()?,
    })
}

async fn insert_variant(
    conn: &mut SqliteConnection,
    product_id: &str,
    v: &CleanVariant,
) -> CoreResult<Id> {
    let id = Id::new();
    let now = time::now_utc_string();
    sqlx::query(
        "INSERT INTO product_variants (id, product_id, sku, barcode, colour, capacity, size, purchase_cost, retail_price, wholesale_price, installment_price, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(product_id)
    .bind(&v.sku)
    .bind(&v.barcode)
    .bind(&v.colour)
    .bind(&v.capacity)
    .bind(&v.size)
    .bind(v.purchase_cost)
    .bind(v.retail_price)
    .bind(v.wholesale_price)
    .bind(v.installment_price)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await
    .map_err(|e| map_unique(e, &format!("SKU or barcode ({})", v.sku)))?;
    Ok(id)
}

struct CleanProduct {
    name: String,
    model: String,
    description: String,
    warranty_terms: String,
}

fn clean_product(
    name: &str,
    model: &str,
    description: &str,
    terms: &str,
    warranty_months: i64,
    min_stock: i64,
) -> CoreResult<CleanProduct> {
    if !(0..=600).contains(&warranty_months) {
        return Err(CoreError::Validation(
            "warranty months must be 0-600".into(),
        ));
    }
    if min_stock < 0 {
        return Err(CoreError::Validation(
            "minimum stock cannot be negative".into(),
        ));
    }
    Ok(CleanProduct {
        name: text("product name", name, 120, true)?,
        model: text("model", model, 60, false)?,
        description: text("description", description, 1000, false)?,
        warranty_terms: text("warranty terms", terms, 500, false)?,
    })
}

pub async fn create_product(pool: &SqlitePool, p: &Principal, np: NewProduct) -> CoreResult<Id> {
    p.require(perm::PRODUCTS_MANAGE)?;
    if np.variants.is_empty() {
        return Err(CoreError::Validation(
            "a product needs at least one variant (SKU)".into(),
        ));
    }
    let cp = clean_product(
        &np.name,
        &np.model,
        &np.description,
        &np.warranty_terms,
        np.warranty_months,
        np.min_stock,
    )?;
    let variants: Vec<CleanVariant> = np
        .variants
        .iter()
        .map(|v| {
            clean_variant(
                &v.sku,
                &v.barcode,
                &v.colour,
                &v.capacity,
                &v.size,
                v.purchase_cost,
                v.retail_price,
                v.wholesale_price,
                v.installment_price,
            )
        })
        .collect::<CoreResult<_>>()?;
    let id = Id::new();
    let now = time::now_utc_string();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO products (id, brand_id, category_id, name, model, description, warranty_months, warranty_terms, min_stock, is_serialized, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&np.brand_id)
    .bind(&np.category_id)
    .bind(&cp.name)
    .bind(&cp.model)
    .bind(&cp.description)
    .bind(np.warranty_months)
    .bind(&cp.warranty_terms)
    .bind(np.min_stock)
    .bind(np.is_serialized as i64)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(|e| map_unique(e, "brand/category"))?;
    let mut vids = vec![];
    for v in &variants {
        vids.push(
            insert_variant(&mut tx, &id.to_string(), v)
                .await?
                .to_string(),
        );
    }
    audit::log(
        &mut tx,
        p,
        "product.created",
        "product",
        &id.to_string(),
        serde_json::json!({"name": cp.name, "serialized": np.is_serialized, "variants": vids}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

async fn has_movements(conn: &mut SqliteConnection, product_id: &str) -> CoreResult<bool> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stock_movements m JOIN product_variants v ON v.id = m.variant_id WHERE v.product_id = ?",
    )
    .bind(product_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(n > 0)
}

pub async fn update_product(pool: &SqlitePool, p: &Principal, up: UpdateProduct) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let cp = clean_product(
        &up.name,
        &up.model,
        &up.description,
        &up.warranty_terms,
        up.warranty_months,
        up.min_stock,
    )?;
    let mut tx = pool.begin().await?;
    let cur: Option<(String, i64)> =
        sqlx::query_as("SELECT name, is_serialized FROM products WHERE id = ?")
            .bind(&up.id)
            .fetch_optional(&mut *tx)
            .await?;
    let (old_name, old_ser) = cur.ok_or_else(|| CoreError::NotFound("product".into()))?;
    if (old_ser == 1) != up.is_serialized && has_movements(&mut tx, &up.id).await? {
        return Err(CoreError::Conflict(
            "serial tracking cannot be changed once the product has stock history".into(),
        ));
    }
    sqlx::query(
        "UPDATE products SET brand_id=?, category_id=?, name=?, model=?, description=?, warranty_months=?, warranty_terms=?, min_stock=?, is_serialized=?, updated_at=? WHERE id=?",
    )
    .bind(&up.brand_id)
    .bind(&up.category_id)
    .bind(&cp.name)
    .bind(&cp.model)
    .bind(&cp.description)
    .bind(up.warranty_months)
    .bind(&cp.warranty_terms)
    .bind(up.min_stock)
    .bind(up.is_serialized as i64)
    .bind(time::now_utc_string())
    .bind(&up.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| map_unique(e, "brand/category"))?;
    audit::log(
        &mut tx,
        p,
        "product.updated",
        "product",
        &up.id,
        serde_json::json!({"before_name": old_name, "after_name": cp.name}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_product_active(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE products SET is_active = ?, updated_at = ? WHERE id = ?")
        .bind(active as i64)
        .bind(time::now_utc_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("product".into()));
    }
    audit::log(
        &mut tx,
        p,
        if active {
            "product.activated"
        } else {
            "product.deactivated"
        },
        "product",
        &id.to_string(),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn add_variant(
    pool: &SqlitePool,
    p: &Principal,
    product_id: Id,
    v: NewVariant,
) -> CoreResult<Id> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let cv = clean_variant(
        &v.sku,
        &v.barcode,
        &v.colour,
        &v.capacity,
        &v.size,
        v.purchase_cost,
        v.retail_price,
        v.wholesale_price,
        v.installment_price,
    )?;
    let mut tx = pool.begin().await?;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE id = ?")
        .bind(product_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if exists == 0 {
        return Err(CoreError::NotFound("product".into()));
    }
    let id = insert_variant(&mut tx, &product_id.to_string(), &cv).await?;
    audit::log(
        &mut tx,
        p,
        "variant.created",
        "variant",
        &id.to_string(),
        serde_json::json!({"sku": cv.sku, "product_id": product_id.to_string()}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update_variant(pool: &SqlitePool, p: &Principal, uv: UpdateVariant) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let cv = clean_variant(
        &uv.sku,
        &uv.barcode,
        &uv.colour,
        &uv.capacity,
        &uv.size,
        uv.purchase_cost,
        uv.retail_price,
        uv.wholesale_price,
        uv.installment_price,
    )?;
    let mut tx = pool.begin().await?;
    let old: Option<(String, i64, i64, i64, Option<i64>)> =
        sqlx::query_as("SELECT sku, purchase_cost, retail_price, wholesale_price, installment_price FROM product_variants WHERE id = ?")
            .bind(&uv.id)
            .fetch_optional(&mut *tx)
            .await?;
    let (osku, opc, orp, owp, oip) = old.ok_or_else(|| CoreError::NotFound("variant".into()))?;
    sqlx::query(
        "UPDATE product_variants SET sku=?, barcode=?, colour=?, capacity=?, size=?, purchase_cost=?, retail_price=?, wholesale_price=?, installment_price=?, updated_at=? WHERE id=?",
    )
    .bind(&cv.sku)
    .bind(&cv.barcode)
    .bind(&cv.colour)
    .bind(&cv.capacity)
    .bind(&cv.size)
    .bind(cv.purchase_cost)
    .bind(cv.retail_price)
    .bind(cv.wholesale_price)
    .bind(cv.installment_price)
    .bind(time::now_utc_string())
    .bind(&uv.id)
    .execute(&mut *tx)
    .await
    .map_err(|e| map_unique(e, "SKU or barcode"))?;
    let prices_changed = (opc, orp, owp, oip)
        != (
            cv.purchase_cost,
            cv.retail_price,
            cv.wholesale_price,
            cv.installment_price,
        );
    audit::log(
        &mut tx,
        p,
        if prices_changed {
            "variant.prices_changed"
        } else {
            "variant.updated"
        },
        "variant",
        &uv.id,
        serde_json::json!({
            "sku": {"before": osku, "after": cv.sku},
            "purchase_cost": {"before": opc, "after": cv.purchase_cost},
            "retail_price": {"before": orp, "after": cv.retail_price},
            "wholesale_price": {"before": owp, "after": cv.wholesale_price},
            "installment_price": {"before": oip, "after": cv.installment_price},
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_variant_active(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE product_variants SET is_active = ?, updated_at = ? WHERE id = ?")
        .bind(active as i64)
        .bind(time::now_utc_string())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("variant".into()));
    }
    audit::log(
        &mut tx,
        p,
        if active {
            "variant.activated"
        } else {
            "variant.deactivated"
        },
        "variant",
        &id.to_string(),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_product_image(
    pool: &SqlitePool,
    p: &Principal,
    product_id: Id,
    data: &[u8],
) -> CoreResult<()> {
    p.require(perm::PRODUCTS_MANAGE)?;
    if data.is_empty() || data.len() > crate::settings::MAX_LOGO_BYTES {
        return Err(CoreError::Validation(
            "image must be 1 byte to 512 KB".into(),
        ));
    }
    let mime = crate::settings::sniff_image(data)
        .ok_or_else(|| CoreError::Validation("image must be PNG or JPEG".into()))?;
    let mut tx = pool.begin().await?;
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE id = ?")
        .bind(product_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
    if exists == 0 {
        return Err(CoreError::NotFound("product".into()));
    }
    let key = format!("product.{product_id}");
    let sha = hex::encode(Sha256::digest(data));
    sqlx::query(
        "INSERT INTO binary_assets (key, mime, data, sha256, updated_at) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET mime = excluded.mime, data = excluded.data, sha256 = excluded.sha256, updated_at = excluded.updated_at",
    )
    .bind(&key)
    .bind(mime)
    .bind(data)
    .bind(&sha)
    .bind(time::now_utc_string())
    .execute(&mut *tx)
    .await?;
    audit::log(
        &mut tx,
        p,
        "product.image_set",
        "product",
        &product_id.to_string(),
        serde_json::json!({"sha256": sha}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_product_image(
    pool: &SqlitePool,
    p: &Principal,
    product_id: Id,
) -> CoreResult<Option<(String, Vec<u8>)>> {
    p.require(perm::PRODUCTS_VIEW)?;
    Ok(
        sqlx::query_as("SELECT mime, data FROM binary_assets WHERE key = ?")
            .bind(format!("product.{product_id}"))
            .fetch_optional(pool)
            .await?,
    )
}

// ------------------------------------------------------------------ queries

#[derive(Debug, Serialize)]
pub struct VariantRow {
    pub id: String,
    pub sku: String,
    pub barcode: Option<String>,
    pub colour: String,
    pub capacity: String,
    pub size: String,
    pub purchase_cost: i64,
    pub retail_price: i64,
    pub wholesale_price: i64,
    pub installment_price: Option<i64>,
    pub is_active: bool,
    pub available: i64,
    pub on_hand_total: i64,
    pub stock_value: i64,
}

#[derive(Debug, Serialize)]
pub struct ProductRow {
    pub id: String,
    pub brand_id: String,
    pub brand: String,
    pub category_id: String,
    pub category: String,
    pub name: String,
    pub model: String,
    pub description: String,
    pub warranty_months: i64,
    pub warranty_terms: String,
    pub min_stock: i64,
    pub is_serialized: bool,
    pub is_active: bool,
    pub available_total: i64,
    pub low_stock: bool,
    pub variants: Vec<VariantRow>,
}

#[derive(Debug, Serialize)]
pub struct ProductPage {
    pub total: i64,
    pub rows: Vec<ProductRow>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProductFilter {
    pub search: Option<String>,
    pub brand_id: Option<String>,
    pub category_id: Option<String>,
    pub include_inactive: bool,
    pub low_stock_only: bool,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn like(s: &str) -> String {
    format!(
        "%{}%",
        s.trim()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

type ProductRaw = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    i64,
    i64,
    i64,
    i64,
);

const PRODUCT_SELECT: &str = "SELECT p.id, p.brand_id, b.name, p.category_id, c.name, p.name, p.model, p.description, p.warranty_months, p.warranty_terms, p.min_stock, p.is_serialized, p.is_active,
    COALESCE((SELECT SUM(sb.qty) FROM stock_balances sb JOIN product_variants pv ON pv.id = sb.variant_id WHERE pv.product_id = p.id AND sb.bucket = 'available'), 0)
  FROM products p JOIN brands b ON b.id = p.brand_id JOIN categories c ON c.id = p.category_id";

async fn load_variants(pool: &SqlitePool, product_id: &str) -> CoreResult<Vec<VariantRow>> {
    type Raw = (
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        Option<i64>,
        i64,
        i64,
        i64,
        i64,
    );
    let rows: Vec<Raw> = sqlx::query_as(
        "SELECT v.id, v.sku, v.barcode, v.colour, v.capacity, v.size, v.purchase_cost, v.retail_price, v.wholesale_price, v.installment_price, v.is_active,
            COALESCE((SELECT qty FROM stock_balances WHERE variant_id = v.id AND bucket = 'available'), 0),
            COALESCE((SELECT SUM(qty) FROM stock_balances WHERE variant_id = v.id), 0),
            COALESCE((SELECT SUM(value) FROM stock_balances WHERE variant_id = v.id), 0)
         FROM product_variants v WHERE v.product_id = ? ORDER BY v.sku",
    )
    .bind(product_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| VariantRow {
            id: r.0,
            sku: r.1,
            barcode: r.2,
            colour: r.3,
            capacity: r.4,
            size: r.5,
            purchase_cost: r.6,
            retail_price: r.7,
            wholesale_price: r.8,
            installment_price: r.9,
            is_active: r.10 == 1,
            available: r.11,
            on_hand_total: r.12,
            stock_value: r.13,
        })
        .collect())
}

fn to_row(r: ProductRaw, variants: Vec<VariantRow>) -> ProductRow {
    ProductRow {
        id: r.0,
        brand_id: r.1,
        brand: r.2,
        category_id: r.3,
        category: r.4,
        name: r.5,
        model: r.6,
        description: r.7,
        warranty_months: r.8,
        warranty_terms: r.9,
        min_stock: r.10,
        is_serialized: r.11 == 1,
        is_active: r.12 == 1,
        available_total: r.13,
        low_stock: r.10 > 0 && r.13 < r.10,
        variants,
    }
}

pub async fn list_products(
    pool: &SqlitePool,
    p: &Principal,
    f: ProductFilter,
) -> CoreResult<ProductPage> {
    p.require(perm::PRODUCTS_VIEW)?;
    let search = f.search.as_deref().map(like);
    let where_sql = "WHERE (?1 IS NULL OR p.name LIKE ?1 ESCAPE '\\' OR p.model LIKE ?1 ESCAPE '\\' OR b.name LIKE ?1 ESCAPE '\\'
            OR EXISTS (SELECT 1 FROM product_variants v WHERE v.product_id = p.id AND (v.sku LIKE ?1 ESCAPE '\\' OR v.barcode LIKE ?1 ESCAPE '\\')))
        AND (?2 IS NULL OR p.brand_id = ?2) AND (?3 IS NULL OR p.category_id = ?3 OR c.parent_id = ?3)
        AND (?4 = 1 OR p.is_active = 1)";
    let low = if f.low_stock_only {
        " AND p.min_stock > 0 AND COALESCE((SELECT SUM(sb.qty) FROM stock_balances sb JOIN product_variants pv ON pv.id = sb.variant_id WHERE pv.product_id = p.id AND sb.bucket = 'available'), 0) < p.min_stock"
    } else {
        ""
    };
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM products p JOIN brands b ON b.id = p.brand_id JOIN categories c ON c.id = p.category_id {where_sql}{low}"))
        .bind(&search)
        .bind(&f.brand_id)
        .bind(&f.category_id)
        .bind(f.include_inactive as i64)
        .fetch_one(pool)
        .await?;
    let raws: Vec<ProductRaw> = sqlx::query_as(&format!(
        "{PRODUCT_SELECT} {where_sql}{low} ORDER BY p.name COLLATE NOCASE LIMIT ?5 OFFSET ?6"
    ))
    .bind(&search)
    .bind(&f.brand_id)
    .bind(&f.category_id)
    .bind(f.include_inactive as i64)
    .bind(f.limit.unwrap_or(50).clamp(1, 200))
    .bind(f.offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await?;
    let mut rows = Vec::with_capacity(raws.len());
    for r in raws {
        let v = load_variants(pool, &r.0).await?;
        rows.push(to_row(r, v));
    }
    Ok(ProductPage { total, rows })
}

pub async fn get_product(pool: &SqlitePool, p: &Principal, id: Id) -> CoreResult<ProductRow> {
    p.require(perm::PRODUCTS_VIEW)?;
    let r: ProductRaw = sqlx::query_as(&format!("{PRODUCT_SELECT} WHERE p.id = ?"))
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| CoreError::NotFound("product".into()))?;
    let v = load_variants(pool, &r.0).await?;
    Ok(to_row(r, v))
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VariantPick {
    pub variant_id: String,
    pub product_id: String,
    pub label: String,
    pub sku: String,
    pub barcode: Option<String>,
    pub is_serialized: bool,
    pub purchase_cost: i64,
    pub retail_price: i64,
    pub wholesale_price: i64,
    pub installment_price: Option<i64>,
    pub available: i64,
}

/// Fast picker for forms/POS: exact barcode/SKU match first, then name/model/brand text.
pub async fn search_variants(
    pool: &SqlitePool,
    p: &Principal,
    q: &str,
    limit: i64,
) -> CoreResult<Vec<VariantPick>> {
    p.require(perm::PRODUCTS_VIEW)?;
    let term = q.trim();
    if term.is_empty() {
        return Ok(vec![]);
    }
    Ok(sqlx::query_as(
        "SELECT v.id AS variant_id, p.id AS product_id,
            TRIM(b.name || ' ' || p.name || CASE WHEN p.model <> '' THEN ' ' || p.model ELSE '' END ||
              CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END || CASE WHEN v.capacity <> '' THEN ' · ' || v.capacity ELSE '' END ||
              CASE WHEN v.size <> '' THEN ' · ' || v.size ELSE '' END) AS label,
            v.sku, v.barcode, p.is_serialized AS is_serialized, v.purchase_cost, v.retail_price, v.wholesale_price, v.installment_price,
            COALESCE((SELECT qty FROM stock_balances WHERE variant_id = v.id AND bucket = 'available'), 0) AS available
         FROM product_variants v JOIN products p ON p.id = v.product_id JOIN brands b ON b.id = p.brand_id
         WHERE p.is_active = 1 AND v.is_active = 1 AND (v.sku = ?1 OR v.barcode = ?1 OR p.name LIKE ?2 ESCAPE '\\' OR p.model LIKE ?2 ESCAPE '\\' OR b.name LIKE ?2 ESCAPE '\\' OR v.sku LIKE ?2 ESCAPE '\\')
         ORDER BY (v.sku = ?1 OR v.barcode = ?1) DESC, p.name COLLATE NOCASE LIMIT ?3",
    )
    .bind(term)
    .bind(like(term))
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await?)
}
