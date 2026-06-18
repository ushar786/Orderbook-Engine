CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL DEFAULT '',
    side TEXT NOT NULL,
    type TEXT NOT NULL,
    time_in_force TEXT NOT NULL DEFAULT 'gtc',
    price INTEGER,
    original_quantity INTEGER NOT NULL,
    remaining_quantity INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at_seq INTEGER NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS trades (
    id INTEGER PRIMARY KEY,
    maker_order_id INTEGER NOT NULL,
    taker_order_id INTEGER NOT NULL,
    price INTEGER NOT NULL,
    quantity INTEGER NOT NULL,
    aggressor_side TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS order_history (
    order_id INTEGER NOT NULL,
    sequence INTEGER NOT NULL,
    status TEXT NOT NULL,
    remaining_quantity INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (order_id, sequence, status, remaining_quantity)
);

CREATE TABLE IF NOT EXISTS event_journal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    engine_sequence INTEGER,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS engine_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    engine_sequence INTEGER NOT NULL,
    event_journal_id INTEGER NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
CREATE INDEX IF NOT EXISTS idx_trades_sequence ON trades(sequence);
CREATE INDEX IF NOT EXISTS idx_order_history_order_id ON order_history(order_id);
CREATE INDEX IF NOT EXISTS idx_event_journal_sequence ON event_journal(engine_sequence);
CREATE INDEX IF NOT EXISTS idx_engine_snapshots_event_journal_id ON engine_snapshots(event_journal_id);
