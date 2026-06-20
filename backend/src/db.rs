use std::{fmt, path::Path};

use postgres::{Client, NoTls};
use postgres_types::Json;
use rusqlite::{Connection, params};
use thiserror::Error;

use crate::model::{
    EngineEvent, EngineSnapshot, Order, OrderAck, OrderHistoryEntry, OrderId, OrderStatus, Side,
    SnapshotCheckpoint, TimeInForce, Trade,
};

#[derive(Debug)]
pub enum Database {
    Sqlite(SqliteDatabase),
    Postgres(PostgresDatabase),
}

impl Database {
    pub fn open(source: impl AsRef<Path>) -> Result<Self, DbError> {
        let source = source.as_ref().to_string_lossy().to_string();
        if source.starts_with("postgres://") || source.starts_with("postgresql://") {
            return Ok(Self::Postgres(PostgresDatabase::open(&source)?));
        }
        Ok(Self::Sqlite(SqliteDatabase::open(source)?))
    }

    pub fn record_ack(&mut self, ack: &OrderAck) -> Result<(), DbError> {
        match self {
            Self::Sqlite(db) => db.record_ack(ack),
            Self::Postgres(db) => db.record_ack(ack),
        }
    }

    pub fn record_order_history(
        &mut self,
        order_id: OrderId,
        history: &[OrderHistoryEntry],
    ) -> Result<(), DbError> {
        match self {
            Self::Sqlite(db) => db.record_order_history(order_id, history),
            Self::Postgres(db) => db.record_order_history(order_id, history),
        }
    }

    pub fn record_events(&mut self, events: &[EngineEvent]) -> Result<(), DbError> {
        match self {
            Self::Sqlite(db) => db.record_events(events),
            Self::Postgres(db) => db.record_events(events),
        }
    }

    pub fn record_cancel(&mut self, order: &Order) -> Result<(), DbError> {
        match self {
            Self::Sqlite(db) => db.record_cancel(order),
            Self::Postgres(db) => db.record_cancel(order),
        }
    }

    pub fn order_history(&mut self, order_id: OrderId) -> Result<Vec<OrderHistoryEntry>, DbError> {
        match self {
            Self::Sqlite(db) => db.order_history(order_id),
            Self::Postgres(db) => db.order_history(order_id),
        }
    }

    pub fn recent_trades(&mut self, limit: usize) -> Result<Vec<Trade>, DbError> {
        match self {
            Self::Sqlite(db) => db.recent_trades(limit),
            Self::Postgres(db) => db.recent_trades(limit),
        }
    }

    pub fn event_count(&mut self) -> Result<u64, DbError> {
        match self {
            Self::Sqlite(db) => db.event_count(),
            Self::Postgres(db) => db.event_count(),
        }
    }

    pub fn events(&mut self) -> Result<Vec<EngineEvent>, DbError> {
        match self {
            Self::Sqlite(db) => db.events(),
            Self::Postgres(db) => db.events(),
        }
    }

    pub fn events_after_id(&mut self, event_journal_id: u64) -> Result<Vec<EngineEvent>, DbError> {
        match self {
            Self::Sqlite(db) => db.events_after_id(event_journal_id),
            Self::Postgres(db) => db.events_after_id(event_journal_id),
        }
    }

    pub fn latest_event_journal_id(&mut self) -> Result<u64, DbError> {
        match self {
            Self::Sqlite(db) => db.latest_event_journal_id(),
            Self::Postgres(db) => db.latest_event_journal_id(),
        }
    }

    pub fn record_snapshot(&mut self, checkpoint: &SnapshotCheckpoint) -> Result<(), DbError> {
        match self {
            Self::Sqlite(db) => db.record_snapshot(checkpoint),
            Self::Postgres(db) => db.record_snapshot(checkpoint),
        }
    }

    pub fn latest_snapshot(&mut self) -> Result<Option<SnapshotCheckpoint>, DbError> {
        match self {
            Self::Sqlite(db) => db.latest_snapshot(),
            Self::Postgres(db) => db.latest_snapshot(),
        }
    }
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Postgres(#[from] postgres::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct SqliteDatabase {
    conn: Connection,
}

impl SqliteDatabase {
    fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SQLITE_SCHEMA)?;
        sqlite_add_column_if_missing(
            &conn,
            "ALTER TABLE orders ADD COLUMN time_in_force TEXT NOT NULL DEFAULT 'gtc'",
        )?;
        sqlite_add_column_if_missing(
            &conn,
            "ALTER TABLE orders ADD COLUMN account_id TEXT NOT NULL DEFAULT ''",
        )?;
        sqlite_add_column_if_missing(
            &conn,
            "ALTER TABLE orders ADD COLUMN original_quote_quantity INTEGER",
        )?;
        sqlite_add_column_if_missing(
            &conn,
            "ALTER TABLE orders ADD COLUMN remaining_quote_quantity INTEGER",
        )?;
        sqlite_add_column_if_missing(&conn, "ALTER TABLE orders ADD COLUMN stop_price INTEGER")?;
        Ok(Self { conn })
    }

    fn record_ack(&mut self, ack: &OrderAck) -> Result<(), DbError> {
        let tx = self.conn.transaction()?;
        sqlite_upsert_order(&tx, &ack.order, ack.status)?;
        for trade in &ack.trades {
            sqlite_insert_trade(&tx, trade)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn record_order_history(
        &mut self,
        order_id: OrderId,
        history: &[OrderHistoryEntry],
    ) -> Result<(), DbError> {
        let tx = self.conn.transaction()?;
        for entry in history {
            sqlite_insert_order_history(&tx, order_id, entry)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn record_events(&mut self, events: &[EngineEvent]) -> Result<(), DbError> {
        let tx = self.conn.transaction()?;
        for event in events {
            sqlite_insert_event(&tx, event)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn record_cancel(&mut self, order: &Order) -> Result<(), DbError> {
        sqlite_upsert_order(&self.conn, order, OrderStatus::Cancelled)?;
        Ok(())
    }

    fn order_history(&mut self, order_id: OrderId) -> Result<Vec<OrderHistoryEntry>, DbError> {
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
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn recent_trades(&mut self, limit: usize) -> Result<Vec<Trade>, DbError> {
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
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn event_count(&mut self) -> Result<u64, DbError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM event_journal", [], |row| row.get(0))?)
    }

    fn events(&mut self) -> Result<Vec<EngineEvent>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM event_journal ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let payloads = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        payloads
            .into_iter()
            .map(|payload| Ok(serde_json::from_str(&payload)?))
            .collect()
    }

    fn events_after_id(&mut self, event_journal_id: u64) -> Result<Vec<EngineEvent>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM event_journal WHERE id > ?1 ORDER BY id ASC")?;
        let rows = stmt.query_map([event_journal_id], |row| row.get::<_, String>(0))?;
        let payloads = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        payloads
            .into_iter()
            .map(|payload| Ok(serde_json::from_str(&payload)?))
            .collect()
    }

    fn latest_event_journal_id(&mut self) -> Result<u64, DbError> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM event_journal",
            [],
            |row| row.get(0),
        )?)
    }

    fn record_snapshot(&mut self, checkpoint: &SnapshotCheckpoint) -> Result<(), DbError> {
        let payload = serde_json::to_string(&checkpoint.snapshot)?;
        self.conn.execute(
            r#"
            INSERT INTO engine_snapshots (engine_sequence, event_journal_id, payload)
            VALUES (?1, ?2, ?3)
            "#,
            params![
                checkpoint.snapshot.sequence,
                checkpoint.event_journal_id,
                payload,
            ],
        )?;
        Ok(())
    }

    fn latest_snapshot(&mut self) -> Result<Option<SnapshotCheckpoint>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT event_journal_id, payload
            FROM engine_snapshots
            ORDER BY id DESC
            LIMIT 1
            "#,
        )?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let event_journal_id = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok(Some(SnapshotCheckpoint {
            event_journal_id,
            snapshot: serde_json::from_str(&payload)?,
        }))
    }
}

pub struct PostgresDatabase {
    client: Option<Client>,
}

impl fmt::Debug for PostgresDatabase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresDatabase")
            .field("client", &"postgres::Client")
            .finish()
    }
}

impl PostgresDatabase {
    fn open(url: &str) -> Result<Self, DbError> {
        let mut client = Client::connect(url, NoTls)?;
        client.batch_execute(POSTGRES_SCHEMA)?;
        client.batch_execute(
            r#"
            ALTER TABLE orders ADD COLUMN IF NOT EXISTS time_in_force TEXT NOT NULL DEFAULT 'gtc';
            ALTER TABLE orders ADD COLUMN IF NOT EXISTS account_id TEXT NOT NULL DEFAULT '';
            ALTER TABLE orders ADD COLUMN IF NOT EXISTS original_quote_quantity BIGINT;
            ALTER TABLE orders ADD COLUMN IF NOT EXISTS remaining_quote_quantity BIGINT;
            ALTER TABLE orders ADD COLUMN IF NOT EXISTS stop_price BIGINT;
            "#,
        )?;
        Ok(Self {
            client: Some(client),
        })
    }

    fn record_ack(&mut self, ack: &OrderAck) -> Result<(), DbError> {
        let mut tx = self.client().transaction()?;
        postgres_upsert_order(&mut tx, &ack.order, ack.status)?;
        for trade in &ack.trades {
            postgres_insert_trade(&mut tx, trade)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn record_order_history(
        &mut self,
        order_id: OrderId,
        history: &[OrderHistoryEntry],
    ) -> Result<(), DbError> {
        let mut tx = self.client().transaction()?;
        for entry in history {
            postgres_insert_order_history(&mut tx, order_id, entry)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn record_events(&mut self, events: &[EngineEvent]) -> Result<(), DbError> {
        let mut tx = self.client().transaction()?;
        for event in events {
            postgres_insert_event(&mut tx, event)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn record_cancel(&mut self, order: &Order) -> Result<(), DbError> {
        postgres_upsert_order(self.client(), order, OrderStatus::Cancelled)?;
        Ok(())
    }

    fn order_history(&mut self, order_id: OrderId) -> Result<Vec<OrderHistoryEntry>, DbError> {
        let rows = self.client().query(
            r#"
            SELECT sequence, status, remaining_quantity
            FROM order_history
            WHERE order_id = $1
            ORDER BY sequence ASC, created_at ASC
            "#,
            &[&to_i64(order_id)],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| OrderHistoryEntry {
                sequence: from_i64(row.get(0)),
                status: status_from_db(row.get::<_, String>(1).as_str()),
                remaining_quantity: from_i64(row.get(2)),
            })
            .collect())
    }

    fn recent_trades(&mut self, limit: usize) -> Result<Vec<Trade>, DbError> {
        let rows = self.client().query(
            r#"
            SELECT id, maker_order_id, taker_order_id, price, quantity, aggressor_side, sequence
            FROM trades
            ORDER BY sequence DESC
            LIMIT $1
            "#,
            &[&(limit as i64)],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| Trade {
                id: from_i64(row.get(0)),
                maker_order_id: from_i64(row.get(1)),
                taker_order_id: from_i64(row.get(2)),
                price: from_i64(row.get(3)),
                quantity: from_i64(row.get(4)),
                aggressor_side: side_from_db(row.get::<_, String>(5).as_str()),
                sequence: from_i64(row.get(6)),
            })
            .collect())
    }

    fn event_count(&mut self) -> Result<u64, DbError> {
        let row = self
            .client()
            .query_one("SELECT COUNT(*) FROM event_journal", &[])?;
        Ok(from_i64(row.get(0)))
    }

    fn events(&mut self) -> Result<Vec<EngineEvent>, DbError> {
        let rows = self
            .client()
            .query("SELECT payload FROM event_journal ORDER BY id ASC", &[])?;
        Ok(rows
            .into_iter()
            .map(|row| row.get::<_, Json<EngineEvent>>(0).0)
            .collect())
    }

    fn events_after_id(&mut self, event_journal_id: u64) -> Result<Vec<EngineEvent>, DbError> {
        let rows = self.client().query(
            "SELECT payload FROM event_journal WHERE id > $1 ORDER BY id ASC",
            &[&to_i64(event_journal_id)],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| row.get::<_, Json<EngineEvent>>(0).0)
            .collect())
    }

    fn latest_event_journal_id(&mut self) -> Result<u64, DbError> {
        let row = self
            .client()
            .query_one("SELECT COALESCE(MAX(id), 0) FROM event_journal", &[])?;
        Ok(from_i64(row.get(0)))
    }

    fn record_snapshot(&mut self, checkpoint: &SnapshotCheckpoint) -> Result<(), DbError> {
        let payload = Json(&checkpoint.snapshot);
        self.client().execute(
            r#"
            INSERT INTO engine_snapshots (engine_sequence, event_journal_id, payload)
            VALUES ($1, $2, $3)
            "#,
            &[
                &to_i64(checkpoint.snapshot.sequence),
                &to_i64(checkpoint.event_journal_id),
                &payload,
            ],
        )?;
        Ok(())
    }

    fn latest_snapshot(&mut self) -> Result<Option<SnapshotCheckpoint>, DbError> {
        let rows = self.client().query(
            r#"
            SELECT event_journal_id, payload
            FROM engine_snapshots
            ORDER BY id DESC
            LIMIT 1
            "#,
            &[],
        )?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(SnapshotCheckpoint {
            event_journal_id: from_i64(row.get(0)),
            snapshot: row.get::<_, Json<EngineSnapshot>>(1).0,
        }))
    }

    fn client(&mut self) -> &mut Client {
        match self.client.as_mut() {
            Some(client) => client,
            None => unreachable!("postgres client already closed"),
        }
    }
}

impl Drop for PostgresDatabase {
    fn drop(&mut self) {
        let Some(client) = self.client.take() else {
            return;
        };
        let _ = std::thread::spawn(move || drop(client)).join();
    }
}

const SQLITE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL DEFAULT '',
    side TEXT NOT NULL,
    type TEXT NOT NULL,
    time_in_force TEXT NOT NULL DEFAULT 'gtc',
    price INTEGER,
    stop_price INTEGER,
    original_quantity INTEGER NOT NULL,
    remaining_quantity INTEGER NOT NULL,
    original_quote_quantity INTEGER,
    remaining_quote_quantity INTEGER,
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
"#;

const POSTGRES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS orders (
    id BIGINT PRIMARY KEY,
    account_id TEXT NOT NULL DEFAULT '',
    side TEXT NOT NULL,
    type TEXT NOT NULL,
    time_in_force TEXT NOT NULL DEFAULT 'gtc',
    price BIGINT,
    stop_price BIGINT,
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
"#;

fn sqlite_upsert_order(
    conn: &Connection,
    order: &Order,
    status: OrderStatus,
) -> rusqlite::Result<()> {
    conn.execute(
        r#"
        INSERT INTO orders (
            id, account_id, side, type, time_in_force, price, stop_price, original_quantity, remaining_quantity, original_quote_quantity, remaining_quote_quantity, status, created_at_seq
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(id) DO UPDATE SET
            account_id = excluded.account_id,
            time_in_force = excluded.time_in_force,
            remaining_quantity = excluded.remaining_quantity,
            remaining_quote_quantity = excluded.remaining_quote_quantity,
            status = excluded.status,
            updated_at = CURRENT_TIMESTAMP
        "#,
        params![
            order.id,
            order.account_id,
            side_to_db(order.side),
            order_kind_to_db(order.kind),
            time_in_force_to_db(order.time_in_force),
            order.price,
            order.stop_price,
            order.original_quantity,
            order.remaining_quantity,
            order.original_quote_quantity,
            order.remaining_quote_quantity,
            status_to_db(status),
            order.created_at_seq,
        ],
    )?;
    Ok(())
}

fn sqlite_add_column_if_missing(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    conn.execute_batch(sql).or_else(|err| match err {
        rusqlite::Error::SqliteFailure(_, Some(message))
            if message.contains("duplicate column name") =>
        {
            Ok(())
        }
        other => Err(other),
    })
}

fn sqlite_insert_trade(conn: &Connection, trade: &Trade) -> rusqlite::Result<()> {
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

fn sqlite_insert_order_history(
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

fn sqlite_insert_event(conn: &Connection, event: &EngineEvent) -> Result<(), DbError> {
    let payload = serde_json::to_string(event)?;
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

fn postgres_upsert_order<C>(
    client: &mut C,
    order: &Order,
    status: OrderStatus,
) -> Result<(), postgres::Error>
where
    C: postgres::GenericClient,
{
    let price = order.price.map(to_i64);
    let side = side_to_db(order.side).to_string();
    let order_type = order_kind_to_db(order.kind).to_string();
    let time_in_force = time_in_force_to_db(order.time_in_force).to_string();
    let status = status_to_db(status).to_string();
    client.execute(
        r#"
        INSERT INTO orders (
            id, account_id, side, type, time_in_force, price, stop_price, original_quantity, remaining_quantity, original_quote_quantity, remaining_quote_quantity, status, created_at_seq
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
        ON CONFLICT(id) DO UPDATE SET
            account_id = EXCLUDED.account_id,
            time_in_force = EXCLUDED.time_in_force,
            remaining_quantity = EXCLUDED.remaining_quantity,
            remaining_quote_quantity = EXCLUDED.remaining_quote_quantity,
            status = EXCLUDED.status,
            updated_at = NOW()
        "#,
        &[
            &to_i64(order.id),
            &order.account_id,
            &side,
            &order_type,
            &time_in_force,
            &price,
            &order.stop_price.map(to_i64),
            &to_i64(order.original_quantity),
            &to_i64(order.remaining_quantity),
            &order.original_quote_quantity.map(to_i64),
            &order.remaining_quote_quantity.map(to_i64),
            &status,
            &to_i64(order.created_at_seq),
        ],
    )?;
    Ok(())
}

fn postgres_insert_trade<C>(client: &mut C, trade: &Trade) -> Result<(), postgres::Error>
where
    C: postgres::GenericClient,
{
    let aggressor_side = side_to_db(trade.aggressor_side).to_string();
    client.execute(
        r#"
        INSERT INTO trades (
            id, maker_order_id, taker_order_id, price, quantity, aggressor_side, sequence
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT(id) DO NOTHING
        "#,
        &[
            &to_i64(trade.id),
            &to_i64(trade.maker_order_id),
            &to_i64(trade.taker_order_id),
            &to_i64(trade.price),
            &to_i64(trade.quantity),
            &aggressor_side,
            &to_i64(trade.sequence),
        ],
    )?;
    Ok(())
}

fn postgres_insert_order_history<C>(
    client: &mut C,
    order_id: OrderId,
    entry: &OrderHistoryEntry,
) -> Result<(), postgres::Error>
where
    C: postgres::GenericClient,
{
    let status = status_to_db(entry.status).to_string();
    client.execute(
        r#"
        INSERT INTO order_history (
            order_id, sequence, status, remaining_quantity
        )
        VALUES ($1, $2, $3, $4)
        ON CONFLICT(order_id, sequence, status, remaining_quantity) DO NOTHING
        "#,
        &[
            &to_i64(order_id),
            &to_i64(entry.sequence),
            &status,
            &to_i64(entry.remaining_quantity),
        ],
    )?;
    Ok(())
}

fn postgres_insert_event<C>(client: &mut C, event: &EngineEvent) -> Result<(), DbError>
where
    C: postgres::GenericClient,
{
    let payload = Json(event);
    let event_type = event_type(event).to_string();
    client.execute(
        r#"
        INSERT INTO event_journal (
            engine_sequence, event_type, payload
        )
        VALUES ($1, $2, $3)
        "#,
        &[&event_sequence(event).map(to_i64), &event_type, &payload],
    )?;
    Ok(())
}

fn event_sequence(event: &EngineEvent) -> Option<u64> {
    match event {
        EngineEvent::Book { data } => Some(data.sequence),
        EngineEvent::Order { data } => Some(data.order.created_at_seq),
        EngineEvent::Replace { data } => Some(data.replacement.order.created_at_seq),
        EngineEvent::Trade { data } => Some(data.sequence),
        EngineEvent::Cancel { .. } => None,
        EngineEvent::MassCancel { .. } => None,
    }
}

fn event_type(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::Book { .. } => "book",
        EngineEvent::Order { .. } => "order",
        EngineEvent::Replace { .. } => "replace",
        EngineEvent::Trade { .. } => "trade",
        EngineEvent::Cancel { .. } => "cancel",
        EngineEvent::MassCancel { .. } => "mass_cancel",
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

fn time_in_force_to_db(time_in_force: TimeInForce) -> &'static str {
    match time_in_force {
        TimeInForce::Gtc => "gtc",
        TimeInForce::Ioc => "ioc",
        TimeInForce::Fok => "fok",
    }
}

fn order_kind_to_db(kind: crate::model::OrderKind) -> &'static str {
    match kind {
        crate::model::OrderKind::Limit => "limit",
        crate::model::OrderKind::Market => "market",
        crate::model::OrderKind::MarketByNotional => "market_by_notional",
        crate::model::OrderKind::PostOnly => "post_only",
        crate::model::OrderKind::StopLimit => "stop_limit",
        crate::model::OrderKind::StopMarket => "stop_market",
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

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}
