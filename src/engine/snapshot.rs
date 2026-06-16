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
    BookSnapshot {
        symbol: symbol.to_string(),
        sequence,
        best_bid: bids.keys().next().map(|price| price.0),
        best_ask: asks.keys().next().copied(),
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
