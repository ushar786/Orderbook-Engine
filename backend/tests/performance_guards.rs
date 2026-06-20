#![allow(clippy::unwrap_used)]

use std::time::Instant;

use orderbook_engine::{
    engine::OrderBook,
    model::{NewOrder, OrderKind, Side, TimeInForce},
};

fn limit(side: Side, price: u64, quantity: u64) -> NewOrder {
    NewOrder {
        account_id: String::new(),
        side,
        kind: OrderKind::Limit,
        time_in_force: TimeInForce::Gtc,
        price: Some(price),
        stop_price: None,
        quantity,
        quote_quantity: None,
    }
}

#[test]
fn matching_10k_orders_stays_inside_guardrail() {
    let mut book = OrderBook::new("BTC-USD");
    for _ in 0..10_000 {
        book.submit(limit(Side::Sell, 10_000, 1)).unwrap();
    }

    let started = Instant::now();
    let outcome = book.submit(limit(Side::Buy, 10_000, 10_000)).unwrap();

    assert_eq!(outcome.ack.trades.len(), 10_000);
    assert!(
        started.elapsed().as_millis() < 1_500,
        "10k crossing workload exceeded 1500ms guardrail"
    );
}

#[test]
fn snapshot_2k_orders_stays_inside_guardrail() {
    let mut book = OrderBook::new("BTC-USD");
    for offset in 0..1_000 {
        book.submit(limit(Side::Buy, 10_000 - offset, 1)).unwrap();
        book.submit(limit(Side::Sell, 10_001 + offset, 1)).unwrap();
    }

    let started = Instant::now();
    let snapshot = book.snapshot(50);

    assert_eq!(snapshot.bids.len(), 50);
    assert_eq!(snapshot.asks.len(), 50);
    assert!(
        started.elapsed().as_millis() < 200,
        "2k order snapshot exceeded 200ms guardrail"
    );
}
