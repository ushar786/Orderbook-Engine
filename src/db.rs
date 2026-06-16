use std::path::Path;

use rusqlite::{Connection, params};

use orderbook_engine::model::{Order, OrderAck, OrderStatus, Side, Trade};

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

            CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
            CREATE INDEX IF NOT EXISTS idx_trades_sequence ON trades(sequence);
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

    pub fn record_cancel(&mut self, order: &Order) -> rusqlite::Result<()> {
        upsert_order(&self.conn, order, OrderStatus::Cancelled)
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
            format!("{:?}", status).to_lowercase(),
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

fn side_to_db(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

fn side_from_db(value: &str) -> Side {
    match value {
        "sell" => Side::Sell,
        _ => Side::Buy,
    }
}
