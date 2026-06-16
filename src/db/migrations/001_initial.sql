CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY,
    side TEXT NOT NULL,
    type TEXT NOT NULL,
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

CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
CREATE INDEX IF NOT EXISTS idx_trades_sequence ON trades(sequence);
