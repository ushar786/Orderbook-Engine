use std::collections::VecDeque;

use crate::model::{OrderId, Price, Quantity, Side, Trade};

pub fn create_trade(
    next_trade_id: &mut u64,
    sequence: u64,
    maker_order_id: OrderId,
    taker_order_id: OrderId,
    price: Price,
    quantity: Quantity,
    aggressor_side: Side,
) -> Trade {
    let trade = Trade {
        id: *next_trade_id,
        maker_order_id,
        taker_order_id,
        price,
        quantity,
        aggressor_side,
        sequence,
    };
    *next_trade_id += 1;
    trade
}

pub fn retain_recent_trades(
    recent_trades: &mut VecDeque<Trade>,
    max_recent_trades: usize,
    trade: Trade,
) {
    if recent_trades.len() == max_recent_trades {
        recent_trades.pop_front();
    }
    recent_trades.push_back(trade);
}
