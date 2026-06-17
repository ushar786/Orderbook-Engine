# Architecture

This project follows the shape of [`joaquinbejar/OrderBook-rs`](https://github.com/joaquinbejar/OrderBook-rs) while keeping the first product version intentionally small.

## Reference Alignment

- `src/engine/` is the hot path. API, database, and frontend code do not own matching rules.
- Prices and quantities are integer ticks/lots, not floats.
- Matching is price-time priority: best price first, FIFO inside a price level.
- Every accepted order and trade advances an engine sequence.
- Snapshots are explicit DTOs and are safe to stream over REST/WebSocket.
- Validation returns typed errors rather than free-form strings.
- Benchmarks and unit tests live with the engine.

Current Rust engine modules:

```txt
backend/src/engine/
  book.rs
  matching.rs
  price_level.rs
  order_state.rs
  reject_reason.rs
  snapshot.rs
  trade.rs
  errors.rs
```

## Frontend Plan

- Keep the current static dashboard as the first usable screen.
- Limit/market ticket supports side, price, and quantity controls.
- Live book depth shows best bid/ask, spread, mid price, depth levels, and sequence.
- Recent trades, order results, cancel actions, and lifecycle history are wired to the API.
- Active orders are loaded on refresh through `GET /api/orders`.
- Later: add latency indicators, symbol selector, and persisted event replay.

## Backend Plan

- Rust + Axum server.
- REST routes for health, active orders, submit order, cancel order, order history, book snapshot, and recent trades.
- WebSocket route for book/trade/order/cancel events.
- Keep API handlers thin: validate transport, call engine, persist audit trail, publish event.
- Later: add auth, rate limits, structured request IDs, metrics, and graceful replay on startup.

## Database Plan

- SQLite for local-first audit storage.
- WAL mode enabled for better concurrent reads.
- `orders` table stores lifecycle state and remaining quantity.
- `trades` table stores immutable executions by engine sequence.
- Later: add event journal table, snapshots table, and PostgreSQL-compatible migrations.

## Engine Roadmap

1. Single-book limit/market matching.
2. Tick/lot validation.
3. Cancel and order index.
4. Snapshot and WebSocket event stream.
5. Active order reads for the frontend.
6. Benchmarks for add-only, crossing, cancel, and mixed workloads.
7. Optional risk controls, kill switch, replay journal, and metrics.
