use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, VecDeque},
};

use crate::{
    engine::{
        MatchError,
        order_state::{self, OrderHistory},
        price_level::PriceLevel,
        reject_reason, snapshot, trade,
    },
    model::{
        BookSnapshot, EngineEvent, NewOrder, Order, OrderAck, OrderHistoryEntry, OrderId,
        OrderKind, OrderStatus, Price, Quantity, Side, Trade,
    },
};

#[derive(Debug, Clone)]
pub struct BookConfig {
    pub symbol: String,
    pub tick_size: Price,
    pub lot_size: Quantity,
    pub max_recent_trades: usize,
    pub default_depth: usize,
}

impl BookConfig {
    pub fn btc_usd() -> Self {
        Self {
            symbol: "BTC-USD".to_string(),
            tick_size: 1,
            lot_size: 1,
            max_recent_trades: 256,
            default_depth: 25,
        }
    }
}

#[derive(Debug)]
pub struct MatchOutcome {
    pub ack: OrderAck,
    pub events: Vec<EngineEvent>,
}

#[derive(Debug)]
pub struct OrderBook {
    config: BookConfig,
    next_order_id: OrderId,
    next_trade_id: u64,
    sequence: u64,
    bids: BTreeMap<Reverse<Price>, PriceLevel>,
    asks: BTreeMap<Price, PriceLevel>,
    order_index: HashMap<OrderId, (Side, Price)>,
    order_history: OrderHistory,
    recent_trades: VecDeque<Trade>,
}

impl OrderBook {
    pub fn new(symbol: impl Into<String>) -> Self {
        Self::with_config(BookConfig {
            symbol: symbol.into(),
            ..BookConfig::btc_usd()
        })
    }

    pub fn with_config(config: BookConfig) -> Self {
        Self {
            config,
            next_order_id: 1,
            next_trade_id: 1,
            sequence: 0,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_index: HashMap::new(),
            order_history: HashMap::new(),
            recent_trades: VecDeque::with_capacity(256),
        }
    }

    pub fn submit(&mut self, request: NewOrder) -> Result<MatchOutcome, MatchError> {
        if let Err(err) =
            reject_reason::validate_order(&request, self.config.tick_size, self.config.lot_size)
        {
            let order_id = self.allocate_order_id();
            let rejected = Order {
                id: order_id,
                side: request.side,
                kind: request.kind,
                price: request.price,
                original_quantity: request.quantity,
                remaining_quantity: request.quantity,
                created_at_seq: self.next_sequence(),
            };
            self.record_status(&rejected, OrderStatus::Rejected);
            return Err(err);
        }

        let price = match request.kind {
            OrderKind::Limit => Some(request.price.ok_or(MatchError::MissingLimitPrice)?),
            OrderKind::Market => None,
        };

        let order_id = self.allocate_order_id();
        let mut taker = Order {
            id: order_id,
            side: request.side,
            kind: request.kind,
            price,
            original_quantity: request.quantity,
            remaining_quantity: request.quantity,
            created_at_seq: self.next_sequence(),
        };
        self.record_status(&taker, OrderStatus::Accepted);

        let trades = match taker.side {
            Side::Buy => self.match_buy(&mut taker),
            Side::Sell => self.match_sell(&mut taker),
        };

        let status = if taker.remaining_quantity == 0 {
            OrderStatus::Filled
        } else if taker.kind == OrderKind::Market {
            if trades.is_empty() {
                OrderStatus::Rejected
            } else {
                OrderStatus::PartiallyFilled
            }
        } else {
            self.rest(taker.clone());
            if taker.remaining_quantity == taker.original_quantity {
                OrderStatus::Resting
            } else {
                OrderStatus::PartiallyFilled
            }
        };
        self.record_status(&taker, status);

        let ack = OrderAck {
            order: taker,
            status,
            trades,
        };
        let mut events = Vec::with_capacity(ack.trades.len() + 2);
        events.push(EngineEvent::Order { data: ack.clone() });
        events.extend(
            ack.trades
                .iter()
                .cloned()
                .map(|trade| EngineEvent::Trade { data: trade }),
        );
        events.push(EngineEvent::Book {
            data: self.snapshot(self.config.default_depth),
        });

        Ok(MatchOutcome { ack, events })
    }

    pub fn cancel(&mut self, order_id: OrderId) -> Result<Order, MatchError> {
        let (side, price) = self
            .order_index
            .remove(&order_id)
            .ok_or(MatchError::OrderNotFound)?;

        let removed = match side {
            Side::Buy => remove_from_level(&mut self.bids, Reverse(price), order_id),
            Side::Sell => remove_from_level(&mut self.asks, price, order_id),
        };
        let cancelled = removed.ok_or(MatchError::OrderNotFound)?;
        self.next_sequence();
        self.record_status(&cancelled, OrderStatus::Cancelled);
        Ok(cancelled)
    }

    pub fn snapshot(&self, depth: usize) -> BookSnapshot {
        snapshot::build_snapshot(
            &self.config.symbol,
            self.sequence,
            &self.bids,
            &self.asks,
            depth,
        )
    }

    pub fn engine_seq(&self) -> u64 {
        self.sequence
    }

    pub fn get_order_history(&self, order_id: OrderId) -> Vec<OrderHistoryEntry> {
        self.order_history
            .get(&order_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn active_order_count(&self) -> usize {
        self.order_index.len()
    }

    fn match_buy(&mut self, taker: &mut Order) -> Vec<Trade> {
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

    fn match_sell(&mut self, taker: &mut Order) -> Vec<Trade> {
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

    fn rest(&mut self, order: Order) {
        let Some(price) = order.price else {
            return;
        };
        self.order_index.insert(order.id, (order.side, price));
        match order.side {
            Side::Buy => self.bids.entry(Reverse(price)).or_default().push(order),
            Side::Sell => self.asks.entry(price).or_default().push(order),
        }
    }

    fn trade(
        &mut self,
        maker_order_id: OrderId,
        taker_order_id: OrderId,
        price: Price,
        quantity: Quantity,
        aggressor_side: Side,
    ) -> Trade {
        let sequence = self.next_sequence();
        let trade = trade::create_trade(
            &mut self.next_trade_id,
            sequence,
            maker_order_id,
            taker_order_id,
            price,
            quantity,
            aggressor_side,
        );
        trade::retain_recent_trades(
            &mut self.recent_trades,
            self.config.max_recent_trades,
            trade.clone(),
        );
        trade
    }

    fn allocate_order_id(&mut self) -> OrderId {
        let id = self.next_order_id;
        self.next_order_id += 1;
        id
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    fn record_status(&mut self, order: &Order, status: OrderStatus) {
        order_state::record_status(&mut self.order_history, self.sequence, order, status);
    }

    fn record_maker_fill(
        &mut self,
        order_id: OrderId,
        remaining_quantity: Quantity,
        maker_filled: bool,
    ) {
        order_state::record_fill(
            &mut self.order_history,
            self.sequence,
            order_id,
            remaining_quantity,
            maker_filled,
        );
    }
}

fn remove_from_level<K: Ord>(
    book: &mut BTreeMap<K, PriceLevel>,
    price: K,
    order_id: OrderId,
) -> Option<Order> {
    let level = book.get_mut(&price)?;
    let removed = level.remove(order_id);
    if level.is_empty() {
        book.remove(&price);
    }
    removed
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn limit(side: Side, price: Price, quantity: Quantity) -> NewOrder {
        NewOrder {
            side,
            kind: OrderKind::Limit,
            price: Some(price),
            quantity,
        }
    }

    #[test]
    fn matches_crossing_limit_orders_fifo() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Sell, 101, 3)).unwrap();
        book.submit(limit(Side::Sell, 101, 4)).unwrap();

        let outcome = book.submit(limit(Side::Buy, 101, 5)).unwrap();

        assert_eq!(outcome.ack.trades.len(), 2);
        assert_eq!(outcome.ack.trades[0].quantity, 3);
        assert_eq!(outcome.ack.trades[1].quantity, 2);
        assert_eq!(book.snapshot(5).asks[0].quantity, 2);
    }

    #[test]
    fn leaves_non_crossing_limit_order_resting() {
        let mut book = OrderBook::new("BTC-USD");
        let outcome = book.submit(limit(Side::Buy, 99, 10)).unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::Resting);
        assert_eq!(book.snapshot(5).best_bid, Some(99));
        assert_eq!(
            statuses(book.get_order_history(outcome.ack.order.id)),
            vec![OrderStatus::Accepted, OrderStatus::Resting]
        );
    }

    #[test]
    fn market_order_does_not_rest() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Sell, 100, 2)).unwrap();

        let outcome = book
            .submit(NewOrder {
                side: Side::Buy,
                kind: OrderKind::Market,
                price: None,
                quantity: 5,
            })
            .unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::PartiallyFilled);
        assert_eq!(book.snapshot(5).best_ask, None);
    }

    #[test]
    fn rejects_invalid_tick_and_lot() {
        let mut book = OrderBook::with_config(BookConfig {
            tick_size: 5,
            lot_size: 10,
            ..BookConfig::btc_usd()
        });

        assert_eq!(
            book.submit(limit(Side::Buy, 101, 10)).unwrap_err(),
            MatchError::InvalidTick
        );
        assert_eq!(
            book.submit(limit(Side::Buy, 100, 11)).unwrap_err(),
            MatchError::InvalidLot
        );
    }

    #[test]
    fn records_maker_and_taker_lifecycle_for_partial_fill() {
        let mut book = OrderBook::new("BTC-USD");
        let maker = book.submit(limit(Side::Sell, 100, 10)).unwrap().ack.order;

        let taker_outcome = book.submit(limit(Side::Buy, 100, 4)).unwrap();

        assert_eq!(taker_outcome.ack.status, OrderStatus::Filled);
        assert_eq!(
            book.get_order_history(maker.id),
            vec![
                OrderHistoryEntry {
                    sequence: 1,
                    status: OrderStatus::Accepted,
                    remaining_quantity: 10,
                },
                OrderHistoryEntry {
                    sequence: 1,
                    status: OrderStatus::Resting,
                    remaining_quantity: 10,
                },
                OrderHistoryEntry {
                    sequence: 3,
                    status: OrderStatus::PartiallyFilled,
                    remaining_quantity: 6,
                },
            ]
        );
        assert_eq!(
            statuses(book.get_order_history(taker_outcome.ack.order.id)),
            vec![OrderStatus::Accepted, OrderStatus::Filled]
        );
        assert_eq!(book.active_order_count(), 1);
    }

    #[test]
    fn records_cancelled_lifecycle_and_removes_order_from_active_index() {
        let mut book = OrderBook::new("BTC-USD");
        let order = book.submit(limit(Side::Buy, 99, 10)).unwrap().ack.order;

        let cancelled = book.cancel(order.id).unwrap();

        assert_eq!(cancelled.id, order.id);
        assert_eq!(book.active_order_count(), 0);
        assert_eq!(
            statuses(book.get_order_history(order.id)),
            vec![
                OrderStatus::Accepted,
                OrderStatus::Resting,
                OrderStatus::Cancelled
            ]
        );
    }

    #[test]
    fn records_rejected_validation_lifecycle() {
        let mut book = OrderBook::with_config(BookConfig {
            tick_size: 5,
            ..BookConfig::btc_usd()
        });

        let err = book.submit(limit(Side::Buy, 101, 10)).unwrap_err();

        assert_eq!(err, MatchError::InvalidTick);
        assert_eq!(
            statuses(book.get_order_history(1)),
            vec![OrderStatus::Rejected]
        );
    }

    fn statuses(history: Vec<OrderHistoryEntry>) -> Vec<OrderStatus> {
        history.into_iter().map(|entry| entry.status).collect()
    }
}
