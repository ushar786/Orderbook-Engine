#![allow(clippy::unwrap_used)]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use orderbook_engine::{
    engine::OrderBook,
    model::{NewOrder, OrderKind, Price, Quantity, Side, TimeInForce},
};

fn limit(side: Side, price: Price, quantity: Quantity) -> NewOrder {
    NewOrder {
        side,
        kind: OrderKind::Limit,
        time_in_force: TimeInForce::Gtc,
        price: Some(price),
        quantity,
    }
}

fn market(side: Side, quantity: Quantity) -> NewOrder {
    NewOrder {
        side,
        kind: OrderKind::Market,
        time_in_force: TimeInForce::Gtc,
        price: None,
        quantity,
    }
}

fn add_only(c: &mut Criterion) {
    c.bench_function("add_only_1k_resting_orders", |b| {
        b.iter(|| {
            let mut book = OrderBook::new("BTC-USD");
            for offset in 0..500 {
                black_box(book.submit(limit(Side::Buy, 9_999 - offset, 10)).unwrap());
                black_box(book.submit(limit(Side::Sell, 10_001 + offset, 10)).unwrap());
            }
            black_box(book.active_order_count());
        });
    });
}

fn crossing(c: &mut Criterion) {
    c.bench_function("crossing_1k_price_time_matches", |b| {
        b.iter(|| {
            let mut book = OrderBook::new("BTC-USD");
            for _ in 0..1_000 {
                book.submit(limit(Side::Sell, 10_000, 1)).unwrap();
            }
            black_box(book.submit(limit(Side::Buy, 10_000, 1_000)).unwrap());
            black_box(book.snapshot(1));
        });
    });
}

fn cancel(c: &mut Criterion) {
    c.bench_function("cancel_1k_resting_orders", |b| {
        b.iter(|| {
            let mut book = OrderBook::new("BTC-USD");
            let mut order_ids = Vec::with_capacity(1_000);
            for offset in 0..1_000 {
                let ack = book.submit(limit(Side::Buy, 9_000 + offset, 1)).unwrap();
                order_ids.push(ack.ack.order.id);
            }
            for order_id in order_ids {
                black_box(book.cancel(order_id).unwrap());
            }
            black_box(book.active_order_count());
        });
    });
}

fn mixed_matching(c: &mut Criterion) {
    c.bench_function("mixed_limit_market_workload", |b| {
        b.iter(|| {
            let mut book = OrderBook::new("BTC-USD");
            for offset in 0..100 {
                book.submit(limit(Side::Sell, 10_000 + offset, 10)).unwrap();
                book.submit(limit(Side::Buy, 9_999 - offset, 10)).unwrap();
            }
            for _ in 0..50 {
                black_box(book.submit(market(Side::Buy, 5)).unwrap());
                black_box(book.submit(market(Side::Sell, 5)).unwrap());
            }
            black_box(book.snapshot(10));
        });
    });
}

criterion_group!(benches, add_only, crossing, cancel, mixed_matching);
criterion_main!(benches);
