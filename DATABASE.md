# Database

Migrations: `crates/core/migrations/*.sql`, embedded and applied at startup by sqlx. Never edit an applied migration; add a new one.

## 0001_foundation

| Table                | Purpose                                                                          |
| -------------------- | -------------------------------------------------------------------------------- |
| `branches`           | Branch concept; single default branch (partial unique index).                    |
| `app_settings`       | Key/value configuration (defaults seeded by `db::bootstrap`, never overwritten). |
| `audit_logs`         | Append-only (UPDATE/DELETE blocked by triggers).                                 |
| `document_sequences` | Prefix/padding/next number per document type.                                    |

Conventions: UUIDv7 TEXT ids, UTC RFC3339 timestamps, money as INTEGER paisa, FK enforced on every connection.
Health: `db::status` reports schema version, WAL, FK, `integrity_check`, `foreign_key_check`.

## 0002_users_roles_settings_backup

| Table                                                    | Purpose                                                                                                                                          |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `users`                                                  | Argon2id hash, active flag, must-change-password, failed attempts / lockout, last login. Unique case-insensitive username.                       |
| `roles`, `permissions`, `role_permissions`, `user_roles` | RBAC. `user_roles` uses `ON DELETE RESTRICT`: roles/users in use cannot be removed. System roles seeded once; Owner always has every permission. |
| `binary_assets`                                          | Shop logo (PNG/JPEG) so backups carry it.                                                                                                        |
| `backup_history`                                         | Backups, pre-restore copies, staged/applied/failed restores.                                                                                     |

`app_settings` is now a closed, validated catalogue (see `crates/core/src/settings.rs`).
Permissions catalogue (34 keys) lives in `crates/core/src/authz.rs` and is synchronised at startup.

## 0003_catalogue_inventory_purchasing

| Table                                                                                                                              | Purpose                                                                                                                                                                                  |
| ---------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `brands`, `categories` (2 levels), `products`, `product_variants`                                                                  | Editable catalogue; prices per channel (purchase cost, retail, wholesale, optional installment) on the variant. SKU and barcode unique (case-insensitive; empty barcode stored as NULL). |
| `product_serials`, `serial_events`                                                                                                 | One row per physical unit with status and cost; event log of every status change (append-only).                                                                                          |
| `stock_movements` (append-only), `stock_balances`                                                                                  | Ledger and its cached projection.                                                                                                                                                        |
| `stock_adjustments`                                                                                                                | Numbered documents (ADJ-) for opening stock, count adjustments, bucket moves, serial status changes.                                                                                     |
| `suppliers`, `supplier_ledger_entries` (append-only, unique per kind+document), `supplier_payments` (append-only, idempotency key) | Supplier master, payable ledger, payments and reversals.                                                                                                                                 |
| `purchases`, `purchase_items`, `purchase_costs`, `purchase_receipts(+items)`, `purchase_returns(+items)`                           | Purchase documents (PO-, PRN-). Partial unique index blocks duplicate supplier invoice numbers.                                                                                          |
| `default_grants`                                                                                                                   | Tracks default-role permission grants applied by upgrades.                                                                                                                               |
