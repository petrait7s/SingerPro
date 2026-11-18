//! Role-based access control. Permissions are enforced HERE (services), never only in the UI.

use crate::{time, CoreError, CoreResult, Id};
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashSet;

pub mod perm {
    pub const USERS_VIEW: &str = "users.view";
    pub const USERS_MANAGE: &str = "users.manage";
    pub const ROLES_MANAGE: &str = "roles.manage";
    pub const SETTINGS_VIEW: &str = "settings.view";
    pub const SETTINGS_MANAGE: &str = "settings.manage";
    pub const TAX_MANAGE: &str = "tax.manage";
    pub const AUDIT_VIEW: &str = "audit.view";
    pub const BACKUP_CREATE: &str = "backup.create";
    pub const BACKUP_RESTORE: &str = "backup.restore";
}

pub struct PermDef {
    pub key: &'static str,
    pub category: &'static str,
    pub description: &'static str,
}

const fn p(key: &'static str, category: &'static str, description: &'static str) -> PermDef {
    PermDef {
        key,
        category,
        description,
    }
}

/// Full catalogue, including permissions for later phases so roles are meaningful from day one.
pub const CATALOGUE: &[PermDef] = &[
    p("users.view", "Administration", "View users and roles"),
    p(
        "users.manage",
        "Administration",
        "Create/disable users, assign roles, reset passwords",
    ),
    p(
        "roles.manage",
        "Administration",
        "Create roles and edit role permissions",
    ),
    p("settings.view", "Administration", "View settings"),
    p(
        "settings.manage",
        "Administration",
        "Change shop identity, currency and installment defaults",
    ),
    p("tax.manage", "Administration", "Change tax configuration"),
    p("audit.view", "Administration", "View the audit log"),
    p("backup.create", "Data safety", "Create and verify backups"),
    p(
        "backup.restore",
        "Data safety",
        "Restore the database from a backup",
    ),
    p("products.view", "Catalogue", "View products"),
    p("products.manage", "Catalogue", "Create and edit products"),
    p("inventory.view", "Inventory", "View stock"),
    p("inventory.adjust", "Inventory", "Make stock adjustments"),
    p("suppliers.manage", "Purchasing", "Manage suppliers"),
    p(
        "purchases.manage",
        "Purchasing",
        "Create and receive purchases",
    ),
    p("customers.view", "Customers", "View customers"),
    p("customers.manage", "Customers", "Create and edit customers"),
    p("sales.view", "Sales", "View sales"),
    p("sales.create", "Sales", "Create retail sales"),
    p("sales.discount", "Sales", "Apply discounts"),
    p("sales.wholesale", "Sales", "Create wholesale sales"),
    p(
        "installments.view",
        "Installments",
        "View installment accounts",
    ),
    p(
        "installments.create",
        "Installments",
        "Create installment agreements",
    ),
    p(
        "installments.collect",
        "Installments",
        "Collect installment payments",
    ),
    p(
        "installments.reschedule",
        "Installments",
        "Reschedule installments",
    ),
    p(
        "installments.reverse_payment",
        "Installments",
        "Reverse collected payments",
    ),
    p("returns.process", "Returns", "Record returns"),
    p(
        "returns.approve",
        "Returns",
        "Approve refunds and restocking",
    ),
    p("expenses.create", "Finance", "Record expenses"),
    p("expenses.approve", "Finance", "Approve expenses"),
    p("accounts.view", "Finance", "View accounts and journals"),
    p("reports.operational", "Reports", "Operational reports"),
    p("reports.financial", "Reports", "Financial reports"),
    p("cash.session", "Finance", "Open and close cash sessions"),
];

pub const OWNER_ROLE: &str = "Owner";

/// (name, description, permissions). Owner always receives the full catalogue.
const ROLE_DEFAULTS: &[(&str, &str, &[&str])] = &[
    (OWNER_ROLE, "Full control of the business", &[]),
    (
        "Manager",
        "Runs daily operations",
        &[
            "users.view",
            "settings.view",
            "audit.view",
            "products.view",
            "products.manage",
            "inventory.view",
            "inventory.adjust",
            "suppliers.manage",
            "purchases.manage",
            "customers.view",
            "customers.manage",
            "sales.view",
            "sales.create",
            "sales.discount",
            "sales.wholesale",
            "installments.view",
            "installments.create",
            "installments.collect",
            "installments.reschedule",
            "returns.process",
            "returns.approve",
            "expenses.create",
            "expenses.approve",
            "accounts.view",
            "reports.operational",
            "reports.financial",
            "cash.session",
            "backup.create",
        ],
    ),
    (
        "Sales Executive",
        "Sells and manages customers",
        &[
            "products.view",
            "inventory.view",
            "customers.view",
            "customers.manage",
            "sales.view",
            "sales.create",
            "sales.wholesale",
            "installments.view",
            "installments.create",
            "installments.collect",
        ],
    ),
    (
        "Cashier",
        "Retail counter and collections",
        &[
            "products.view",
            "inventory.view",
            "customers.view",
            "sales.view",
            "sales.create",
            "installments.view",
            "installments.collect",
            "cash.session",
        ],
    ),
    (
        "Accountant",
        "Books and financial reports",
        &[
            "accounts.view",
            "reports.operational",
            "reports.financial",
            "expenses.create",
            "expenses.approve",
            "customers.view",
            "sales.view",
            "installments.view",
            "cash.session",
        ],
    ),
    (
        "Inventory Manager",
        "Stock control",
        &[
            "products.view",
            "products.manage",
            "inventory.view",
            "inventory.adjust",
            "reports.operational",
        ],
    ),
    (
        "Purchase Manager",
        "Purchasing and suppliers",
        &[
            "products.view",
            "inventory.view",
            "suppliers.manage",
            "purchases.manage",
            "reports.operational",
        ],
    ),
];

/// Upsert the catalogue, create system roles once, and keep Owner = every permission.
/// Never changes permissions of non-Owner roles that already exist (admins may have edited them).
pub async fn sync_catalogue(pool: &SqlitePool) -> CoreResult<()> {
    let mut tx = pool.begin().await?;
    for d in CATALOGUE {
        sqlx::query(
            "INSERT INTO permissions (key, category, description) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET category = excluded.category, description = excluded.description",
        )
        .bind(d.key)
        .bind(d.category)
        .bind(d.description)
        .execute(&mut *tx)
        .await?;
    }
    for (name, desc, perms) in ROLE_DEFAULTS {
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM roles WHERE name = ?")
            .bind(name)
            .fetch_optional(&mut *tx)
            .await?;
        let role_id = match existing {
            Some(id) => id,
            None => {
                let id = Id::new().to_string();
                sqlx::query("INSERT INTO roles (id, name, description, is_system, created_at) VALUES (?, ?, ?, 1, ?)")
                    .bind(&id)
                    .bind(name)
                    .bind(desc)
                    .bind(time::now_utc_string())
                    .execute(&mut *tx)
                    .await?;
                if *name != OWNER_ROLE {
                    for k in *perms {
                        sqlx::query("INSERT OR IGNORE INTO role_permissions (role_id, perm_key) VALUES (?, ?)")
                            .bind(&id)
                            .bind(k)
                            .execute(&mut *tx)
                            .await?;
                    }
                }
                id
            }
        };
        if *name == OWNER_ROLE {
            for d in CATALOGUE {
                sqlx::query(
                    "INSERT OR IGNORE INTO role_permissions (role_id, perm_key) VALUES (?, ?)",
                )
                .bind(&role_id)
                .bind(d.key)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

/// The authenticated caller with permissions loaded FRESH from the database on every request,
/// so role changes and deactivation take effect immediately.
#[derive(Debug, Clone, Serialize)]
pub struct Principal {
    pub user_id: Id,
    pub username: String,
    pub full_name: String,
    pub must_change_password: bool,
    pub roles: Vec<String>,
    pub permissions: HashSet<String>,
}

impl Principal {
    pub fn has(&self, key: &str) -> bool {
        self.permissions.contains(key)
    }

    /// Fails closed: an account that must change its password can do nothing else.
    pub fn require(&self, key: &str) -> CoreResult<()> {
        if self.must_change_password {
            return Err(CoreError::Forbidden("password change required".into()));
        }
        if self.has(key) {
            Ok(())
        } else {
            Err(CoreError::Forbidden(key.to_string()))
        }
    }

    pub fn is_owner(&self) -> bool {
        self.roles
            .iter()
            .any(|r| r.eq_ignore_ascii_case(OWNER_ROLE))
    }
}

pub async fn load_principal(pool: &SqlitePool, user_id: Id) -> CoreResult<Principal> {
    let row: Option<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT username, full_name, is_active, must_change_password FROM users WHERE id = ?",
    )
    .bind(user_id.to_string())
    .fetch_optional(pool)
    .await?;
    let (username, full_name, active, must) = row.ok_or(CoreError::Unauthenticated)?;
    if active != 1 {
        return Err(CoreError::Unauthenticated);
    }
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT r.name FROM user_roles ur JOIN roles r ON r.id = ur.role_id WHERE ur.user_id = ? ORDER BY r.name",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    let perms: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT rp.perm_key FROM user_roles ur JOIN role_permissions rp ON rp.role_id = ur.role_id WHERE ur.user_id = ?",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(Principal {
        user_id,
        username,
        full_name,
        must_change_password: must == 1,
        roles,
        permissions: perms.into_iter().collect(),
    })
}
