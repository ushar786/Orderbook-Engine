CREATE TABLE IF NOT EXISTS orders (
    id BIGINT PRIMARY KEY,
    account_id TEXT NOT NULL DEFAULT '',
    side TEXT NOT NULL,
    type TEXT NOT NULL,
    time_in_force TEXT NOT NULL DEFAULT 'gtc',
    price BIGINT,
    original_quantity BIGINT NOT NULL,
    remaining_quantity BIGINT NOT NULL,
    original_quote_quantity BIGINT,
    remaining_quote_quantity BIGINT,
    status TEXT NOT NULL,
    created_at_seq BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS trades (
    id BIGINT PRIMARY KEY,
    maker_order_id BIGINT NOT NULL,
    taker_order_id BIGINT NOT NULL,
    price BIGINT NOT NULL,
    quantity BIGINT NOT NULL,
    aggressor_side TEXT NOT NULL,
    sequence BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS order_history (
    order_id BIGINT NOT NULL,
    sequence BIGINT NOT NULL,
    status TEXT NOT NULL,
    remaining_quantity BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (order_id, sequence, status, remaining_quantity)
);

CREATE TABLE IF NOT EXISTS event_journal (
    id BIGSERIAL PRIMARY KEY,
    engine_sequence BIGINT,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS engine_snapshots (
    id BIGSERIAL PRIMARY KEY,
    engine_sequence BIGINT NOT NULL,
    event_journal_id BIGINT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
CREATE INDEX IF NOT EXISTS idx_trades_sequence ON trades(sequence);
CREATE INDEX IF NOT EXISTS idx_order_history_order_id ON order_history(order_id);
CREATE INDEX IF NOT EXISTS idx_event_journal_sequence ON event_journal(engine_sequence);
CREATE INDEX IF NOT EXISTS idx_engine_snapshots_event_journal_id ON engine_snapshots(event_journal_id);
