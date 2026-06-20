# Database

Database-related assets live here so the persistence layer is easy to find from VS Code.

## Backends

- SQLite is the default local development backend.
- PostgreSQL is selected when `ORDERBOOK_DB` starts with `postgres://` or `postgresql://`.

## Migrations

```txt
database/migrations/
  sqlite/001_init.sql
  postgres/001_init.sql
```

The Rust server currently applies equivalent schema creation at startup from `backend/src/db.rs`.

The schema includes:

- `orders`, including `account_id`, optional quote quantity, and optional stop price for self-trade prevention/audit
- `trades`
- `order_history`
- `event_journal`
- `engine_snapshots`
