use std::cmp::Reverse;

use crate::model::{Order, OrderKind, Price, Trade};

use super::OrderBook;

impl OrderBook {
    pub(super) fn match_buy(&mut self, taker: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        while has_remaining_buy_interest(taker) {
            let Some(best_ask) = self.best_matchable_ask(taker) else {
                break;
            };
            let Some(trade) = self.fill_at_ask(best_ask, taker) else {
                break;
            };
            trades.push(trade);
        }
        trades
    }

    pub(super) fn match_sell(&mut self, taker: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        while taker.remaining_quantity > 0 {
            let Some(best_bid) = self.best_matchable_bid(taker) else {
                break;
            };
            let Some(trade) = self.fill_at_bid(best_bid, taker) else {
                break;
            };
            trades.push(trade);
        }
        trades
    }

    fn best_matchable_ask(&self, taker: &Order) -> Option<Price> {
        self.asks.iter().find_map(|(price, level)| {
            if matches!(taker.kind, OrderKind::Limit | OrderKind::PostOnly)
                && taker.price.is_some_and(|limit| *price > limit)
            {
                return None;
            }
            level
                .has_matchable_order(&taker.account_id)
                .then_some(*price)
        })
    }

    fn best_matchable_bid(&self, taker: &Order) -> Option<Price> {
        self.bids.iter().find_map(|(price, level)| {
            let price = price.0;
            if matches!(taker.kind, OrderKind::Limit | OrderKind::PostOnly)
                && taker.price.is_some_and(|limit| price < limit)
            {
                return None;
            }
            level
                .has_matchable_order(&taker.account_id)
                .then_some(price)
        })
    }

    fn fill_at_ask(&mut self, price: Price, taker: &mut Order) -> Option<Trade> {
        let (maker_id, trade_quantity, maker_remaining, maker_filled, level_empty) = {
            let level = self.asks.get_mut(&price)?;
            let maker = level.first_matchable_mut(&taker.account_id)?;
            let maker_id = maker.id;
            let trade_quantity = trade_quantity_at_price(maker.remaining_quantity, taker, price)?;
            maker.remaining_quantity -= trade_quantity;
            reduce_taker(taker, price, trade_quantity);
            let maker_remaining = maker.remaining_quantity;
            let maker_filled = maker.remaining_quantity == 0;
            if maker_filled {
                level.remove(maker_id);
            }
            (
                maker_id,
                trade_quantity,
                maker_remaining,
                maker_filled,
                level.is_empty(),
            )
        };
        if level_empty {
            self.asks.remove(&price);
        }
        if maker_filled {
            self.order_index.remove(&maker_id);
        }
        let trade = self.trade(maker_id, taker.id, price, trade_quantity, taker.side);
        self.record_maker_fill(maker_id, maker_remaining, maker_filled);
        Some(trade)
    }

    fn fill_at_bid(&mut self, price: Price, taker: &mut Order) -> Option<Trade> {
        let (maker_id, trade_quantity, maker_remaining, maker_filled, level_empty) = {
            let level = self.bids.get_mut(&Reverse(price))?;
            let maker = level.first_matchable_mut(&taker.account_id)?;
            let maker_id = maker.id;
            let trade_quantity = maker.remaining_quantity.min(taker.remaining_quantity);
            maker.remaining_quantity -= trade_quantity;
            taker.remaining_quantity -= trade_quantity;
            let maker_remaining = maker.remaining_quantity;
            let maker_filled = maker.remaining_quantity == 0;
            if maker_filled {
                level.remove(maker_id);
            }
            (
                maker_id,
                trade_quantity,
                maker_remaining,
                maker_filled,
                level.is_empty(),
            )
        };
        if level_empty {
            self.bids.remove(&Reverse(price));
        }
        if maker_filled {
            self.order_index.remove(&maker_id);
        }
        let trade = self.trade(maker_id, taker.id, price, trade_quantity, taker.side);
        self.record_maker_fill(maker_id, maker_remaining, maker_filled);
        Some(trade)
    }
}

fn has_remaining_buy_interest(taker: &Order) -> bool {
    match taker.kind {
        OrderKind::MarketByNotional => taker
            .remaining_quote_quantity
            .is_some_and(|value| value > 0),
        _ => taker.remaining_quantity > 0,
    }
}

fn trade_quantity_at_price(
    maker_remaining_quantity: u64,
    taker: &Order,
    price: Price,
) -> Option<u64> {
    match taker.kind {
        OrderKind::MarketByNotional => {
            let max_by_quote = taker.remaining_quote_quantity? / price;
            let quantity = maker_remaining_quantity.min(max_by_quote);
            (quantity > 0).then_some(quantity)
        }
        _ => Some(maker_remaining_quantity.min(taker.remaining_quantity)),
    }
}

fn reduce_taker(taker: &mut Order, price: Price, quantity: u64) {
    match taker.kind {
        OrderKind::MarketByNotional => {
            if let Some(remaining_quote_quantity) = taker.remaining_quote_quantity.as_mut() {
                *remaining_quote_quantity =
                    remaining_quote_quantity.saturating_sub(price.saturating_mul(quantity));
            }
        }
        _ => taker.remaining_quantity -= quantity,
    }
}
