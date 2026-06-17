# Reference Alignment

This project is not an exact copy of `joaquinbejar/OrderBook-rs`; it is a Rust-first implementation that tracks the same orderbook domain in a smaller product shell.

## Implemented Alignment

- Single hot-path Rust engine isolated from API, database, and frontend code.
- Integer price/quantity ticks and lot validation.
- Price-time priority matching with FIFO queues inside each price level.
- Limit and market orders.
- GTC and IOC time-in-force.
- Order cancel, cancel-replace, mass cancel, active order index, and active order reads.
- Engine sequence numbers, trades, snapshots, and order lifecycle history.
- Durable SQLite audit tables for orders, trades, order history, and event journal.
- REST, WebSocket, frontend dashboard, integration tests, and Criterion benchmarks.

## Reference Features Still Missing

- Kill switch.
- Risk layer.
- Self-trade prevention.
- Market order by amount/notional.
- Snapshot restore and deterministic replay into an in-memory book.
- Wire protocol modules for inbound/outbound messages.
- Metrics.
- Allocation budget tests and broader benchmark matrix.
- Larger example suite and user guide.

## Next Alignment Order

1. Complete Phase 2 with PostgreSQL persistence.
2. Add snapshot restore for Phase 3.
3. Add risk controls and self-trade prevention.
4. Add kill switch.
5. Add metrics and allocation-focused benchmarks.
