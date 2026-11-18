-- P5: sales returns (with inspection), refunds, warranty cases, expenses, daily cash sessions.

-- A returned unit that passes inspection can be sold again, so a serial may appear on more than one
-- sale line over its life. At most one active sale per unit is enforced by the serial status graph.
DROP INDEX ux_sale_items_serial;
CREATE INDEX ix_sale_items_serial ON sale_items (serial_id) WHERE serial_id IS NOT NULL;

CREATE TABLE sale_returns (
    id            TEXT PRIMARY KEY,
    number        TEXT NOT NULL UNIQUE,
    sale_id       TEXT NOT NULL REFERENCES sales (id) ON DELETE RESTRICT,
    customer_id   TEXT REFERENCES customers (id) ON DELETE RESTRICT,
    status        TEXT NOT NULL DEFAULT 'requested' CHECK (status IN ('requested', 'completed', 'rejected')),
    return_date   TEXT NOT NULL,
    reason        TEXT NOT NULL,
    decision_note TEXT,
    client_key    TEXT UNIQUE,
    requested_by  TEXT,
    decided_by    TEXT,
    created_at    TEXT NOT NULL,
    decided_at    TEXT
);
CREATE INDEX ix_returns_sale ON sale_returns (sale_id);

CREATE TABLE sale_return_items (
    id           TEXT PRIMARY KEY,
    return_id    TEXT NOT NULL REFERENCES sale_returns (id) ON DELETE RESTRICT,
    sale_item_id TEXT NOT NULL REFERENCES sale_items (id) ON DELETE RESTRICT,
    variant_id   TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    serial_id    TEXT REFERENCES product_serials (id) ON DELETE RESTRICT,
    qty          INTEGER NOT NULL CHECK (qty > 0),
    value        INTEGER NOT NULL CHECK (value >= 0),   -- refundable value incl. tax
    tax          INTEGER NOT NULL CHECK (tax >= 0),
    cost_total   INTEGER NOT NULL CHECK (cost_total >= 0),
    outcome      TEXT CHECK (outcome IN ('restock', 'damaged', 'write_off')),
    note         TEXT
);
CREATE INDEX ix_return_items_return ON sale_return_items (return_id);
CREATE INDEX ix_return_items_sale_item ON sale_return_items (sale_item_id);

-- Money handed back to a customer. Append-only.
CREATE TABLE sale_refunds (
    id         TEXT PRIMARY KEY,
    return_id  TEXT NOT NULL REFERENCES sale_returns (id) ON DELETE RESTRICT,
    sale_id    TEXT NOT NULL REFERENCES sales (id) ON DELETE RESTRICT,
    receipt_no TEXT NOT NULL UNIQUE,
    amount     INTEGER NOT NULL CHECK (amount > 0),
    method     TEXT NOT NULL CHECK (method IN ('cash', 'bank', 'mobile_wallet', 'cheque', 'other')),
    paid_on    TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL
);
CREATE TRIGGER trg_refunds_no_update BEFORE UPDATE ON sale_refunds
BEGIN SELECT RAISE(ABORT, 'sale_refunds is append-only'); END;
CREATE TRIGGER trg_refunds_no_delete BEFORE DELETE ON sale_refunds
BEGIN SELECT RAISE(ABORT, 'sale_refunds is append-only'); END;

CREATE TABLE warranty_cases (
    id                  TEXT PRIMARY KEY,
    number              TEXT NOT NULL UNIQUE,
    customer_id         TEXT REFERENCES customers (id) ON DELETE RESTRICT,
    sale_id             TEXT REFERENCES sales (id) ON DELETE RESTRICT,
    variant_id          TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    serial_id           TEXT REFERENCES product_serials (id) ON DELETE RESTRICT,
    serial_no           TEXT NOT NULL DEFAULT '',
    purchased_on        TEXT,
    warranty_expires_on TEXT,
    in_warranty         INTEGER NOT NULL CHECK (in_warranty IN (0, 1)),
    issue               TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'in_service', 'resolved', 'rejected')),
    resolution          TEXT,
    opened_on           TEXT NOT NULL,
    closed_on           TEXT,
    created_by          TEXT,
    created_at          TEXT NOT NULL
);
CREATE INDEX ix_warranty_serial ON warranty_cases (serial_id);
CREATE INDEX ix_warranty_customer ON warranty_cases (customer_id);

CREATE TABLE warranty_events (
    id       TEXT PRIMARY KEY,
    case_id  TEXT NOT NULL REFERENCES warranty_cases (id) ON DELETE RESTRICT,
    at       TEXT NOT NULL,
    actor_id TEXT,
    status   TEXT NOT NULL,
    note     TEXT NOT NULL
);
CREATE TRIGGER trg_warranty_events_no_update BEFORE UPDATE ON warranty_events
BEGIN SELECT RAISE(ABORT, 'warranty_events is append-only'); END;
CREATE TRIGGER trg_warranty_events_no_delete BEFORE DELETE ON warranty_events
BEGIN SELECT RAISE(ABORT, 'warranty_events is append-only'); END;

CREATE TABLE expense_categories (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    account_code TEXT NOT NULL,
    is_active    INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at   TEXT NOT NULL
);

CREATE TABLE expenses (
    id             TEXT PRIMARY KEY,
    number         TEXT NOT NULL UNIQUE,
    category_id    TEXT NOT NULL REFERENCES expense_categories (id) ON DELETE RESTRICT,
    expense_date   TEXT NOT NULL,
    amount         INTEGER NOT NULL CHECK (amount > 0),
    method         TEXT NOT NULL CHECK (method IN ('cash', 'bank', 'mobile_wallet', 'cheque', 'other')),
    payee          TEXT NOT NULL DEFAULT '',
    description    TEXT NOT NULL DEFAULT '',
    status         TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected', 'voided')),
    client_key     TEXT UNIQUE,
    created_by     TEXT,
    created_at     TEXT NOT NULL,
    decided_by     TEXT,
    decided_at     TEXT,
    decision_note  TEXT,
    voided_at      TEXT,
    void_reason    TEXT
);
CREATE INDEX ix_expenses_date ON expenses (expense_date);
CREATE INDEX ix_expenses_status ON expenses (status);

CREATE TABLE cash_sessions (
    id                TEXT PRIMARY KEY,
    number            TEXT NOT NULL UNIQUE,
    status            TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed')),
    business_date     TEXT NOT NULL,
    opening_cash      INTEGER NOT NULL CHECK (opening_cash >= 0),
    opening_note      TEXT NOT NULL DEFAULT '',
    opened_by         TEXT,
    opened_at         TEXT NOT NULL,
    closed_by         TEXT,
    closed_at         TEXT,
    expected_cash     INTEGER,
    counted_cash      INTEGER,
    variance          INTEGER,
    close_note        TEXT
);
-- At most one open session (single-PC shop).
CREATE UNIQUE INDEX ux_cash_sessions_open ON cash_sessions (status) WHERE status = 'open';
CREATE TRIGGER trg_cash_sessions_closed_locked BEFORE UPDATE ON cash_sessions
WHEN OLD.status = 'closed'
BEGIN SELECT RAISE(ABORT, 'a closed cash session is locked'); END;
CREATE TRIGGER trg_cash_sessions_no_delete BEFORE DELETE ON cash_sessions
BEGIN SELECT RAISE(ABORT, 'cash sessions are never deleted'); END;
