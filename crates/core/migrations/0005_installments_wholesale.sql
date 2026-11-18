-- P4: wholesale pricing (tiers, customer-specific prices) and installment agreements.

CREATE TABLE price_tiers (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    -- basis points applied to the wholesale price: -300 = 3% cheaper, +200 = 2% dearer
    adjust_bp  INTEGER NOT NULL CHECK (adjust_bp BETWEEN -5000 AND 5000),
    is_active  INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at TEXT NOT NULL
);

ALTER TABLE customers ADD COLUMN price_tier_id TEXT REFERENCES price_tiers (id);

CREATE TABLE customer_prices (
    customer_id TEXT NOT NULL REFERENCES customers (id) ON DELETE RESTRICT,
    variant_id  TEXT NOT NULL REFERENCES product_variants (id) ON DELETE RESTRICT,
    price       INTEGER NOT NULL CHECK (price >= 0),
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (customer_id, variant_id)
);

CREATE TABLE installment_agreements (
    id                 TEXT PRIMARY KEY,
    number             TEXT NOT NULL UNIQUE,
    sale_id            TEXT NOT NULL UNIQUE REFERENCES sales (id) ON DELETE RESTRICT,
    customer_id        TEXT NOT NULL REFERENCES customers (id) ON DELETE RESTRICT,
    status             TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'completed')),
    cash_price         INTEGER NOT NULL,   -- what the same goods cost in cash (retail list price, net of discount)
    price_total        INTEGER NOT NULL,   -- invoice total of the goods incl. tax (installment price basis)
    down_payment       INTEGER NOT NULL CHECK (down_payment >= 0),
    principal          INTEGER NOT NULL CHECK (principal > 0),
    markup_bp          INTEGER NOT NULL CHECK (markup_bp >= 0),
    markup             INTEGER NOT NULL CHECK (markup >= 0),
    fees_total         INTEGER NOT NULL DEFAULT 0 CHECK (fees_total >= 0),
    total_payable      INTEGER NOT NULL CHECK (total_payable > 0), -- principal + markup + fees, collected over the schedule
    frequency          TEXT NOT NULL CHECK (frequency IN ('weekly', 'monthly', 'custom')),
    installment_count  INTEGER NOT NULL CHECK (installment_count BETWEEN 1 AND 120),
    first_due_date     TEXT NOT NULL,
    recognition_policy TEXT NOT NULL CHECK (recognition_policy IN ('proportional_to_collections', 'on_agreement')),
    notes              TEXT NOT NULL DEFAULT '',
    created_by         TEXT,
    created_at         TEXT NOT NULL
);
CREATE INDEX ix_agreements_customer ON installment_agreements (customer_id);

CREATE TABLE installment_charges (
    id           TEXT PRIMARY KEY,
    agreement_id TEXT NOT NULL REFERENCES installment_agreements (id) ON DELETE RESTRICT,
    label        TEXT NOT NULL,
    amount       INTEGER NOT NULL CHECK (amount > 0)
);

CREATE TABLE installment_schedule (
    id            TEXT PRIMARY KEY,
    agreement_id  TEXT NOT NULL REFERENCES installment_agreements (id) ON DELETE RESTRICT,
    seq           INTEGER NOT NULL,
    due_date      TEXT NOT NULL,
    amount        INTEGER NOT NULL CHECK (amount > 0),
    superseded    INTEGER NOT NULL DEFAULT 0 CHECK (superseded IN (0, 1)),
    reschedule_id TEXT,
    UNIQUE (agreement_id, seq)
);
CREATE INDEX ix_schedule_due ON installment_schedule (due_date);

-- kind 'rebate' = authorised early-settlement reduction (no cash). amount < 0 = reversal of an earlier row.
CREATE TABLE installment_collections (
    id           TEXT PRIMARY KEY,
    agreement_id TEXT NOT NULL REFERENCES installment_agreements (id) ON DELETE RESTRICT,
    receipt_no   TEXT NOT NULL UNIQUE,
    kind         TEXT NOT NULL DEFAULT 'payment' CHECK (kind IN ('payment', 'rebate')),
    amount       INTEGER NOT NULL CHECK (amount <> 0),
    method       TEXT NOT NULL CHECK (method IN ('cash', 'bank', 'mobile_wallet', 'cheque', 'other', 'rebate')),
    paid_on      TEXT NOT NULL,
    reference    TEXT NOT NULL DEFAULT '',
    note         TEXT NOT NULL DEFAULT '',
    reverses_id  TEXT UNIQUE REFERENCES installment_collections (id) ON DELETE RESTRICT,
    client_key   TEXT UNIQUE,
    created_by   TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX ix_collections_agreement ON installment_collections (agreement_id);

CREATE TABLE collection_allocations (
    id            TEXT PRIMARY KEY,
    collection_id TEXT NOT NULL REFERENCES installment_collections (id) ON DELETE RESTRICT,
    schedule_id   TEXT NOT NULL REFERENCES installment_schedule (id) ON DELETE RESTRICT,
    amount        INTEGER NOT NULL CHECK (amount <> 0)
);
CREATE INDEX ix_alloc_schedule ON collection_allocations (schedule_id);

CREATE TABLE installment_late_fees (
    id          TEXT PRIMARY KEY,
    agreement_id TEXT NOT NULL REFERENCES installment_agreements (id) ON DELETE RESTRICT,
    schedule_id TEXT NOT NULL UNIQUE REFERENCES installment_schedule (id) ON DELETE RESTRICT,
    amount      INTEGER NOT NULL CHECK (amount > 0),
    reason      TEXT NOT NULL,
    created_by  TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE installment_reschedules (
    id           TEXT PRIMARY KEY,
    agreement_id TEXT NOT NULL REFERENCES installment_agreements (id) ON DELETE RESTRICT,
    at           TEXT NOT NULL,
    actor_id     TEXT,
    reason       TEXT NOT NULL,
    outstanding  INTEGER NOT NULL,
    old_count    INTEGER NOT NULL,
    new_count    INTEGER NOT NULL
);

CREATE TABLE installment_followups (
    id           TEXT PRIMARY KEY,
    agreement_id TEXT NOT NULL REFERENCES installment_agreements (id) ON DELETE RESTRICT,
    at           TEXT NOT NULL,
    actor_id     TEXT,
    note         TEXT NOT NULL,
    promise_date TEXT
);

CREATE TRIGGER trg_collections_no_update BEFORE UPDATE ON installment_collections
BEGIN SELECT RAISE(ABORT, 'installment_collections is append-only; use a reversal'); END;
CREATE TRIGGER trg_collections_no_delete BEFORE DELETE ON installment_collections
BEGIN SELECT RAISE(ABORT, 'installment_collections is append-only; use a reversal'); END;
CREATE TRIGGER trg_alloc_no_update BEFORE UPDATE ON collection_allocations
BEGIN SELECT RAISE(ABORT, 'collection_allocations is append-only'); END;
CREATE TRIGGER trg_alloc_no_delete BEFORE DELETE ON collection_allocations
BEGIN SELECT RAISE(ABORT, 'collection_allocations is append-only'); END;
CREATE TRIGGER trg_late_fees_no_update BEFORE UPDATE ON installment_late_fees
BEGIN SELECT RAISE(ABORT, 'installment_late_fees is append-only'); END;
CREATE TRIGGER trg_late_fees_no_delete BEFORE DELETE ON installment_late_fees
BEGIN SELECT RAISE(ABORT, 'installment_late_fees is append-only'); END;
-- Schedule rows may only be marked superseded (reschedule); amounts and dates never change.
CREATE TRIGGER trg_schedule_immutable BEFORE UPDATE ON installment_schedule
WHEN NEW.amount <> OLD.amount OR NEW.due_date <> OLD.due_date OR NEW.seq <> OLD.seq OR NEW.agreement_id <> OLD.agreement_id
BEGIN SELECT RAISE(ABORT, 'schedule amounts and dates are immutable; reschedule instead'); END;
CREATE TRIGGER trg_schedule_no_delete BEFORE DELETE ON installment_schedule
BEGIN SELECT RAISE(ABORT, 'schedule rows are never deleted'); END;
