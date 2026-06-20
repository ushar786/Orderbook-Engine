# Reference Alignment

This project is not an exact copy of `joaquinbejar/OrderBook-rs`; it is a Rust-first implementation that tracks the same orderbook domain in a smaller product shell.

## Implemented Alignment

- Single hot-path Rust engine isolated from API, database, and frontend code.
- Integer price/quantity ticks and lot validation.
- Price-time priority matching with FIFO queues inside each price level.
- Limit, market, market-by-notional, post-only, stop-limit, and stop-market orders.
- GTC, IOC, and FOK time-in-force.
- Order cancel, cancel-replace, mass cancel, active order index, and active order reads.
- Account-aware self-trade prevention.
- Protocol envelope module for inbound/outbound messages.
- Account blocklist and per-account open order risk hooks.
- Performance guard tests for crossing and snapshot workloads.
- CLI binary verified against the running REST backend.
- Dedicated in-memory sequencer, trades, enriched snapshots, and order lifecycle history.
- Serialized command worker for concurrent producer threads and API transport.
- Complete engine snapshot restore and deterministic event-journal replay.
- Persisted snapshot checkpoints for faster replay startup.
- Configurable risk checks for max quantity, max notional, max open orders, and kill switch.
- Durable SQLite/PostgreSQL audit tables for orders, trades, order history, event journal, and engine snapshots.
- REST, WebSocket, frontend dashboard, engine metrics, integration tests, and Criterion benchmarks.

## Reference Features Still Missing

- Full risk layer with exposure limits and venue policy hooks.
- Position-aware reduce-only orders.
- Wire protocol modules for inbound/outbound messages.
- Allocation budget tests with a custom allocator.
- Larger example suite and user guide.

## Next Alignment Order

1. Add position-aware reduce-only orders.
2. Add allocation-budget tests with a custom allocator.
3. Add larger example suite and user guide.
