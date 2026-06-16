#![allow(clippy::unwrap_used)]

use criterion::{Criterion, criterion_group, criterion_main};
use orderbook_engine::{
    engine::OrderBook,
    model::{NewOrder, OrderKind, Side},
};

fn mixed_matching(c: &mut Criterion) {
    c.bench_function("mixed_limit_matching", |b| {
        b.iter(|| {
            let mut book = OrderBook::new("BTC-USD");
            for offset in 0..100 {
                book.submit(NewOrder {
                    side: Side::Sell,
                    kind: OrderKind::Limit,
                    price: Some(10_000 + offset),
                    quantity: 10,
                })
                .unwrap();
                book.submit(NewOrder {
                    side: Side::Buy,
                    kind: OrderKind::Limit,
                    price: Some(9_999 - offset),
                    quantity: 10,
                })
                .unwrap();
            }
            for _ in 0..50 {
                book.submit(NewOrder {
                    side: Side::Buy,
                    kind: OrderKind::Market,
                    price: None,
                    quantity: 5,
                })
                .unwrap();
            }
        });
    });
}

criterion_group!(benches, mixed_matching);
criterion_main!(benches);
