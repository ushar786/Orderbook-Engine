mod repository;
mod schema;

use std::path::Path;

use rusqlite::Connection;

pub use repository::{OrderRepository, TradeRepository};

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
        schema::configure(&conn)?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn orders(&mut self) -> OrderRepository<'_> {
        OrderRepository::new(&mut self.conn)
    }

    pub fn trades(&self) -> TradeRepository<'_> {
        TradeRepository::new(&self.conn)
    }
}
