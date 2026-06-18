# Reference Alignment

This project is not an exact copy of `joaquinbejar/OrderBook-rs`; it is a Rust-first implementation that tracks the same orderbook domain in a smaller product shell.

## Implemented Alignment

- Single hot-path Rust engine isolated from API, database, and frontend code.
- Integer price/quantity ticks and lot validation.
- Price-time priority matching with FIFO queues inside each price level.
- Limit and market orders.
- GTC and IOC time-in-force.
- Order cancel, cancel-replace, mass cancel, active order index, and active order reads.
- Engine sequence numbers, trades, enriched snapshots, and order lifecycle history.
- Complete engine snapshot restore and deterministic event-journal replay.
- Persisted snapshot checkpoints for faster replay startup.
- Configurable risk checks for max quantity, max notional, max open orders, and kill switch.
- Durable SQLite/PostgreSQL audit tables for orders, trades, order history, event journal, and engine snapshots.
- REST, WebSocket, frontend dashboard, integration tests, and Criterion benchmarks.

## Reference Features Still Missing

- Full risk layer with exposure limits and venue policy hooks.
- Self-trade prevention.
- Market order by amount/notional.
- Wire protocol modules for inbound/outbound messages.
- Metrics.
- Allocation budget tests.
- Larger example suite and user guide.

## Next Alignment Order

1. Add self-trade prevention.
2. Add metrics and allocation-focused benchmarks.
3. Add exposure limits and venue policy hooks.
4. Add wire protocol modules.
