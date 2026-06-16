use std::collections::HashMap;

use crate::model::{Order, OrderHistoryEntry, OrderId, OrderStatus, Quantity};

pub type OrderHistory = HashMap<OrderId, Vec<OrderHistoryEntry>>;

pub fn record_status(
    history: &mut OrderHistory,
    sequence: u64,
    order: &Order,
    status: OrderStatus,
) {
    history
        .entry(order.id)
        .or_default()
        .push(OrderHistoryEntry {
            sequence,
            status,
            remaining_quantity: order.remaining_quantity,
        });
}

pub fn record_fill(
    history: &mut OrderHistory,
    sequence: u64,
    order_id: OrderId,
    remaining_quantity: Quantity,
    maker_filled: bool,
) {
    let status = if maker_filled {
        OrderStatus::Filled
    } else {
        OrderStatus::PartiallyFilled
    };
    history
        .entry(order_id)
        .or_default()
        .push(OrderHistoryEntry {
            sequence,
            status,
            remaining_quantity,
        });
}
