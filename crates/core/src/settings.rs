//! Validated, permissioned, audited configuration. Unknown keys are rejected.

use crate::audit::{self, AuditEntry};
use crate::authz::{perm, Principal};
use crate::{time, CoreError, CoreResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy)]
enum Kind {
    Text { max: usize, required: bool },
    Bool,
    Int { min: i64, max: i64 },
    Enum(&'static [&'static str]),
    Currency,
    Prefix,
}

struct Def {
    key: &'static str,
    kind: Kind,
    permission: &'static str,
}

const T80: Kind = Kind::Text {
    max: 80,
    required: true,
};
const T200: Kind = Kind::Text {
    max: 200,
    required: false,
};

const DEFS: &[Def] = &[
    Def {
        key: "shop.name",
        kind: T80,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "shop.address",
        kind: T200,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "shop.phone",
        kind: Kind::Text {
            max: 40,
            required: false,
        },
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "shop.email",
        kind: Kind::Text {
            max: 80,
            required: false,
        },
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "shop.invoice_footer",
        kind: T200,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "invoice.prefix",
        kind: Kind::Prefix,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "currency.code",
        kind: Kind::Currency,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "currency.symbol",
        kind: Kind::Text {
            max: 4,
            required: true,
        },
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "timezone",
        kind: Kind::Enum(&["Asia/Dhaka"]),
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "tax.enabled",
        kind: Kind::Bool,
        permission: perm::TAX_MANAGE,
    },
    Def {
        key: "tax.label",
        kind: Kind::Text {
            max: 20,
            required: true,
        },
        permission: perm::TAX_MANAGE,
    },
    Def {
        key: "tax.default_rate_bp",
        kind: Kind::Int {
            min: 0,
            max: 10_000,
        },
        permission: perm::TAX_MANAGE,
    },
    Def {
        key: "tax.prices_include_tax",
        kind: Kind::Bool,
        permission: perm::TAX_MANAGE,
    },
    Def {
        key: "installment.default_markup_bp",
        kind: Kind::Int {
            min: 0,
            max: 10_000,
        },
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "installment.rounding_remainder",
        kind: Kind::Enum(&["last_installment"]),
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "installment.markup_recognition",
        kind: Kind::Enum(&["proportional_to_collections", "on_agreement"]),
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "installment.grace_days",
        kind: Kind::Int { min: 0, max: 365 },
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "installment.late_fee_enabled",
        kind: Kind::Bool,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "installment.late_fee_bp",
        kind: Kind::Int {
            min: 0,
            max: 10_000,
        },
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "customers.collect_nid",
        kind: Kind::Bool,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "sales.credit_limit_enforced",
        kind: Kind::Bool,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "stock.allow_negative",
        kind: Kind::Bool,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "returns.require_inspection",
        kind: Kind::Bool,
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "backup.reminder_days",
        kind: Kind::Int { min: 0, max: 365 },
        permission: perm::SETTINGS_MANAGE,
    },
    Def {
        key: "ui.language",
        kind: Kind::Enum(&["en", "bn"]),
        permission: perm::SETTINGS_MANAGE,
    },
];

/// Defaults seeded on first run (never overwrite existing values).
pub const DEFAULTS: &[(&str, &str)] = &[
    ("shop.name", "Singer Pro"),
    ("shop.address", ""),
    ("shop.phone", ""),
    ("shop.email", ""),
    ("shop.invoice_footer", ""),
    ("invoice.prefix", "INV-"),
    ("currency.code", "BDT"),
    ("currency.symbol", "৳"),
    ("timezone", "Asia/Dhaka"),
    ("tax.enabled", "false"),
    ("tax.label", "VAT"),
    ("tax.default_rate_bp", "0"),
    ("tax.prices_include_tax", "false"),
    ("installment.default_markup_bp", "0"),
    ("installment.rounding_remainder", "last_installment"),
    (
        "installment.markup_recognition",
        "proportional_to_collections",
    ),
    ("installment.grace_days", "0"),
    ("installment.late_fee_enabled", "false"),
    ("installment.late_fee_bp", "0"),
    ("customers.collect_nid", "false"),
    ("sales.credit_limit_enforced", "true"),
    ("stock.allow_negative", "false"),
    ("returns.require_inspection", "true"),
    ("backup.reminder_days", "7"),
    ("ui.language", "en"),
];

fn normalize(def: &Def, raw: &str) -> CoreResult<String> {
    let bad = |m: String| Err(CoreError::Validation(format!("{}: {m}", def.key)));
    let v = raw.trim();
    match def.kind {
        Kind::Text { max, required } => {
            if required && v.is_empty() {
                return bad("required".into());
            }
            if v.chars().count() > max {
                return bad(format!("at most {max} characters"));
            }
            if v.chars().any(|c| c.is_control()) {
                return bad("control characters not allowed".into());
            }
            Ok(v.to_string())
        }
        Kind::Bool => match v {
            "true" | "false" => Ok(v.to_string()),
            _ => bad("must be true or false".into()),
        },
        Kind::Int { min, max } => match v.parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => Ok(n.to_string()),
            _ => bad(format!("integer between {min} and {max}")),
        },
        Kind::Enum(opts) => {
            if opts.contains(&v) {
                Ok(v.to_string())
            } else {
                bad(format!("one of {opts:?}"))
            }
        }
        Kind::Currency => {
            if v.len() == 3 && v.chars().all(|c| c.is_ascii_uppercase()) {
                Ok(v.to_string())
            } else {
                bad("3 uppercase letters, e.g. BDT".into())
            }
        }
        Kind::Prefix => {
            if v.len() <= 8
                && v.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
            {
                Ok(v.to_string())
            } else {
                bad("up to 8 characters: A-Z, 0-9, '-'".into())
            }
        }
    }
}

pub async fn seed_defaults(pool: &SqlitePool) -> CoreResult<()> {
    let now = time::now_utc_string();
    for (k, v) in DEFAULTS {
        sqlx::query("INSERT OR IGNORE INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)")
            .bind(k)
            .bind(v)
            .bind(&now)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn get_all(pool: &SqlitePool, p: &Principal) -> CoreResult<BTreeMap<String, String>> {
    p.require(perm::SETTINGS_VIEW)?;
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM app_settings")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|(k, _)| DEFS.iter().any(|d| d.key == k))
        .collect())
}

#[derive(Debug, Serialize)]
pub struct Branding {
    pub shop_name: String,
    pub shop_address: String,
    pub shop_phone: String,
    pub shop_email: String,
    pub invoice_footer: String,
    pub currency_symbol: String,
    pub language: String,
}

/// Non-sensitive display settings, available to any signed-in user.
pub async fn branding(pool: &SqlitePool, _p: &Principal) -> CoreResult<Branding> {
    let get = |k: &'static str| async move {
        sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
            .bind(k)
            .fetch_optional(pool)
            .await
    };
    Ok(Branding {
        shop_name: get("shop.name")
            .await?
            .unwrap_or_else(|| "Singer Pro".into()),
        shop_address: get("shop.address").await?.unwrap_or_default(),
        shop_phone: get("shop.phone").await?.unwrap_or_default(),
        shop_email: get("shop.email").await?.unwrap_or_default(),
        invoice_footer: get("shop.invoice_footer").await?.unwrap_or_default(),
        currency_symbol: get("currency.symbol").await?.unwrap_or_else(|| "৳".into()),
        language: get("ui.language").await?.unwrap_or_else(|| "en".into()),
    })
}

/// Atomic multi-key update. Returns the keys that actually changed. Each change is audited with
/// before/after values; permission is checked per key (tax keys need `tax.manage`).
pub async fn update(
    pool: &SqlitePool,
    p: &Principal,
    changes: HashMap<String, String>,
) -> CoreResult<Vec<String>> {
    let mut staged: Vec<(&Def, String)> = Vec::new();
    for (k, raw) in &changes {
        let def = DEFS
            .iter()
            .find(|d| d.key == k)
            .ok_or_else(|| CoreError::Validation(format!("unknown setting: {k}")))?;
        p.require(def.permission)?;
        staged.push((def, normalize(def, raw)?));
    }
    staged.sort_by_key(|(d, _)| d.key);
    let mut tx = pool.begin().await?;
    let mut changed = Vec::new();
    for (def, new) in staged {
        let old: Option<String> =
            sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
                .bind(def.key)
                .fetch_optional(&mut *tx)
                .await?;
        if old.as_deref() == Some(new.as_str()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO app_settings (key, value, updated_at, updated_by) VALUES (?, ?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
        )
        .bind(def.key)
        .bind(&new)
        .bind(time::now_utc_string())
        .bind(p.user_id.to_string())
        .execute(&mut *tx)
        .await?;
        let action = if def.key.starts_with("tax.") {
            "settings.tax_changed"
        } else {
            "settings.changed"
        };
        audit::record(
            &mut tx,
            AuditEntry {
                actor_id: Some(p.user_id),
                action,
                entity_type: "setting",
                entity_id: Some(def.key),
                detail: Some(serde_json::json!({ "before": old, "after": new })),
                reason: None,
            },
        )
        .await?;
        changed.push(def.key.to_string());
    }
    tx.commit().await?;
    Ok(changed)
}

pub const MAX_LOGO_BYTES: usize = 512 * 1024;

pub(crate) fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// The mime type is derived from the file's own bytes, never trusted from the caller.
pub async fn set_logo(pool: &SqlitePool, p: &Principal, data: &[u8]) -> CoreResult<()> {
    p.require(perm::SETTINGS_MANAGE)?;
    if data.is_empty() || data.len() > MAX_LOGO_BYTES {
        return Err(CoreError::Validation(format!(
            "logo must be 1 byte to {} KB",
            MAX_LOGO_BYTES / 1024
        )));
    }
    let mime = sniff_image(data)
        .ok_or_else(|| CoreError::Validation("logo must be a PNG or JPEG image".into()))?;
    let sha = hex::encode(Sha256::digest(data));
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO binary_assets (key, mime, data, sha256, updated_at) VALUES ('shop.logo', ?, ?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET mime = excluded.mime, data = excluded.data, sha256 = excluded.sha256, updated_at = excluded.updated_at",
    )
    .bind(mime)
    .bind(data)
    .bind(&sha)
    .bind(time::now_utc_string())
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut tx,
        AuditEntry {
            actor_id: Some(p.user_id),
            action: "settings.logo_set",
            entity_type: "setting",
            entity_id: Some("shop.logo"),
            detail: Some(serde_json::json!({ "sha256": sha, "bytes": data.len(), "mime": mime })),
            reason: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn clear_logo(pool: &SqlitePool, p: &Principal) -> CoreResult<()> {
    p.require(perm::SETTINGS_MANAGE)?;
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM binary_assets WHERE key = 'shop.logo'")
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut tx,
        AuditEntry {
            actor_id: Some(p.user_id),
            action: "settings.logo_cleared",
            entity_type: "setting",
            entity_id: Some("shop.logo"),
            detail: None,
            reason: None,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Returns (mime, bytes). Any signed-in user may read it (it appears on every invoice).
pub async fn get_logo(pool: &SqlitePool, _p: &Principal) -> CoreResult<Option<(String, Vec<u8>)>> {
    Ok(
        sqlx::query_as("SELECT mime, data FROM binary_assets WHERE key = 'shop.logo'")
            .fetch_optional(pool)
            .await?,
    )
}
