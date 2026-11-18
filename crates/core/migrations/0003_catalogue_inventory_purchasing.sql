-- P2: catalogue, suppliers, purchasing, stock ledger and serial lifecycle.
-- Money = INTEGER paisa. Quantities = INTEGER units. Ledgers are append-only (triggers below).

CREATE TABLE brands (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    is_active  INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE TABLE categories (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL COLLATE NOCASE,
    parent_id  TEXT REFERENCES categories (id) ON DELETE RESTRICT,
    is_active  INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX ux_categories_name ON categories (COALESCE(parent_id, ''), name);

CREATE TABLE products (
    id              TEXT PRIMARY KEY,
    brand_id        TEXT NOT NULL REFERENCES brands (id) ON DELETE RESTRICT,
    category_id     TEXT NOT NULL REFERENCES categories (id) ON DELETE RESTRICT,
    name            TEXT NOT NULL,
    model           TEXT NOT NULL DEFAULT '',
    description     TEXT NOT NULL DEFAULT '',
    warranty_months INTEGER NOT NULL DEFAULT 0 CHECK (warranty_months >= 0),
    warranty_terms  TEXT NOT NULL DEFAULT '',
    min_stock       INTEGER NOT NULL DEFAULT 0 CHECK (min_stock >= 0),
    is_serialized   INTEGER NOT NULL DEFAULT 0 CHECK (is_serialized IN (0, 1)),
    is_active       INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX ix_products_brand ON products (brand_id);
CREATE INDEX ix_products_category ON products (category_id);
CREATE INDEX ix_products_name ON products (name COLLATE NOCASE);

CREATE TABLE product_variants (
    id                TEXT PRIMARY KEY,
    product_id        TEXT NOT NULL REFERENCES products (id) ON DELETE RESTRICT,
    sku               TEXT NOT NULL UNIQUE COLLATE NOCASE,
    barcode           TEXT UNIQUE COLLATE NOCASE,
    colour            TEXT NOT NULL DEFAULT '',
    capacity          TEXT NOT NULL DEFAULT '',
    size              TEXT NOT NULL DEFAULT '',
    purchase_cost     INTEGER NOT NULL DEFAULT 0 CHECK (purchase_cost >= 0),
    retail_price      INTEGER NOT NULL DEFAULT 0 CHECK (retail_price >= 0),
    wholesale_price   INTEGER NOT NULL DEFAULT 0 CHECK (wholesale_price >= 0),
    installment_price INTEGER CHECK (installment_price IS NULL OR installment_price >= 0),
    is_active         INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX ix_variants_product ON product_variants (product_id);

CREATE TABLE suppliers (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL UNIQUE COLLATE NOCASE,
    contact_person TEXT NOT NULL DEFAULT '',
    phone          TEXT NOT NULL DEFAULT '',
    alt_phone      TEXT NOT NULL DEFAULT '',
    email          TEXT NOT NULL DEFAULT '',
    address        TEXT NOT NULL DEFAULT '',
    notes          TEXT NOT NULL DEFAULT '',
    is_active      INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

-- Supplier ledger: positive = we owe the supplier more; negative = payments / credits. Append-only.
CREATE TABLE supplier_ledger_entries (
    id          TEXT PRIMARY KEY,
    supplier_id TEXT NOT NULL REFERENCES suppliers (id) ON DELETE RESTRICT,
    at          TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('purchase', 'purchase_cancel', 'payment', 'payment_reversal', 'return', 'adjustment')),
    amount      INTEGER NOT NULL CHECK (amount <> 0),
    doc_type    TEXT NOT NULL,
    doc_id      TEXT NOT NULL,
    note        TEXT NOT NULL DEFAULT '',
    actor_id    TEXT
);
CREATE INDEX ix_supplier_ledger ON supplier_ledger_entries (supplier_id, at);
CREATE UNIQUE INDEX ux_supplier_ledger_doc ON supplier_ledger_entries (kind, doc_type, doc_id);

CREATE TABLE purchases (
    id                  TEXT PRIMARY KEY,
    number              TEXT NOT NULL UNIQUE,
    supplier_id         TEXT NOT NULL REFERENCES suppliers (id) ON DELETE RESTRICT,
    supplier_invoice_no TEXT,
    order_date          TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'confirmed', 'cancelled')),
    discount            INTEGER NOT NULL DEFAULT 0 CHECK (discount >= 0),
    subtotal            INTEGER NOT NULL DEFAULT 0,
    landed_total        INTEGER NOT NULL DEFAULT 0,
    grand_total         INTEGER NOT NULL DEFAULT 0,
    notes               TEXT NOT NULL DEFAULT '',
    confirmed_at        TEXT,
    cancelled_at        TEXT,
    cancel_reason       TEXT,
    created_by          TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);
CREATE INDEX ix_purchases_supplier ON purchases (supplier_id, order_date);
-- Duplicate supplier invoice references are blocked per supplier (cancelled ones may be re-entered).
CREATE UNIQUE INDEX ux_purchases_supplier_invoice ON purchases (supplier_id, supplier_invoice_no COLLATE NOCASE)
    WHERE supplier_invoice_no IS NOT NULL AND status <> 'cancelled';

CREATE TABLE purchase_items (
    id           TEXT PRIMARY KEY,
    purchase_id  TEXT NOT NULL REFERENCES purchases (id) ON DELETE RESTRICT,
    variant_id   TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    qty          INTEGER NOT NULL CHECK (qty > 0),
    unit_cost    INTEGER NOT NULL CHECK (unit_cost >= 0),
    landed_alloc INTEGER NOT NULL DEFAULT 0, -- signed: share of (landed costs - discount), fixed at confirmation
    received_qty INTEGER NOT NULL DEFAULT 0,
    returned_qty INTEGER NOT NULL DEFAULT 0,
    CHECK (received_qty >= 0 AND received_qty <= qty),
    CHECK (returned_qty >= 0 AND returned_qty <= received_qty)
);
CREATE INDEX ix_purchase_items_purchase ON purchase_items (purchase_id);
CREATE INDEX ix_purchase_items_variant ON purchase_items (variant_id);

CREATE TABLE purchase_costs (
    id          TEXT PRIMARY KEY,
    purchase_id TEXT NOT NULL REFERENCES purchases (id) ON DELETE RESTRICT,
    label       TEXT NOT NULL,
    amount      INTEGER NOT NULL CHECK (amount > 0)
);
CREATE INDEX ix_purchase_costs_purchase ON purchase_costs (purchase_id);

CREATE TABLE purchase_receipts (
    id          TEXT PRIMARY KEY,
    purchase_id TEXT NOT NULL REFERENCES purchases (id) ON DELETE RESTRICT,
    received_on TEXT NOT NULL,
    note        TEXT NOT NULL DEFAULT '',
    actor_id    TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE purchase_receipt_items (
    id               TEXT PRIMARY KEY,
    receipt_id       TEXT NOT NULL REFERENCES purchase_receipts (id) ON DELETE RESTRICT,
    purchase_item_id TEXT NOT NULL REFERENCES purchase_items (id) ON DELETE RESTRICT,
    qty              INTEGER NOT NULL CHECK (qty > 0),
    cost_total       INTEGER NOT NULL
);

CREATE TABLE purchase_returns (
    id           TEXT PRIMARY KEY,
    number       TEXT NOT NULL UNIQUE,
    purchase_id  TEXT NOT NULL REFERENCES purchases (id) ON DELETE RESTRICT,
    returned_on  TEXT NOT NULL,
    reason       TEXT NOT NULL,
    credit_total INTEGER NOT NULL CHECK (credit_total >= 0),
    actor_id     TEXT,
    created_at   TEXT NOT NULL
);

CREATE TABLE purchase_return_items (
    id               TEXT PRIMARY KEY,
    return_id        TEXT NOT NULL REFERENCES purchase_returns (id) ON DELETE RESTRICT,
    purchase_item_id TEXT NOT NULL REFERENCES purchase_items (id) ON DELETE RESTRICT,
    qty              INTEGER NOT NULL CHECK (qty > 0),
    credit           INTEGER NOT NULL CHECK (credit >= 0),
    cost_total       INTEGER NOT NULL
);

-- amount > 0 = payment to supplier; amount < 0 = reversal (links to the original).
CREATE TABLE supplier_payments (
    id          TEXT PRIMARY KEY,
    supplier_id TEXT NOT NULL REFERENCES suppliers (id) ON DELETE RESTRICT,
    purchase_id TEXT REFERENCES purchases (id) ON DELETE RESTRICT,
    amount      INTEGER NOT NULL CHECK (amount <> 0),
    method      TEXT NOT NULL CHECK (method IN ('cash', 'bank', 'mobile_wallet', 'cheque', 'other')),
    paid_on     TEXT NOT NULL,
    reference   TEXT NOT NULL DEFAULT '',
    note        TEXT NOT NULL DEFAULT '',
    reverses_id TEXT REFERENCES supplier_payments (id) ON DELETE RESTRICT,
    client_key  TEXT UNIQUE,
    created_by  TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX ix_supplier_payments_supplier ON supplier_payments (supplier_id, paid_on);
CREATE UNIQUE INDEX ux_supplier_payment_reversal ON supplier_payments (reverses_id) WHERE reverses_id IS NOT NULL;

CREATE TABLE product_serials (
    id               TEXT PRIMARY KEY,
    variant_id       TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    serial_no        TEXT NOT NULL COLLATE NOCASE,
    status           TEXT NOT NULL CHECK (status IN (
        'available', 'reserved', 'sold', 'returned_pending_inspection',
        'damaged', 'warranty_service', 'written_off', 'returned_to_supplier')),
    cost             INTEGER NOT NULL DEFAULT 0,
    purchase_item_id TEXT REFERENCES purchase_items (id) ON DELETE RESTRICT,
    received_at      TEXT NOT NULL,
    UNIQUE (variant_id, serial_no)
);
CREATE INDEX ix_serials_status ON product_serials (variant_id, status);
CREATE INDEX ix_serials_no ON product_serials (serial_no);

CREATE TABLE serial_events (
    id          TEXT PRIMARY KEY,
    serial_id   TEXT NOT NULL REFERENCES product_serials (id) ON DELETE RESTRICT,
    at          TEXT NOT NULL,
    from_status TEXT,
    to_status   TEXT NOT NULL,
    doc_type    TEXT NOT NULL,
    doc_id      TEXT NOT NULL,
    reason      TEXT NOT NULL,
    actor_id    TEXT
);
CREATE INDEX ix_serial_events ON serial_events (serial_id, at);

-- Stock ledger. Immutable. qty_delta is signed; cost_total (paisa) is signed with it.
CREATE TABLE stock_movements (
    id            TEXT PRIMARY KEY,
    at            TEXT NOT NULL,
    variant_id    TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    bucket        TEXT NOT NULL CHECK (bucket IN ('available', 'reserved', 'damaged', 'warranty', 'pending_inspection')),
    qty_delta     INTEGER NOT NULL CHECK (qty_delta <> 0),
    cost_total    INTEGER NOT NULL DEFAULT 0,
    serial_id     TEXT REFERENCES product_serials (id) ON DELETE RESTRICT,
    doc_type      TEXT NOT NULL,
    doc_id        TEXT NOT NULL,
    reason        TEXT NOT NULL,
    actor_id      TEXT,
    balance_after INTEGER NOT NULL
);
CREATE INDEX ix_movements_variant ON stock_movements (variant_id, at);
CREATE INDEX ix_movements_doc ON stock_movements (doc_type, doc_id);

-- Cached projection of the ledger (reconciled by stock::reconcile).
CREATE TABLE stock_balances (
    variant_id TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    bucket     TEXT NOT NULL CHECK (bucket IN ('available', 'reserved', 'damaged', 'warranty', 'pending_inspection')),
    qty        INTEGER NOT NULL DEFAULT 0,
    value      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (variant_id, bucket)
);

CREATE TABLE stock_adjustments (
    id         TEXT PRIMARY KEY,
    number     TEXT NOT NULL UNIQUE,
    kind       TEXT NOT NULL CHECK (kind IN ('adjustment', 'opening', 'write_off', 'transfer', 'serial_status')),
    variant_id TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    summary    TEXT NOT NULL,
    reason     TEXT NOT NULL,
    at         TEXT NOT NULL,
    actor_id   TEXT
);
CREATE INDEX ix_adjustments_variant ON stock_adjustments (variant_id, at);

-- Immutability of ledgers.
CREATE TRIGGER trg_stock_movements_no_update BEFORE UPDATE ON stock_movements
BEGIN SELECT RAISE(ABORT, 'stock_movements is append-only'); END;
CREATE TRIGGER trg_stock_movements_no_delete BEFORE DELETE ON stock_movements
BEGIN SELECT RAISE(ABORT, 'stock_movements is append-only'); END;
CREATE TRIGGER trg_serial_events_no_update BEFORE UPDATE ON serial_events
BEGIN SELECT RAISE(ABORT, 'serial_events is append-only'); END;
CREATE TRIGGER trg_serial_events_no_delete BEFORE DELETE ON serial_events
BEGIN SELECT RAISE(ABORT, 'serial_events is append-only'); END;
CREATE TRIGGER trg_supplier_ledger_no_update BEFORE UPDATE ON supplier_ledger_entries
BEGIN SELECT RAISE(ABORT, 'supplier_ledger_entries is append-only'); END;
CREATE TRIGGER trg_supplier_ledger_no_delete BEFORE DELETE ON supplier_ledger_entries
BEGIN SELECT RAISE(ABORT, 'supplier_ledger_entries is append-only'); END;
CREATE TRIGGER trg_supplier_payments_no_update BEFORE UPDATE ON supplier_payments
BEGIN SELECT RAISE(ABORT, 'supplier_payments is append-only; use a reversal'); END;
CREATE TRIGGER trg_supplier_payments_no_delete BEFORE DELETE ON supplier_payments
BEGIN SELECT RAISE(ABORT, 'supplier_payments is append-only; use a reversal'); END;

-- Which default role grants have been applied, so permissions added in later releases reach the
-- default roles exactly once and never re-add something an administrator removed.
CREATE TABLE default_grants (
    role_name TEXT NOT NULL,
    perm_key  TEXT NOT NULL,
    PRIMARY KEY (role_name, perm_key)
);
