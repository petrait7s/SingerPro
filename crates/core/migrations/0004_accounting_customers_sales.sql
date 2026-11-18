-- P3: double-entry accounting, customers, sales (retail + generic credit mechanics).

CREATE TABLE accounts (
    id          TEXT PRIMARY KEY,
    code        TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    type        TEXT NOT NULL CHECK (type IN ('asset', 'liability', 'equity', 'income', 'expense')),
    normal_side TEXT NOT NULL CHECK (normal_side IN ('debit', 'credit')),
    is_system   INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0, 1)),
    is_active   INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at  TEXT NOT NULL
);

-- Posted journals are immutable. posting_key makes posting idempotent (one entry per business event).
CREATE TABLE journal_entries (
    id          TEXT PRIMARY KEY,
    number      TEXT NOT NULL UNIQUE,
    entry_date  TEXT NOT NULL,
    memo        TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_id   TEXT NOT NULL,
    posting_key TEXT NOT NULL UNIQUE,
    reverses_id TEXT UNIQUE REFERENCES journal_entries (id) ON DELETE RESTRICT,
    actor_id    TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX ix_journal_date ON journal_entries (entry_date);
CREATE INDEX ix_journal_source ON journal_entries (source_type, source_id);

CREATE TABLE journal_lines (
    id         TEXT PRIMARY KEY,
    entry_id   TEXT NOT NULL REFERENCES journal_entries (id) ON DELETE RESTRICT,
    line_no    INTEGER NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    debit      INTEGER NOT NULL DEFAULT 0 CHECK (debit >= 0),
    credit     INTEGER NOT NULL DEFAULT 0 CHECK (credit >= 0),
    memo       TEXT NOT NULL DEFAULT '',
    CHECK ((debit = 0) <> (credit = 0)),
    UNIQUE (entry_id, line_no)
);
CREATE INDEX ix_journal_lines_account ON journal_lines (account_id);

CREATE TRIGGER trg_journal_entries_no_update BEFORE UPDATE ON journal_entries
BEGIN SELECT RAISE(ABORT, 'journal_entries is append-only; post a reversal'); END;
CREATE TRIGGER trg_journal_entries_no_delete BEFORE DELETE ON journal_entries
BEGIN SELECT RAISE(ABORT, 'journal_entries is append-only; post a reversal'); END;
CREATE TRIGGER trg_journal_lines_no_update BEFORE UPDATE ON journal_lines
BEGIN SELECT RAISE(ABORT, 'journal_lines is append-only; post a reversal'); END;
CREATE TRIGGER trg_journal_lines_no_delete BEFORE DELETE ON journal_lines
BEGIN SELECT RAISE(ABORT, 'journal_lines is append-only; post a reversal'); END;

CREATE TABLE customers (
    id            TEXT PRIMARY KEY,
    code          TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    phone         TEXT NOT NULL DEFAULT '',
    alt_phone     TEXT NOT NULL DEFAULT '',
    address       TEXT NOT NULL DEFAULT '',
    customer_type TEXT NOT NULL DEFAULT 'retail' CHECK (customer_type IN ('retail', 'wholesale', 'dealer')),
    credit_limit  INTEGER NOT NULL DEFAULT 0 CHECK (credit_limit >= 0),
    nid           TEXT NOT NULL DEFAULT '',
    notes         TEXT NOT NULL DEFAULT '',
    is_active     INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    registered_on TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX ix_customers_name ON customers (name COLLATE NOCASE);
CREATE INDEX ix_customers_phone ON customers (phone);

-- positive = the customer owes us. Append-only.
CREATE TABLE customer_ledger_entries (
    id          TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL REFERENCES customers (id) ON DELETE RESTRICT,
    at          TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('opening', 'sale', 'payment', 'payment_reversal', 'return', 'adjustment')),
    amount      INTEGER NOT NULL CHECK (amount <> 0),
    doc_type    TEXT NOT NULL,
    doc_id      TEXT NOT NULL,
    note        TEXT NOT NULL DEFAULT '',
    actor_id    TEXT
);
CREATE INDEX ix_customer_ledger ON customer_ledger_entries (customer_id, at);
CREATE UNIQUE INDEX ux_customer_ledger_doc ON customer_ledger_entries (kind, doc_type, doc_id);
CREATE TRIGGER trg_customer_ledger_no_update BEFORE UPDATE ON customer_ledger_entries
BEGIN SELECT RAISE(ABORT, 'customer_ledger_entries is append-only'); END;
CREATE TRIGGER trg_customer_ledger_no_delete BEFORE DELETE ON customer_ledger_entries
BEGIN SELECT RAISE(ABORT, 'customer_ledger_entries is append-only'); END;

CREATE TABLE sales (
    id             TEXT PRIMARY KEY,
    number         TEXT NOT NULL UNIQUE,
    mode           TEXT NOT NULL CHECK (mode IN ('retail', 'wholesale', 'installment')),
    customer_id    TEXT REFERENCES customers (id) ON DELETE RESTRICT,
    sale_date      TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'completed' CHECK (status IN ('completed')),
    subtotal       INTEGER NOT NULL,
    discount_total INTEGER NOT NULL DEFAULT 0,
    tax_total      INTEGER NOT NULL DEFAULT 0,
    grand_total    INTEGER NOT NULL,
    notes          TEXT NOT NULL DEFAULT '',
    client_key     TEXT UNIQUE,
    created_by     TEXT,
    created_at     TEXT NOT NULL
);
CREATE INDEX ix_sales_date ON sales (sale_date);
CREATE INDEX ix_sales_customer ON sales (customer_id, sale_date);

CREATE TABLE sale_items (
    id          TEXT PRIMARY KEY,
    sale_id     TEXT NOT NULL REFERENCES sales (id) ON DELETE RESTRICT,
    variant_id  TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    serial_id   TEXT REFERENCES product_serials (id) ON DELETE RESTRICT,
    qty         INTEGER NOT NULL CHECK (qty > 0),
    unit_price  INTEGER NOT NULL CHECK (unit_price >= 0),
    discount    INTEGER NOT NULL DEFAULT 0 CHECK (discount >= 0),
    tax         INTEGER NOT NULL DEFAULT 0 CHECK (tax >= 0),
    line_total  INTEGER NOT NULL,
    cost_total  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX ix_sale_items_sale ON sale_items (sale_id);
CREATE INDEX ix_sale_items_variant ON sale_items (variant_id);
CREATE UNIQUE INDEX ux_sale_items_serial ON sale_items (serial_id) WHERE serial_id IS NOT NULL;

-- Receipts. amount > 0 = money received; amount < 0 = reversal (links to original). Append-only.
CREATE TABLE sale_payments (
    id          TEXT PRIMARY KEY,
    sale_id     TEXT NOT NULL REFERENCES sales (id) ON DELETE RESTRICT,
    receipt_no  TEXT NOT NULL UNIQUE,
    method      TEXT NOT NULL CHECK (method IN ('cash', 'bank', 'mobile_wallet', 'cheque', 'other')),
    amount      INTEGER NOT NULL CHECK (amount <> 0),
    paid_on     TEXT NOT NULL,
    reference   TEXT NOT NULL DEFAULT '',
    note        TEXT NOT NULL DEFAULT '',
    reverses_id TEXT UNIQUE REFERENCES sale_payments (id) ON DELETE RESTRICT,
    client_key  TEXT UNIQUE,
    created_by  TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX ix_sale_payments_sale ON sale_payments (sale_id);
CREATE TRIGGER trg_sale_payments_no_update BEFORE UPDATE ON sale_payments
BEGIN SELECT RAISE(ABORT, 'sale_payments is append-only; use a reversal'); END;
CREATE TRIGGER trg_sale_payments_no_delete BEFORE DELETE ON sale_payments
BEGIN SELECT RAISE(ABORT, 'sale_payments is append-only; use a reversal'); END;
