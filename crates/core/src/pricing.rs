//! Wholesale pricing: price tiers (percentage off/over the wholesale price) and customer-specific prices.
//! Precedence for wholesale sales: customer-specific price > tier-adjusted wholesale price > wholesale price.

use crate::audit;
use crate::authz::{perm, Principal};
use crate::{time, CoreError, CoreResult, Id, Money};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TierRow {
    pub id: String,
    pub name: String,
    pub adjust_bp: i64,
    pub is_active: bool,
    pub customers: i64,
}

pub async fn list_tiers(pool: &SqlitePool, p: &Principal) -> CoreResult<Vec<TierRow>> {
    p.require(perm::CUSTOMERS_VIEW)?;
    Ok(sqlx::query_as("SELECT t.id, t.name, t.adjust_bp, t.is_active, (SELECT COUNT(*) FROM customers c WHERE c.price_tier_id = t.id) AS customers FROM price_tiers t ORDER BY t.name").fetch_all(pool).await?)
}

pub async fn create_tier(
    pool: &SqlitePool,
    p: &Principal,
    name: &str,
    adjust_bp: i64,
) -> CoreResult<Id> {
    p.require(perm::PRICING_MANAGE)?;
    let name = name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err(CoreError::Validation(
            "tier name must be 1-60 characters".into(),
        ));
    }
    if !(-5000..=5000).contains(&adjust_bp) {
        return Err(CoreError::Validation(
            "tier adjustment must be between -50% and +50%".into(),
        ));
    }
    let id = Id::new();
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO price_tiers (id, name, adjust_bp, created_at) VALUES (?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(name)
        .bind(adjust_bp)
        .bind(time::now_utc_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| crate::catalogue::map_unique(e, "price tier"))?;
    audit::log(
        &mut tx,
        p,
        "pricing.tier_created",
        "price_tier",
        &id.to_string(),
        serde_json::json!({"name": name, "adjust_bp": adjust_bp}),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn update_tier(
    pool: &SqlitePool,
    p: &Principal,
    id: Id,
    adjust_bp: i64,
    active: bool,
) -> CoreResult<()> {
    p.require(perm::PRICING_MANAGE)?;
    if !(-5000..=5000).contains(&adjust_bp) {
        return Err(CoreError::Validation(
            "tier adjustment must be between -50% and +50%".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    let old: Option<(i64, i64)> =
        sqlx::query_as("SELECT adjust_bp, is_active FROM price_tiers WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    let (old_bp, old_active) = old.ok_or_else(|| CoreError::NotFound("price tier".into()))?;
    sqlx::query("UPDATE price_tiers SET adjust_bp = ?, is_active = ? WHERE id = ?")
        .bind(adjust_bp)
        .bind(active as i64)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    audit::log(&mut tx, p, "pricing.tier_changed", "price_tier", &id.to_string(), serde_json::json!({"adjust_bp": {"before": old_bp, "after": adjust_bp}, "active": {"before": old_active == 1, "after": active}})).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_customer_tier(
    pool: &SqlitePool,
    p: &Principal,
    customer_id: Id,
    tier_id: Option<Id>,
) -> CoreResult<()> {
    p.require(perm::PRICING_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("UPDATE customers SET price_tier_id = ?, updated_at = ? WHERE id = ?")
        .bind(tier_id.map(|t| t.to_string()))
        .bind(time::now_utc_string())
        .bind(customer_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| crate::catalogue::map_unique(e, "price tier"))?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("customer".into()));
    }
    audit::log(
        &mut tx,
        p,
        "pricing.customer_tier_set",
        "customer",
        &customer_id.to_string(),
        serde_json::json!({"tier": tier_id.map(|t| t.to_string())}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct CustomerPriceRow {
    pub variant_id: String,
    pub label: String,
    pub sku: String,
    pub wholesale_price: i64,
    pub price: i64,
}

pub async fn list_customer_prices(
    pool: &SqlitePool,
    p: &Principal,
    customer_id: Id,
) -> CoreResult<Vec<CustomerPriceRow>> {
    p.require(perm::CUSTOMERS_VIEW)?;
    Ok(sqlx::query_as(
        "SELECT cp.variant_id, TRIM(pr.name || CASE WHEN pr.model <> '' THEN ' ' || pr.model ELSE '' END || CASE WHEN v.colour <> '' THEN ' · ' || v.colour ELSE '' END) AS label, v.sku, v.wholesale_price, cp.price
         FROM customer_prices cp JOIN product_variants v ON v.id = cp.variant_id JOIN products pr ON pr.id = v.product_id WHERE cp.customer_id = ? ORDER BY pr.name",
    )
    .bind(customer_id.to_string())
    .fetch_all(pool)
    .await?)
}

pub async fn set_customer_price(
    pool: &SqlitePool,
    p: &Principal,
    customer_id: Id,
    variant_id: Id,
    price: i64,
) -> CoreResult<()> {
    p.require(perm::PRICING_MANAGE)?;
    if price < 0 {
        return Err(CoreError::Validation("price cannot be negative".into()));
    }
    let mut tx = pool.begin().await?;
    let old: Option<i64> = sqlx::query_scalar(
        "SELECT price FROM customer_prices WHERE customer_id = ? AND variant_id = ?",
    )
    .bind(customer_id.to_string())
    .bind(variant_id.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO customer_prices (customer_id, variant_id, price, updated_at) VALUES (?, ?, ?, ?)
         ON CONFLICT(customer_id, variant_id) DO UPDATE SET price = excluded.price, updated_at = excluded.updated_at",
    )
    .bind(customer_id.to_string())
    .bind(variant_id.to_string())
    .bind(price)
    .bind(time::now_utc_string())
    .execute(&mut *tx)
    .await
    .map_err(|e| crate::catalogue::map_unique(e, "customer or product"))?;
    audit::log(
        &mut tx,
        p,
        "pricing.customer_price_set",
        "customer",
        &customer_id.to_string(),
        serde_json::json!({"variant": variant_id.to_string(), "before": old, "after": price}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn remove_customer_price(
    pool: &SqlitePool,
    p: &Principal,
    customer_id: Id,
    variant_id: Id,
) -> CoreResult<()> {
    p.require(perm::PRICING_MANAGE)?;
    let mut tx = pool.begin().await?;
    let r = sqlx::query("DELETE FROM customer_prices WHERE customer_id = ? AND variant_id = ?")
        .bind(customer_id.to_string())
        .bind(variant_id.to_string())
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        return Err(CoreError::NotFound("customer price".into()));
    }
    audit::log(
        &mut tx,
        p,
        "pricing.customer_price_removed",
        "customer",
        &customer_id.to_string(),
        serde_json::json!({"variant": variant_id.to_string()}),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Wholesale unit price for a customer and variant (used by sales and the POS preview).
pub(crate) async fn wholesale_price(
    conn: &mut SqliteConnection,
    customer_id: Option<&str>,
    variant_id: &str,
    list_wholesale: i64,
) -> CoreResult<i64> {
    let Some(cid) = customer_id else {
        return Ok(list_wholesale);
    };
    if let Some(special) = sqlx::query_scalar::<_, i64>(
        "SELECT price FROM customer_prices WHERE customer_id = ? AND variant_id = ?",
    )
    .bind(cid)
    .bind(variant_id)
    .fetch_optional(&mut *conn)
    .await?
    {
        return Ok(special);
    }
    let bp: Option<i64> = sqlx::query_scalar("SELECT t.adjust_bp FROM customers c JOIN price_tiers t ON t.id = c.price_tier_id WHERE c.id = ? AND t.is_active = 1").bind(cid).fetch_optional(&mut *conn).await?;
    match bp {
        Some(bp) if bp != 0 => Ok(Money::from_paisa(list_wholesale)
            .checked_add(Money::from_paisa(list_wholesale).percent_bp(bp)?)?
            .paisa()),
        _ => Ok(list_wholesale),
    }
}
