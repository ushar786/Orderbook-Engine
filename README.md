# Orderbook Engine

A compact, production-shaped single-book matching engine modeled after [`OrderBook-rs`](https://github.com/joaquinbejar/OrderBook-rs).

The first version keeps the business surface intentionally simple: one orderbook, integer tick/lot validation, limit/market orders, FIFO matching inside each price level, REST APIs, WebSocket updates, SQLite audit storage, and a static frontend.

## Shape

```txt
backend/
  src/
    engine/
      book.rs          # matching engine and sequencing
      order_state.rs   # lifecycle history tracking
      errors.rs        # typed rejects
      reject_reason.rs # validation to typed rejects
      price_level.rs   # FIFO queue per price level
      snapshot.rs      # book snapshot builder
      trade.rs         # trade creation and retention
    api.rs             # REST + WebSocket transport
    db.rs              # SQLite audit storage
    model.rs           # DTOs shared by engine/API/frontend
  benches/             # Criterion benchmarks
frontend/              # static trading dashboard
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the frontend, backend, database, and engine plan.

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

## API

- `POST /api/orders` submits an order.
- `DELETE /api/orders/:id` cancels a resting order.
- `GET /api/orders/:id/history` returns in-memory order lifecycle history.
- `GET /api/book` returns top-of-book depth.
- `GET /api/trades` returns recent trades.
- `GET /ws` streams book, trade, and order events.

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

Recommended local loop:

```sh
cd backend
cargo fmt
cargo test
cargo check-all
```
