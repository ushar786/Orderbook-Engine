use rusqlite::{Connection, params};

use orderbook_engine::model::{Order, OrderAck, OrderStatus, Side, Trade};

pub struct OrderRepository<'conn> {
    conn: &'conn mut Connection,
}

impl<'conn> OrderRepository<'conn> {
    pub fn new(conn: &'conn mut Connection) -> Self {
        Self { conn }
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
        upsert_order(self.conn, order, OrderStatus::Cancelled)
    }
}

pub struct TradeRepository<'conn> {
    conn: &'conn Connection,
}

impl<'conn> TradeRepository<'conn> {
    pub fn new(conn: &'conn Connection) -> Self {
        Self { conn }
    }

    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<Trade>> {
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
