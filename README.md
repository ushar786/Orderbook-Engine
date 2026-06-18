# Orderbook Engine

A compact, production-shaped single-book matching engine modeled after [`OrderBook-rs`](https://github.com/joaquinbejar/OrderBook-rs).

The first version keeps the business surface intentionally simple: one in-memory orderbook, integer tick/lot/risk validation, limit/market orders, GTC/IOC time-in-force, FIFO matching inside each price level, enriched snapshots, REST APIs, WebSocket updates, SQLite/PostgreSQL audit storage, and a static frontend.

## Shape

```txt
backend/
  src/
    engine/
      book.rs          # matching engine and sequencing
      matching.rs      # price-crossing and fill loop
      order_state.rs   # lifecycle history tracking
      errors.rs        # typed rejects
      reject_reason.rs # validation to typed rejects
      price_level.rs   # FIFO queue per price level
      snapshot.rs      # book snapshot builder
      trade.rs         # trade creation and retention
    api.rs             # REST + WebSocket transport
    db.rs              # SQLite/PostgreSQL audit storage
    model.rs           # DTOs shared by engine/API/frontend
  benches/             # Criterion benchmarks
  tests/                # API integration tests
database/              # SQL schemas and persistence notes
frontend/              # static trading dashboard
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the frontend, backend, database, and engine plan. See [REFERENCE_ALIGNMENT.md](REFERENCE_ALIGNMENT.md) for the current match against the reference repo.

## Phases

```txt
Phase 1: limit order + market order + matching + cancel
Phase 2: Axum REST API + WebSocket frontend + PostgreSQL persistence
Phase 3: benchmarks + snapshots + risk checks
Phase 4: concurrency + sequencer + advanced order types
```

## Run

```sh
cd backend
cargo run --release
```

Open `http://127.0.0.1:8080`.

Useful environment variables:

```sh
ORDERBOOK_ADDR=127.0.0.1:8080
ORDERBOOK_DB=data/orderbook.db
RUST_LOG=info
```

For PostgreSQL persistence, point `ORDERBOOK_DB` at a PostgreSQL URL:

```sh
ORDERBOOK_DB=postgres://orderbook:orderbook@localhost:5432/orderbook
```

The server creates the same schema at startup for both backends. SQL copies live in `database/migrations/`.

## API

- `POST /api/orders` submits an order.
- `GET /api/orders` returns active resting/partially-filled orders.
- `DELETE /api/orders` mass-cancels active resting/partially-filled orders.
- `DELETE /api/orders/:id` cancels a resting order.
- `PATCH /api/orders/:id` cancel-replaces a resting order.
- `GET /api/orders/:id/history` returns in-memory order lifecycle history.
- `GET /api/book` returns top-of-book depth.
- `GET /api/trades` returns recent trades.
- `GET /api/metrics` returns in-memory engine metrics.
- `GET /api/engine-snapshot` exports a complete in-memory engine snapshot.
- `POST /api/engine-snapshot` restores the in-memory book from a complete engine snapshot.
- `GET /api/engine-snapshot/checkpoint` returns the latest persisted snapshot checkpoint.
- `POST /api/engine-snapshot/checkpoint` persists the current engine snapshot as a replay checkpoint.
- `POST /api/replay` rebuilds the in-memory book from the latest checkpoint plus durable event journal.
- `GET /api/risk/kill-switch` returns kill switch state.
- `POST /api/risk/kill-switch` enables/disables kill switch order blocking.
- `GET /ws` upgrades to a WebSocket stream for book, trade, order, and cancel events.

Prices and quantities are unsigned integers. In a real venue these should represent fixed-point ticks and lots.

## Quality

```sh
cd backend
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
cargo bench
```

## Rust Workflow

This repo is configured as a Rust-first project:

- `backend/rust-toolchain.toml` pins the stable toolchain with `rustfmt` and `clippy`.
- `backend/rustfmt.toml` keeps formatting consistent.
- `backend/.cargo/config.toml` adds aliases:
  - `cargo dev`
  - `cargo t`
  - `cargo check-all`
  - `cargo b`
- `unsafe_code` is forbidden for this engine.
- Criterion benchmarks cover add-only, crossing, cancel, and mixed workloads.
- Phase 3 benchmark coverage also includes snapshot depth, risk rejection, metrics, and snapshot capture workloads.
- API integration tests cover matching, active orders, cancel, persisted history, event journal, and deterministic replay.

Recommended local loop:

```sh
cd backend
cargo fmt
cargo test
cargo check-all
```
