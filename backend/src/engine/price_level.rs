use std::collections::VecDeque;

use crate::model::{Level, Order, OrderId, Price, Quantity};

#[derive(Debug, Default)]
pub struct PriceLevel {
    orders: VecDeque<Order>,
}

impl PriceLevel {
    pub fn push(&mut self, order: Order) {
        self.orders.push_back(order);
    }

    pub fn first_matchable_mut(&mut self, account_id: &str) -> Option<&mut Order> {
        self.orders
            .iter_mut()
            .find(|order| account_id.is_empty() || order.account_id != account_id)
    }

    pub fn has_matchable_order(&self, account_id: &str) -> bool {
        self.orders
            .iter()
            .any(|order| account_id.is_empty() || order.account_id != account_id)
    }

    pub fn has_order_for_account(&self, account_id: &str) -> bool {
        !account_id.is_empty()
            && self
                .orders
                .iter()
                .any(|order| order.account_id == account_id)
    }

    pub fn get_mut(&mut self, order_id: OrderId) -> Option<&mut Order> {
        self.orders.iter_mut().find(|order| order.id == order_id)
    }

    pub fn remove(&mut self, order_id: OrderId) -> Option<Order> {
        let index = self.orders.iter().position(|order| order.id == order_id)?;
        self.orders.remove(index)
    }

    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.iter()
    }

    pub fn len(&self) -> usize {
        self.orders.len()
    }

    pub fn snapshot(&self, price: Price) -> Level {
        Level {
            price,
            quantity: self.depth(),
            order_count: self.orders.len(),
        }
    }

    pub fn depth(&self) -> Quantity {
        self.orders
            .iter()
            .map(|order| order.remaining_quantity)
            .sum()
    }
}
