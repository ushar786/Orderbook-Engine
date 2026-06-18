use std::{cmp::Reverse, collections::BTreeMap};

use crate::{
    engine::price_level::PriceLevel,
    model::{BookSnapshot, Price},
};

pub fn build_snapshot(
    symbol: &str,
    sequence: u64,
    bids: &BTreeMap<Reverse<Price>, PriceLevel>,
    asks: &BTreeMap<Price, PriceLevel>,
    depth: usize,
) -> BookSnapshot {
    let best_bid = bids.keys().next().map(|price| price.0);
    let best_ask = asks.keys().next().copied();
    let spread = best_bid
        .zip(best_ask)
        .and_then(|(bid, ask)| ask.checked_sub(bid));
    let mid_price = best_bid
        .zip(best_ask)
        .map(|(bid, ask)| (bid as f64 + ask as f64) / 2.0);
    let bid_depth = bids.values().map(PriceLevel::depth).sum();
    let ask_depth = asks.values().map(PriceLevel::depth).sum();
    let bid_order_count = bids.values().map(PriceLevel::len).sum();
    let ask_order_count = asks.values().map(PriceLevel::len).sum();

    BookSnapshot {
        symbol: symbol.to_string(),
        sequence,
        best_bid,
        best_ask,
        spread,
        mid_price,
        bid_depth,
        ask_depth,
        bid_order_count,
        ask_order_count,
        bids: bids
            .iter()
            .take(depth)
            .map(|(price, level)| level.snapshot(price.0))
            .collect(),
        asks: asks
            .iter()
            .take(depth)
            .map(|(price, level)| level.snapshot(*price))
            .collect(),
    }
}
