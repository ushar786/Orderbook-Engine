# Orderbook Engine

A compact, production-shaped single-book matching engine modeled after [`OrderBook-rs`](https://github.com/joaquinbejar/OrderBook-rs).

The first version keeps the business surface intentionally simple: one orderbook, integer tick/lot validation, limit/market orders, FIFO matching inside each price level, REST APIs, WebSocket updates, SQLite audit storage, and a static frontend.

## Shape

```txt
src/
  engine/
    book.rs          # matching engine and sequencing
    errors.rs        # typed rejects
    price_level.rs   # FIFO queue per price level
  api.rs             # REST + WebSocket transport
  db.rs              # SQLite audit storage
  model.rs           # DTOs shared by engine/API/frontend
frontend/            # static trading dashboard
benches/             # Criterion benchmarks
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the frontend, backend, database, and engine plan.

## Run

```sh
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
- `GET /api/book` returns top-of-book depth.
- `GET /api/trades` returns recent trades.
- `GET /ws` streams book, trade, and order events.

Prices and quantities are unsigned integers. In a real venue these should represent fixed-point ticks and lots.

## Quality

```sh
cargo fmt
cargo test
cargo bench
```
