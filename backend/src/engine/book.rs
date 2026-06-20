use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, VecDeque},
};

use crate::{
    engine::{
        MatchError,
        order_state::{self, OrderHistory},
        price_level::PriceLevel,
        reject_reason,
        sequencer::InMemorySequencer,
        snapshot, trade,
    },
    model::{
        BookSnapshot, EngineEvent, EngineMetrics, EngineSnapshot, MassCancelAck, NewOrder, Order,
        OrderAck, OrderHistoryEntry, OrderHistorySnapshot, OrderId, OrderKind, OrderStatus, Price,
        Quantity, ReplaceAck, ReplaceOrder, Side, TimeInForce, Trade,
    },
};

#[derive(Debug, Clone)]
pub struct BookConfig {
    pub symbol: String,
    pub tick_size: Price,
    pub lot_size: Quantity,
    pub max_recent_trades: usize,
    pub default_depth: usize,
    pub risk: RiskConfig,
}

#[derive(Debug, Clone, Default)]
pub struct RiskConfig {
    pub max_order_quantity: Option<Quantity>,
    pub max_order_notional: Option<u64>,
    pub max_open_orders: Option<usize>,
}

impl BookConfig {
    pub fn btc_usd() -> Self {
        Self {
            symbol: "BTC-USD".to_string(),
            tick_size: 1,
            lot_size: 1,
            max_recent_trades: 256,
            default_depth: 25,
            risk: RiskConfig {
                max_order_quantity: Some(1_000_000),
                max_order_notional: Some(10_000_000_000),
                max_open_orders: Some(100_000),
            },
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
    pub(super) config: BookConfig,
    pub(super) next_order_id: OrderId,
    pub(super) next_trade_id: u64,
    pub(super) sequencer: InMemorySequencer,
    pub(super) bids: BTreeMap<Reverse<Price>, PriceLevel>,
    pub(super) asks: BTreeMap<Price, PriceLevel>,
    pub(super) order_index: HashMap<OrderId, (Side, Price)>,
    pub(super) stop_orders: HashMap<OrderId, Order>,
    pub(super) order_history: OrderHistory,
    pub(super) recent_trades: VecDeque<Trade>,
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
            sequencer: InMemorySequencer::default(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            order_index: HashMap::new(),
            stop_orders: HashMap::new(),
            order_history: HashMap::new(),
            recent_trades: VecDeque::with_capacity(256),
        }
    }

    pub fn submit(&mut self, request: NewOrder) -> Result<MatchOutcome, MatchError> {
        if let Err(err) = reject_reason::validate_order(
            &request,
            self.config.tick_size,
            self.config.lot_size,
            self.active_order_count(),
            &self.config.risk,
        ) {
            let order_id = self.allocate_order_id();
            let rejected = Order {
                id: order_id,
                account_id: request.account_id,
                side: request.side,
                kind: request.kind,
                time_in_force: request.time_in_force,
                price: request.price,
                stop_price: request.stop_price,
                original_quantity: request.quantity,
                remaining_quantity: request.quantity,
                original_quote_quantity: request.quote_quantity,
                remaining_quote_quantity: request.quote_quantity,
                created_at_seq: self.next_sequence(),
            };
            self.record_status(&rejected, OrderStatus::Rejected);
            return Err(err);
        }

        let price = match request.kind {
            OrderKind::Limit | OrderKind::PostOnly | OrderKind::StopLimit => {
                Some(request.price.ok_or(MatchError::MissingLimitPrice)?)
            }
            OrderKind::Market | OrderKind::MarketByNotional | OrderKind::StopMarket => None,
        };

        let order_id = self.allocate_order_id();
        let taker = Order {
            id: order_id,
            account_id: request.account_id,
            side: request.side,
            kind: request.kind,
            time_in_force: request.time_in_force,
            price,
            stop_price: request.stop_price,
            original_quantity: request.quantity,
            remaining_quantity: request.quantity,
            original_quote_quantity: request.quote_quantity,
            remaining_quote_quantity: request.quote_quantity,
            created_at_seq: self.next_sequence(),
        };
        self.record_status(&taker, OrderStatus::Accepted);

        if is_stop_order(taker.kind) {
            self.stop_orders.insert(taker.id, taker.clone());
            self.record_status(&taker, OrderStatus::Resting);
            let ack = OrderAck {
                order: taker,
                status: OrderStatus::Resting,
                trades: Vec::new(),
            };
            let events = vec![
                EngineEvent::Order { data: ack.clone() },
                EngineEvent::Book {
                    data: self.snapshot(self.config.default_depth),
                },
            ];
            return Ok(MatchOutcome { ack, events });
        }

        Ok(self.execute_taker(taker))
    }

    fn execute_taker(&mut self, mut taker: Order) -> MatchOutcome {
        if taker.kind == OrderKind::PostOnly && self.would_cross(&taker) {
            self.record_status(&taker, OrderStatus::Rejected);
            let ack = OrderAck {
                order: taker,
                status: OrderStatus::Rejected,
                trades: Vec::new(),
            };
            let events = vec![
                EngineEvent::Order { data: ack.clone() },
                EngineEvent::Book {
                    data: self.snapshot(self.config.default_depth),
                },
            ];
            return MatchOutcome { ack, events };
        }

        let trades = match taker.side {
            Side::Buy => self.match_buy(&mut taker),
            Side::Sell => self.match_sell(&mut taker),
        };
        let blocked_by_self_trade = taker.remaining_quantity > 0
            && taker.kind == OrderKind::Limit
            && taker.time_in_force == TimeInForce::Gtc
            && self.crosses_own_liquidity(&taker);

        let status = if taker_is_filled(&taker) {
            OrderStatus::Filled
        } else if taker.kind == OrderKind::Market
            || taker.kind == OrderKind::MarketByNotional
            || taker.time_in_force == TimeInForce::Ioc
            || blocked_by_self_trade
        {
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

        let trigger_price = ack.trades.last().map(|trade| trade.price);
        if let Some(trigger_price) = trigger_price {
            events.extend(self.trigger_stop_orders(trigger_price));
        }

        MatchOutcome { ack, events }
    }

    pub fn cancel(&mut self, order_id: OrderId) -> Result<Order, MatchError> {
        let Some((side, price)) = self.order_index.remove(&order_id) else {
            let cancelled = self
                .stop_orders
                .remove(&order_id)
                .ok_or(MatchError::OrderNotFound)?;
            self.next_sequence();
            self.record_status(&cancelled, OrderStatus::Cancelled);
            return Ok(cancelled);
        };

        let removed = match side {
            Side::Buy => remove_from_level(&mut self.bids, Reverse(price), order_id),
            Side::Sell => remove_from_level(&mut self.asks, price, order_id),
        };
        let cancelled = removed.ok_or(MatchError::OrderNotFound)?;
        self.next_sequence();
        self.record_status(&cancelled, OrderStatus::Cancelled);
        Ok(cancelled)
    }

    pub fn cancel_all(&mut self) -> Vec<Order> {
        let order_ids: Vec<OrderId> = self
            .order_index
            .keys()
            .chain(self.stop_orders.keys())
            .copied()
            .collect();
        order_ids
            .into_iter()
            .filter_map(|order_id| self.cancel(order_id).ok())
            .collect()
    }

    pub fn mass_cancel_events(cancelled: &[Order], snapshot: BookSnapshot) -> Vec<EngineEvent> {
        vec![
            EngineEvent::MassCancel {
                data: MassCancelAck {
                    cancelled_order_ids: cancelled.iter().map(|order| order.id).collect(),
                },
            },
            EngineEvent::Book { data: snapshot },
        ]
    }

    pub fn replace(
        &mut self,
        order_id: OrderId,
        replacement: ReplaceOrder,
    ) -> Result<MatchOutcome, MatchError> {
        self.cancel(order_id)?;
        let mut outcome = self.submit(replacement.into())?;
        outcome.events.insert(
            0,
            EngineEvent::Replace {
                data: ReplaceAck {
                    cancelled_order_id: order_id,
                    replacement: outcome.ack.clone(),
                },
            },
        );
        Ok(outcome)
    }

    pub fn snapshot(&self, depth: usize) -> BookSnapshot {
        snapshot::build_snapshot(
            &self.config.symbol,
            self.sequencer.current(),
            &self.bids,
            &self.asks,
            depth,
        )
    }

    pub fn capture_snapshot(&self) -> EngineSnapshot {
        let mut order_history = self
            .order_history
            .iter()
            .map(|(order_id, entries)| OrderHistorySnapshot {
                order_id: *order_id,
                entries: entries.clone(),
            })
            .collect::<Vec<_>>();
        order_history.sort_by_key(|history| history.order_id);

        EngineSnapshot {
            symbol: self.config.symbol.clone(),
            sequence: self.sequencer.current(),
            next_order_id: self.next_order_id,
            next_trade_id: self.next_trade_id,
            active_orders: self.active_orders(),
            recent_trades: self.recent_trades.iter().cloned().collect(),
            order_history,
        }
    }

    pub fn restore_from_snapshot(mut config: BookConfig, snapshot: EngineSnapshot) -> Self {
        config.symbol = snapshot.symbol;
        let mut book = Self::with_config(config);
        book.sequencer.advance_to(snapshot.sequence);
        book.next_order_id = snapshot.next_order_id;
        book.next_trade_id = snapshot.next_trade_id;
        book.recent_trades = snapshot.recent_trades.into_iter().collect();
        book.order_history = snapshot
            .order_history
            .into_iter()
            .map(|history| (history.order_id, history.entries))
            .collect();

        for order in snapshot.active_orders {
            book.rest_active_order(order);
        }
        book
    }

    pub fn replay(config: BookConfig, events: impl IntoIterator<Item = EngineEvent>) -> Self {
        let mut book = Self::with_config(config);
        book.apply_replay_events(events);
        book
    }

    pub fn apply_replay_events(&mut self, events: impl IntoIterator<Item = EngineEvent>) {
        for event in events {
            self.apply_replay_event(event);
        }
    }

    pub fn config(&self) -> &BookConfig {
        &self.config
    }

    pub fn metrics(&self) -> EngineMetrics {
        EngineMetrics {
            symbol: self.config.symbol.clone(),
            sequence: self.sequencer.current(),
            active_order_count: self.active_order_count(),
            bid_level_count: self.bids.len(),
            ask_level_count: self.asks.len(),
            bid_depth: self.bids.values().map(PriceLevel::depth).sum(),
            ask_depth: self.asks.values().map(PriceLevel::depth).sum(),
            recent_trade_count: self.recent_trades.len(),
            order_history_count: self.order_history.len(),
            order_history_entry_count: self.order_history.values().map(std::vec::Vec::len).sum(),
            next_order_id: self.next_order_id,
            next_trade_id: self.next_trade_id,
        }
    }

    pub fn engine_seq(&self) -> u64 {
        self.sequencer.current()
    }

    pub fn get_order_history(&self, order_id: OrderId) -> Vec<OrderHistoryEntry> {
        self.order_history
            .get(&order_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn active_order_count(&self) -> usize {
        self.order_index.len() + self.stop_orders.len()
    }

    pub fn active_orders(&self) -> Vec<Order> {
        self.stop_orders
            .values()
            .chain(
                self.bids
                    .values()
                    .chain(self.asks.values())
                    .flat_map(PriceLevel::orders),
            )
            .cloned()
            .collect()
    }

    pub fn recent_trades(&self) -> Vec<Trade> {
        self.recent_trades.iter().cloned().collect()
    }

    fn rest(&mut self, order: Order) {
        self.rest_active_order(order);
    }

    fn rest_active_order(&mut self, order: Order) {
        if is_stop_order(order.kind) {
            self.stop_orders.insert(order.id, order);
            return;
        }
        let Some(price) = order.price else {
            return;
        };
        self.order_index.insert(order.id, (order.side, price));
        match order.side {
            Side::Buy => self.bids.entry(Reverse(price)).or_default().push(order),
            Side::Sell => self.asks.entry(price).or_default().push(order),
        }
    }

    fn trigger_stop_orders(&mut self, trade_price: Price) -> Vec<EngineEvent> {
        let mut triggered = self
            .stop_orders
            .values()
            .filter(|order| stop_triggered(order, trade_price))
            .map(|order| order.id)
            .collect::<Vec<_>>();
        triggered.sort_unstable();

        let mut events = Vec::new();
        for order_id in triggered {
            let Some(mut order) = self.stop_orders.remove(&order_id) else {
                continue;
            };
            order.kind = match order.kind {
                OrderKind::StopLimit => OrderKind::Limit,
                OrderKind::StopMarket => OrderKind::Market,
                kind => kind,
            };
            let outcome = self.execute_taker(order);
            events.extend(outcome.events);
        }
        events
    }

    fn would_cross(&self, order: &Order) -> bool {
        match order.side {
            Side::Buy => order
                .price
                .zip(self.asks.keys().next().copied())
                .is_some_and(|(limit, best_ask)| limit >= best_ask),
            Side::Sell => order
                .price
                .zip(self.bids.keys().next().map(|price| price.0))
                .is_some_and(|(limit, best_bid)| limit <= best_bid),
        }
    }

    fn crosses_own_liquidity(&self, order: &Order) -> bool {
        match order.side {
            Side::Buy => self.asks.iter().any(|(price, level)| {
                order.price.is_some_and(|limit| *price <= limit)
                    && level.has_order_for_account(&order.account_id)
            }),
            Side::Sell => self.bids.iter().any(|(price, level)| {
                order.price.is_some_and(|limit| price.0 >= limit)
                    && level.has_order_for_account(&order.account_id)
            }),
        }
    }

    fn apply_replay_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Order { data } => self.replay_order_ack(data),
            EngineEvent::Trade { data } => self.replay_trade(data),
            EngineEvent::Cancel { order_id } => {
                self.remove_active_order(order_id);
            }
            EngineEvent::MassCancel { data } => {
                for order_id in data.cancelled_order_ids {
                    self.remove_active_order(order_id);
                }
            }
            EngineEvent::Replace { data } => {
                self.remove_active_order(data.cancelled_order_id);
            }
            EngineEvent::Book { data } => {
                self.sequencer.advance_to(data.sequence);
            }
        }
    }

    fn replay_order_ack(&mut self, ack: OrderAck) {
        self.sequencer.advance_to(ack.order.created_at_seq);
        self.next_order_id = self.next_order_id.max(ack.order.id + 1);
        self.record_status(&ack.order, ack.status);
        if matches!(
            ack.status,
            OrderStatus::Resting | OrderStatus::PartiallyFilled
        ) && ack.order.remaining_quantity > 0
        {
            self.rest(ack.order);
        }
    }

    fn replay_trade(&mut self, trade: Trade) {
        self.sequencer.advance_to(trade.sequence);
        self.next_trade_id = self.next_trade_id.max(trade.id + 1);
        self.reduce_active_order(trade.maker_order_id, trade.quantity);
        trade::retain_recent_trades(
            &mut self.recent_trades,
            self.config.max_recent_trades,
            trade,
        );
    }

    fn reduce_active_order(&mut self, order_id: OrderId, quantity: Quantity) {
        let Some((side, price)) = self.order_index.get(&order_id).copied() else {
            return;
        };
        let (remaining_quantity, filled, level_empty) = match side {
            Side::Buy => reduce_level_order(&mut self.bids, Reverse(price), order_id, quantity),
            Side::Sell => reduce_level_order(&mut self.asks, price, order_id, quantity),
        };

        if filled {
            self.order_index.remove(&order_id);
        }
        if level_empty {
            match side {
                Side::Buy => {
                    self.bids.remove(&Reverse(price));
                }
                Side::Sell => {
                    self.asks.remove(&price);
                }
            }
        }
        self.record_maker_fill(order_id, remaining_quantity, filled);
    }

    fn remove_active_order(&mut self, order_id: OrderId) -> Option<Order> {
        if let Some(order) = self.stop_orders.remove(&order_id) {
            return Some(order);
        }
        let (side, price) = self.order_index.remove(&order_id)?;
        match side {
            Side::Buy => remove_from_level(&mut self.bids, Reverse(price), order_id),
            Side::Sell => remove_from_level(&mut self.asks, price, order_id),
        }
    }

    pub(super) fn trade(
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
        self.sequencer.next_sequence()
    }

    fn record_status(&mut self, order: &Order, status: OrderStatus) {
        order_state::record_status(
            &mut self.order_history,
            self.sequencer.current(),
            order,
            status,
        );
    }

    pub(super) fn record_maker_fill(
        &mut self,
        order_id: OrderId,
        remaining_quantity: Quantity,
        maker_filled: bool,
    ) {
        order_state::record_fill(
            &mut self.order_history,
            self.sequencer.current(),
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

fn reduce_level_order<K: Ord>(
    book: &mut BTreeMap<K, PriceLevel>,
    price: K,
    order_id: OrderId,
    quantity: Quantity,
) -> (Quantity, bool, bool) {
    let Some(level) = book.get_mut(&price) else {
        return (0, false, false);
    };
    let Some(order) = level.get_mut(order_id) else {
        return (0, false, level.is_empty());
    };
    order.remaining_quantity = order.remaining_quantity.saturating_sub(quantity);
    let remaining_quantity = order.remaining_quantity;
    let filled = remaining_quantity == 0;
    if filled {
        level.remove(order_id);
    }
    (remaining_quantity, filled, level.is_empty())
}

fn taker_is_filled(taker: &Order) -> bool {
    match taker.kind {
        OrderKind::MarketByNotional => taker.remaining_quote_quantity == Some(0),
        _ => taker.remaining_quantity == 0,
    }
}

fn is_stop_order(kind: OrderKind) -> bool {
    matches!(kind, OrderKind::StopLimit | OrderKind::StopMarket)
}

fn stop_triggered(order: &Order, trade_price: Price) -> bool {
    let Some(stop_price) = order.stop_price else {
        return false;
    };
    match order.side {
        Side::Buy => trade_price >= stop_price,
        Side::Sell => trade_price <= stop_price,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn limit(side: Side, price: Price, quantity: Quantity) -> NewOrder {
        limit_for("", side, price, quantity)
    }

    fn limit_for(account_id: &str, side: Side, price: Price, quantity: Quantity) -> NewOrder {
        NewOrder {
            account_id: account_id.to_string(),
            side,
            kind: OrderKind::Limit,
            time_in_force: TimeInForce::Gtc,
            price: Some(price),
            stop_price: None,
            quantity,
            quote_quantity: None,
        }
    }

    fn post_only(side: Side, price: Price, quantity: Quantity) -> NewOrder {
        NewOrder {
            account_id: String::new(),
            side,
            kind: OrderKind::PostOnly,
            time_in_force: TimeInForce::Gtc,
            price: Some(price),
            stop_price: None,
            quantity,
            quote_quantity: None,
        }
    }

    fn market_by_notional(side: Side, quote_quantity: Quantity) -> NewOrder {
        NewOrder {
            account_id: String::new(),
            side,
            kind: OrderKind::MarketByNotional,
            time_in_force: TimeInForce::Gtc,
            price: None,
            stop_price: None,
            quantity: 0,
            quote_quantity: Some(quote_quantity),
        }
    }

    fn stop_limit(side: Side, stop_price: Price, price: Price, quantity: Quantity) -> NewOrder {
        NewOrder {
            account_id: String::new(),
            side,
            kind: OrderKind::StopLimit,
            time_in_force: TimeInForce::Gtc,
            price: Some(price),
            stop_price: Some(stop_price),
            quantity,
            quote_quantity: None,
        }
    }

    fn stop_market(side: Side, stop_price: Price, quantity: Quantity) -> NewOrder {
        NewOrder {
            account_id: String::new(),
            side,
            kind: OrderKind::StopMarket,
            time_in_force: TimeInForce::Gtc,
            price: None,
            stop_price: Some(stop_price),
            quantity,
            quote_quantity: None,
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
    fn prevents_self_trade_and_does_not_rest_crossing_remainder() {
        let mut book = OrderBook::new("BTC-USD");
        let maker = book
            .submit(limit_for("acct-a", Side::Sell, 101, 3))
            .unwrap()
            .ack
            .order;

        let outcome = book.submit(limit_for("acct-a", Side::Buy, 101, 3)).unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::Rejected);
        assert!(outcome.ack.trades.is_empty());
        assert_eq!(book.active_order_count(), 1);
        assert_eq!(book.active_orders()[0].id, maker.id);
        assert_eq!(book.snapshot(5).best_bid, None);
        assert_eq!(book.snapshot(5).best_ask, Some(101));
    }

    #[test]
    fn skips_own_liquidity_and_matches_other_account_at_same_price() {
        let mut book = OrderBook::new("BTC-USD");
        let own = book
            .submit(limit_for("acct-a", Side::Sell, 101, 3))
            .unwrap()
            .ack
            .order;
        let other = book
            .submit(limit_for("acct-b", Side::Sell, 101, 4))
            .unwrap()
            .ack
            .order;

        let outcome = book.submit(limit_for("acct-a", Side::Buy, 101, 4)).unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::Filled);
        assert_eq!(outcome.ack.trades.len(), 1);
        assert_eq!(outcome.ack.trades[0].maker_order_id, other.id);
        assert_eq!(book.active_order_count(), 1);
        assert_eq!(book.active_orders()[0].id, own.id);
        assert_eq!(book.snapshot(5).ask_depth, 3);
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
                account_id: String::new(),
                side: Side::Buy,
                kind: OrderKind::Market,
                time_in_force: TimeInForce::Gtc,
                price: None,
                stop_price: None,
                quantity: 5,
                quote_quantity: None,
            })
            .unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::PartiallyFilled);
        assert_eq!(book.snapshot(5).best_ask, None);
    }

    #[test]
    fn market_by_notional_buys_with_quote_budget() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Sell, 100, 3)).unwrap();
        book.submit(limit(Side::Sell, 110, 3)).unwrap();

        let outcome = book.submit(market_by_notional(Side::Buy, 410)).unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::Filled);
        assert_eq!(outcome.ack.order.original_quote_quantity, Some(410));
        assert_eq!(outcome.ack.order.remaining_quote_quantity, Some(0));
        assert_eq!(outcome.ack.trades.len(), 2);
        assert_eq!(outcome.ack.trades[0].quantity, 3);
        assert_eq!(outcome.ack.trades[1].quantity, 1);
        assert_eq!(book.snapshot(5).ask_depth, 2);
    }

    #[test]
    fn market_by_notional_rejects_unsupported_sell_side() {
        let mut book = OrderBook::new("BTC-USD");

        let err = book
            .submit(market_by_notional(Side::Sell, 100))
            .unwrap_err();

        assert_eq!(err, MatchError::UnsupportedSide);
    }

    #[test]
    fn stop_limit_rests_dormant_then_triggers_after_trade_price_crosses_stop() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Sell, 100, 1)).unwrap();
        let stop = book.submit(stop_limit(Side::Buy, 100, 101, 2)).unwrap().ack;

        assert_eq!(stop.status, OrderStatus::Resting);
        assert_eq!(stop.order.kind, OrderKind::StopLimit);
        assert_eq!(book.active_order_count(), 2);
        assert_eq!(book.snapshot(5).bid_depth, 0);

        let trigger = book.submit(limit(Side::Buy, 100, 1)).unwrap();

        assert_eq!(trigger.ack.status, OrderStatus::Filled);
        assert_eq!(book.active_orders().len(), 1);
        let triggered_order = book.active_orders().pop().unwrap();
        assert_eq!(triggered_order.id, stop.order.id);
        assert_eq!(triggered_order.kind, OrderKind::Limit);
        assert_eq!(triggered_order.price, Some(101));
        assert_eq!(book.snapshot(5).best_bid, Some(101));
    }

    #[test]
    fn stop_market_triggers_into_market_execution() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Sell, 100, 1)).unwrap();
        let stop = book
            .submit(stop_market(Side::Buy, 100, 2))
            .unwrap()
            .ack
            .order;
        book.submit(limit(Side::Sell, 101, 2)).unwrap();

        let trigger = book.submit(limit(Side::Buy, 100, 1)).unwrap();

        assert_eq!(trigger.ack.status, OrderStatus::Filled);
        assert!(
            book.get_order_history(stop.id)
                .iter()
                .any(|entry| entry.status == OrderStatus::Filled)
        );
        assert_eq!(book.snapshot(5).ask_depth, 0);
    }

    #[test]
    fn post_only_rests_when_it_does_not_cross() {
        let mut book = OrderBook::new("BTC-USD");

        let outcome = book.submit(post_only(Side::Buy, 99, 10)).unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::Resting);
        assert_eq!(outcome.ack.order.kind, OrderKind::PostOnly);
        assert_eq!(book.snapshot(5).best_bid, Some(99));
        assert_eq!(book.active_order_count(), 1);
    }

    #[test]
    fn post_only_rejects_when_it_would_cross() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Sell, 100, 2)).unwrap();

        let outcome = book.submit(post_only(Side::Buy, 100, 5)).unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::Rejected);
        assert!(outcome.ack.trades.is_empty());
        assert_eq!(book.snapshot(5).best_ask, Some(100));
        assert_eq!(book.active_order_count(), 1);
        assert_eq!(
            statuses(book.get_order_history(outcome.ack.order.id)),
            vec![OrderStatus::Accepted, OrderStatus::Rejected]
        );
    }

    #[test]
    fn restores_complete_engine_snapshot() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Buy, 99, 10)).unwrap();
        book.submit(limit(Side::Sell, 101, 7)).unwrap();

        let restored =
            OrderBook::restore_from_snapshot(BookConfig::btc_usd(), book.capture_snapshot());

        assert_eq!(restored.engine_seq(), book.engine_seq());
        assert_eq!(restored.active_order_count(), book.active_order_count());
        assert_eq!(restored.snapshot(5).bid_depth, 10);
        assert_eq!(restored.snapshot(5).ask_depth, 7);
        assert_eq!(restored.get_order_history(1), book.get_order_history(1));
    }

    #[test]
    fn replays_order_trade_and_cancel_events_deterministically() {
        let mut book = OrderBook::new("BTC-USD");
        let mut events = Vec::new();
        let resting = book.submit(limit(Side::Buy, 100, 5)).unwrap();
        events.extend(resting.events);
        let crossing = book.submit(limit(Side::Sell, 99, 2)).unwrap();
        events.extend(crossing.events);
        let cancelled = book.cancel(1).unwrap();
        let snapshot = book.snapshot(5);
        events.push(EngineEvent::Cancel {
            order_id: cancelled.id,
        });
        events.push(EngineEvent::Book { data: snapshot });

        let replayed = OrderBook::replay(BookConfig::btc_usd(), events);

        assert_eq!(replayed.engine_seq(), book.engine_seq());
        assert_eq!(replayed.active_order_count(), book.active_order_count());
        assert_eq!(replayed.snapshot(5).best_bid, book.snapshot(5).best_bid);
        assert_eq!(replayed.snapshot(5).bid_depth, book.snapshot(5).bid_depth);
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
    fn rejects_orders_that_breach_risk_limits() {
        let mut book = OrderBook::with_config(BookConfig {
            risk: RiskConfig {
                max_order_quantity: Some(100),
                max_order_notional: Some(10_000),
                max_open_orders: Some(1),
            },
            ..BookConfig::btc_usd()
        });

        assert_eq!(
            book.submit(limit(Side::Buy, 100, 101)).unwrap_err(),
            MatchError::MaxOrderQuantityExceeded
        );
        assert_eq!(
            book.submit(limit(Side::Buy, 101, 100)).unwrap_err(),
            MatchError::MaxOrderNotionalExceeded
        );

        book.submit(limit(Side::Buy, 99, 10)).unwrap();
        assert_eq!(
            book.submit(limit(Side::Sell, 101, 10)).unwrap_err(),
            MatchError::MaxOpenOrdersExceeded
        );
    }

    #[test]
    fn snapshot_includes_depth_and_spread_metrics() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Buy, 99, 10)).unwrap();
        book.submit(limit(Side::Buy, 98, 5)).unwrap();
        book.submit(limit(Side::Sell, 101, 7)).unwrap();

        let snapshot = book.snapshot(5);

        assert_eq!(snapshot.best_bid, Some(99));
        assert_eq!(snapshot.best_ask, Some(101));
        assert_eq!(snapshot.spread, Some(2));
        assert_eq!(snapshot.mid_price, Some(100.0));
        assert_eq!(snapshot.bid_depth, 15);
        assert_eq!(snapshot.ask_depth, 7);
        assert_eq!(snapshot.bid_order_count, 2);
        assert_eq!(snapshot.ask_order_count, 1);
    }

    #[test]
    fn metrics_report_engine_shape_and_history_size() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Buy, 99, 10)).unwrap();
        book.submit(limit(Side::Sell, 101, 7)).unwrap();

        let metrics = book.metrics();

        assert_eq!(metrics.symbol, "BTC-USD");
        assert_eq!(metrics.sequence, 2);
        assert_eq!(metrics.active_order_count, 2);
        assert_eq!(metrics.bid_level_count, 1);
        assert_eq!(metrics.ask_level_count, 1);
        assert_eq!(metrics.bid_depth, 10);
        assert_eq!(metrics.ask_depth, 7);
        assert_eq!(metrics.recent_trade_count, 0);
        assert_eq!(metrics.order_history_count, 2);
        assert_eq!(metrics.order_history_entry_count, 4);
        assert_eq!(metrics.next_order_id, 3);
        assert_eq!(metrics.next_trade_id, 1);
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

    #[test]
    fn exposes_active_resting_orders() {
        let mut book = OrderBook::new("BTC-USD");
        let buy = book.submit(limit(Side::Buy, 99, 10)).unwrap().ack.order.id;
        let sell = book.submit(limit(Side::Sell, 101, 5)).unwrap().ack.order.id;

        let active = book.active_orders();

        assert_eq!(active.len(), 2);
        assert!(active.iter().any(|order| order.id == buy));
        assert!(active.iter().any(|order| order.id == sell));
        assert_eq!(book.active_order_count(), active.len());
    }

    #[test]
    fn ioc_limit_matches_immediately_without_resting_remainder() {
        let mut book = OrderBook::new("BTC-USD");
        book.submit(limit(Side::Sell, 100, 2)).unwrap();

        let outcome = book
            .submit(NewOrder {
                account_id: String::new(),
                side: Side::Buy,
                kind: OrderKind::Limit,
                time_in_force: TimeInForce::Ioc,
                price: Some(100),
                stop_price: None,
                quantity: 5,
                quote_quantity: None,
            })
            .unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::PartiallyFilled);
        assert_eq!(outcome.ack.order.remaining_quantity, 3);
        assert_eq!(book.active_order_count(), 0);
        assert!(book.snapshot(5).bids.is_empty());
        assert_eq!(
            statuses(book.get_order_history(outcome.ack.order.id)),
            vec![OrderStatus::Accepted, OrderStatus::PartiallyFilled]
        );
    }

    #[test]
    fn replace_cancels_existing_order_and_submits_replacement() {
        let mut book = OrderBook::new("BTC-USD");
        let original = book.submit(limit(Side::Buy, 99, 10)).unwrap().ack.order.id;

        let outcome = book
            .replace(
                original,
                ReplaceOrder {
                    account_id: String::new(),
                    side: Side::Buy,
                    kind: OrderKind::Limit,
                    time_in_force: TimeInForce::Gtc,
                    price: Some(100),
                    stop_price: None,
                    quantity: 4,
                    quote_quantity: None,
                },
            )
            .unwrap();

        assert_eq!(outcome.ack.status, OrderStatus::Resting);
        assert_ne!(outcome.ack.order.id, original);
        assert_eq!(book.active_order_count(), 1);
        assert_eq!(book.snapshot(5).best_bid, Some(100));
        assert_eq!(
            statuses(book.get_order_history(original)),
            vec![
                OrderStatus::Accepted,
                OrderStatus::Resting,
                OrderStatus::Cancelled
            ]
        );
    }

    #[test]
    fn mass_cancel_removes_all_active_orders() {
        let mut book = OrderBook::new("BTC-USD");
        let buy = book.submit(limit(Side::Buy, 99, 10)).unwrap().ack.order.id;
        let sell = book.submit(limit(Side::Sell, 101, 5)).unwrap().ack.order.id;

        let cancelled = book.cancel_all();

        assert_eq!(cancelled.len(), 2);
        assert_eq!(book.active_order_count(), 0);
        assert!(book.snapshot(5).bids.is_empty());
        assert!(book.snapshot(5).asks.is_empty());
        assert_eq!(
            statuses(book.get_order_history(buy)),
            vec![
                OrderStatus::Accepted,
                OrderStatus::Resting,
                OrderStatus::Cancelled,
            ]
        );
        assert_eq!(
            statuses(book.get_order_history(sell)),
            vec![
                OrderStatus::Accepted,
                OrderStatus::Resting,
                OrderStatus::Cancelled,
            ]
        );
    }

    fn statuses(history: Vec<OrderHistoryEntry>) -> Vec<OrderStatus> {
        history.into_iter().map(|entry| entry.status).collect()
    }
}
