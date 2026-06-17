use std::cmp::Reverse;

use crate::model::{Order, OrderKind, Price, Trade};

use super::OrderBook;

impl OrderBook {
    pub(super) fn match_buy(&mut self, taker: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();
        while taker.remaining_quantity > 0 {
            let Some(best_ask) = self.asks.keys().next().copied() else {
                break;
            };
            if taker.kind == OrderKind::Limit && taker.price.is_some_and(|limit| best_ask > limit) {
                break;
            }
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
            let Some(best_bid) = self.bids.keys().next().map(|price| price.0) else {
                break;
            };
            if taker.kind == OrderKind::Limit && taker.price.is_some_and(|limit| best_bid < limit) {
                break;
            }
            let Some(trade) = self.fill_at_bid(best_bid, taker) else {
                break;
            };
            trades.push(trade);
        }
        trades
    }

    fn fill_at_ask(&mut self, price: Price, taker: &mut Order) -> Option<Trade> {
        let (maker_id, trade_quantity, maker_remaining, maker_filled, level_empty) = {
            let level = self.asks.get_mut(&price)?;
            let maker = level.front_mut()?;
            let maker_id = maker.id;
            let trade_quantity = maker.remaining_quantity.min(taker.remaining_quantity);
            maker.remaining_quantity -= trade_quantity;
            taker.remaining_quantity -= trade_quantity;
            let maker_remaining = maker.remaining_quantity;
            let maker_filled = maker.remaining_quantity == 0;
            if maker_filled {
                level.pop_front();
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
            let maker = level.front_mut()?;
            let maker_id = maker.id;
            let trade_quantity = maker.remaining_quantity.min(taker.remaining_quantity);
            maker.remaining_quantity -= trade_quantity;
            taker.remaining_quantity -= trade_quantity;
            let maker_remaining = maker.remaining_quantity;
            let maker_filled = maker.remaining_quantity == 0;
            if maker_filled {
                level.pop_front();
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
