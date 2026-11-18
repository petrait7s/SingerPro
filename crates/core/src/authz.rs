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
    pub const PRODUCTS_VIEW: &str = "products.view";
    pub const PRODUCTS_MANAGE: &str = "products.manage";
    pub const INVENTORY_VIEW: &str = "inventory.view";
    pub const INVENTORY_ADJUST: &str = "inventory.adjust";
    pub const SUPPLIERS_VIEW: &str = "suppliers.view";
    pub const SUPPLIERS_MANAGE: &str = "suppliers.manage";
    pub const PURCHASES_VIEW: &str = "purchases.view";
    pub const PURCHASES_MANAGE: &str = "purchases.manage";
    pub const PURCHASES_PAY: &str = "purchases.pay";
    pub const PURCHASES_REVERSE_PAYMENT: &str = "purchases.reverse_payment";
    pub const PURCHASES_RETURN: &str = "purchases.return";
    pub const ACCOUNTS_VIEW: &str = "accounts.view";
    pub const ACCOUNTS_MANAGE: &str = "accounts.manage";
    pub const CUSTOMERS_VIEW: &str = "customers.view";
    pub const CUSTOMERS_MANAGE: &str = "customers.manage";
    pub const CUSTOMERS_COLLECT: &str = "customers.collect_payment";
    pub const CUSTOMERS_REVERSE: &str = "customers.reverse_payment";
    pub const SALES_CREDIT_OVERRIDE: &str = "sales.credit_override";
    pub const PRICING_MANAGE: &str = "customers.manage_pricing";
    pub const INSTALLMENTS_VIEW: &str = "installments.view";
    pub const INSTALLMENTS_CREATE: &str = "installments.create";
    pub const INSTALLMENTS_COLLECT: &str = "installments.collect";
    pub const INSTALLMENTS_RESCHEDULE: &str = "installments.reschedule";
    pub const INSTALLMENTS_REVERSE: &str = "installments.reverse_payment";
    pub const INSTALLMENTS_MARKUP: &str = "installments.set_markup";
    pub const SALES_VIEW: &str = "sales.view";
    pub const SALES_CREATE: &str = "sales.create";
    pub const SALES_DISCOUNT: &str = "sales.discount";
    pub const SALES_WHOLESALE: &str = "sales.wholesale";
}

pub struct PermDef {
    pub key: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    /// Release phase that introduced the permission (1 = P1, 2 = P2 …). Used to grant new
    /// permissions to default roles exactly once on upgrade.
    pub since: u8,
}

const fn p(key: &'static str, category: &'static str, description: &'static str) -> PermDef {
    PermDef {
        key,
        category,
        description,
        since: 1,
    }
}

const fn p3(key: &'static str, category: &'static str, description: &'static str) -> PermDef {
    PermDef {
        key,
        category,
        description,
        since: 3,
    }
}

const fn p4(key: &'static str, category: &'static str, description: &'static str) -> PermDef {
    PermDef {
        key,
        category,
        description,
        since: 4,
    }
}

const fn p2(key: &'static str, category: &'static str, description: &'static str) -> PermDef {
    PermDef {
        key,
        category,
        description,
        since: 2,
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
    p3(
        "accounts.manage",
        "Finance",
        "Post opening balances, manual journals and reversals",
    ),
    p3(
        "customers.reverse_payment",
        "Customers",
        "Reverse payments received from customers",
    ),
    p3(
        "sales.credit_override",
        "Sales",
        "Sell on credit beyond a customer's credit limit",
    ),
    p4(
        "customers.manage_pricing",
        "Customers",
        "Manage price tiers and customer-specific prices",
    ),
    p4(
        "installments.set_markup",
        "Installments",
        "Set a markup other than the default and add explicit fees",
    ),
    p3(
        "customers.collect_payment",
        "Customers",
        "Receive payments from customers",
    ),
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
    p2(
        "suppliers.view",
        "Purchasing",
        "View suppliers and supplier ledger",
    ),
    p2("purchases.view", "Purchasing", "View purchases"),
    p2("purchases.pay", "Purchasing", "Record supplier payments"),
    p2(
        "purchases.reverse_payment",
        "Purchasing",
        "Reverse supplier payments",
    ),
    p2(
        "purchases.return",
        "Purchasing",
        "Return goods to suppliers",
    ),
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
            "suppliers.view",
            "purchases.view",
            "purchases.pay",
            "purchases.return",
            "accounts.manage",
            "customers.collect_payment",
            "customers.reverse_payment",
            "sales.credit_override",
            "customers.manage_pricing",
            "installments.set_markup",
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
            "customers.collect_payment",
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
            "customers.collect_payment",
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
            "suppliers.view",
            "purchases.view",
            "purchases.pay",
            "accounts.manage",
            "customers.collect_payment",
            "customers.reverse_payment",
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
            "suppliers.view",
            "purchases.view",
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
            "suppliers.view",
            "purchases.view",
            "purchases.pay",
            "purchases.return",
            "purchases.reverse_payment",
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
        let (role_id, created) = match existing {
            Some(id) => (id, false),
            None => {
                let id = Id::new().to_string();
                sqlx::query("INSERT INTO roles (id, name, description, is_system, created_at) VALUES (?, ?, ?, 1, ?)")
                    .bind(&id)
                    .bind(name)
                    .bind(desc)
                    .bind(time::now_utc_string())
                    .execute(&mut *tx)
                    .await?;
                (id, true)
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
            continue;
        }
        // Default grants: a new role gets all of them; an existing role gets only permissions
        // introduced after P1 and never one already handled (so admin removals stick).
        for k in *perms {
            let seen: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM default_grants WHERE role_name = ? AND perm_key = ?",
            )
            .bind(name)
            .bind(k)
            .fetch_one(&mut *tx)
            .await?;
            if seen > 0 {
                continue;
            }
            let since = CATALOGUE
                .iter()
                .find(|d| d.key == *k)
                .map(|d| d.since)
                .unwrap_or(1);
            if created || since >= 2 {
                sqlx::query(
                    "INSERT OR IGNORE INTO role_permissions (role_id, perm_key) VALUES (?, ?)",
                )
                .bind(&role_id)
                .bind(k)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query("INSERT OR IGNORE INTO default_grants (role_name, perm_key) VALUES (?, ?)")
                .bind(name)
                .bind(k)
                .execute(&mut *tx)
                .await?;
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
