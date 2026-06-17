use std::path::Path;

use rusqlite::{Connection, params};

use crate::model::{
    EngineEvent, Order, OrderAck, OrderHistoryEntry, OrderId, OrderStatus, Side, Trade,
};

#[derive(Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            r#"
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

            CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
            CREATE INDEX IF NOT EXISTS idx_trades_sequence ON trades(sequence);
            CREATE INDEX IF NOT EXISTS idx_order_history_order_id ON order_history(order_id);
            CREATE INDEX IF NOT EXISTS idx_event_journal_sequence ON event_journal(engine_sequence);
            "#,
        )?;
        Ok(Self { conn })
    }

    pub fn record_ack(&mut self, ack: &OrderAck) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        upsert_order(&tx, &ack.order, ack.status)?;
        for trade in &ack.trades {
            insert_trade(&tx, trade)?;
        }
        tx.commit()
    }

    pub fn record_order_history(
        &mut self,
        order_id: OrderId,
        history: &[OrderHistoryEntry],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for entry in history {
            insert_order_history(&tx, order_id, entry)?;
        }
        tx.commit()
    }

    pub fn record_events(&mut self, events: &[EngineEvent]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for event in events {
            insert_event(&tx, event)?;
        }
        tx.commit()
    }

    pub fn record_cancel(&mut self, order: &Order) -> rusqlite::Result<()> {
        upsert_order(&self.conn, order, OrderStatus::Cancelled)
    }

    pub fn order_history(&self, order_id: OrderId) -> rusqlite::Result<Vec<OrderHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT sequence, status, remaining_quantity
            FROM order_history
            WHERE order_id = ?1
            ORDER BY sequence ASC, created_at ASC
            "#,
        )?;
        let rows = stmt.query_map([order_id], |row| {
            Ok(OrderHistoryEntry {
                sequence: row.get(0)?,
                status: status_from_db(row.get::<_, String>(1)?.as_str()),
                remaining_quantity: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn recent_trades(&self, limit: usize) -> rusqlite::Result<Vec<Trade>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, maker_order_id, taker_order_id, price, quantity, aggressor_side, sequence
            FROM trades
            ORDER BY sequence DESC
            LIMIT ?1
            "#,
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            Ok(Trade {
                id: row.get(0)?,
                maker_order_id: row.get(1)?,
                taker_order_id: row.get(2)?,
                price: row.get(3)?,
                quantity: row.get(4)?,
                aggressor_side: side_from_db(row.get::<_, String>(5)?.as_str()),
                sequence: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn event_count(&self) -> rusqlite::Result<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM event_journal", [], |row| row.get(0))
    }
}

fn upsert_order(conn: &Connection, order: &Order, status: OrderStatus) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT INTO orders (
            id, side, type, price, original_quantity, remaining_quantity, status, created_at_seq
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(id) DO UPDATE SET
            remaining_quantity = excluded.remaining_quantity,
            status = excluded.status,
            updated_at = CURRENT_TIMESTAMP
        "#,
        params![
            order.id,
            side_to_db(order.side),
            format!("{:?}", order.kind).to_lowercase(),
            order.price,
            order.original_quantity,
            order.remaining_quantity,
            status_to_db(status),
            order.created_at_seq,
        ],
    )?;
    Ok(())
}

fn insert_trade(conn: &Connection, trade: &Trade) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT OR IGNORE INTO trades (
            id, maker_order_id, taker_order_id, price, quantity, aggressor_side, sequence
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
        params![
            trade.id,
            trade.maker_order_id,
            trade.taker_order_id,
            trade.price,
            trade.quantity,
            side_to_db(trade.aggressor_side),
            trade.sequence,
        ],
    )?;
    Ok(())
}

fn insert_order_history(
    conn: &Connection,
    order_id: OrderId,
    entry: &OrderHistoryEntry,
) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT OR IGNORE INTO order_history (
            order_id, sequence, status, remaining_quantity
        )
        VALUES (?1, ?2, ?3, ?4)
        "#,
        params![
            order_id,
            entry.sequence,
            status_to_db(entry.status),
            entry.remaining_quantity,
        ],
    )?;
    Ok(())
}

fn insert_event(conn: &Connection, event: &EngineEvent) -> rusqlite::Result<()> {
    let payload = serde_json::to_string(event)
        .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
    conn.execute(
        r#"
        INSERT INTO event_journal (
            engine_sequence, event_type, payload
        )
        VALUES (?1, ?2, ?3)
        "#,
        params![event_sequence(event), event_type(event), payload],
    )?;
    Ok(())
}

fn event_sequence(event: &EngineEvent) -> Option<u64> {
    match event {
        EngineEvent::Book { data } => Some(data.sequence),
        EngineEvent::Order { data } => Some(data.order.created_at_seq),
        EngineEvent::Trade { data } => Some(data.sequence),
        EngineEvent::Cancel { .. } => None,
    }
}

fn event_type(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::Book { .. } => "book",
        EngineEvent::Order { .. } => "order",
        EngineEvent::Trade { .. } => "trade",
        EngineEvent::Cancel { .. } => "cancel",
    }
}

fn side_to_db(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn status_to_db(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::Accepted => "accepted",
        OrderStatus::Filled => "filled",
        OrderStatus::PartiallyFilled => "partially_filled",
        OrderStatus::Resting => "resting",
        OrderStatus::Rejected => "rejected",
        OrderStatus::Cancelled => "cancelled",
    }
}

fn status_from_db(value: &str) -> OrderStatus {
    match value {
        "accepted" => OrderStatus::Accepted,
        "filled" => OrderStatus::Filled,
        "partially_filled" => OrderStatus::PartiallyFilled,
        "cancelled" => OrderStatus::Cancelled,
        "rejected" => OrderStatus::Rejected,
        _ => OrderStatus::Resting,
    }
}

fn side_from_db(value: &str) -> Side {
    match value {
        "sell" => Side::Sell,
        _ => Side::Buy,
    }
}
