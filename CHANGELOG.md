# Changelog

## Unreleased — P6 dashboard, reports, exports, import

- Dashboard figures from real records: sales, cash and bank, receivables, payables, profit, stock value, low stock, overdue installments, approval queues, 14-day sales chart.
- 17 reports (sales by day/month/mode/product/brand/category, purchases, stock valuation, low stock, customer and supplier dues, installments due and collected, expenses, cash book, profit and loss, balance sheet) with A4 print, CSV and Excel export.
- CSV import for products and customers with a downloadable template and full pre-validation.
- 11 new Rust tests (140 total) and 6 new UI tests (89 total).

## Unreleased — P5 returns, warranty, expenses, cash close

- Returns with an inspection gate, credit notes, limited refunds and write-offs; returned units can be resold.
- Warranty lookup by serial number and service cases with a full event history.
- Expenses with approval, rejection and void; configurable categories.
- Daily cash sessions with computed expected cash, counted cash, variance posting and locking.
- Migration 0006. 13 new Rust tests (129 total) and 7 new UI tests (83 total).

## Unreleased — P4 wholesale and installments

- Wholesale tiers and customer-specific prices.
- Installment agreements: full disclosure, weekly/monthly/custom schedules, partial/advance collections, overdue tracking and worklist, early settlement with authorised rebate, rescheduling, explicit late fees, reversals, follow-ups.
- Markup recognition policy (proportional or at signing) with exact totals; reconciliation extended.
- 116 Rust tests, 77 UI tests.

## Unreleased — P3 accounting, customers, retail POS

- Double-entry accounting (chart, immutable idempotent journals, ledger, trial balance, manual journals, reversals, reconciliation, start-up back-fill).
- Customers with append-only ledger, NID off by default, credit limits.
- POS with server-computed totals, serial selection, split payments, change, A4/80mm printing; sales list, receipts, reversals, customer payments.
- 97 Rust tests, 68 UI tests.

## Unreleased — P2 catalogue, suppliers, purchasing, inventory

- Brands, categories, products with variants, per-channel prices, barcodes, images, serial tracking.
- Suppliers with append-only ledger; purchases (draft/confirm/receive/return), landed-cost allocation, idempotent payments and reversals.
- Immutable stock ledger, cached balances, serial lifecycle, opening stock, adjustments, bucket moves, reconciliation.
- Products, Inventory, Suppliers and Purchases screens. 76 Rust tests, 55 UI tests.
- Restore hardened for Windows file locks.

## Unreleased — P1 users, settings, backup

- First-run Owner setup, Argon2id, lockout, in-memory sessions held in Rust.
- RBAC with 34 permissions and 7 system roles, enforced in core services; privilege-escalation guards.
- Typed, audited settings; logo upload; EN/বাংলা UI with bundled fonts.
- Verified backups with manifest; staged, reversible restore applied at startup.
- 56 Rust + 32 UI tests.

## 0.1.0 — P0 foundation

- Cargo workspace: `singerpro-core` + Tauri 2 shell; React/TS/Tailwind UI shell with bundled fonts.
- Money, UUIDv7, Dhaka time, SQLite (WAL, FK), migration 0001, audit trail, document sequences.
- CI workflow for Windows NSIS installer.
