use rusqlite::Connection;

const INITIAL_SCHEMA: &str = include_str!("migrations/001_initial.sql");

pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(INITIAL_SCHEMA)
}
