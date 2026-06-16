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

    pub fn front_mut(&mut self) -> Option<&mut Order> {
        self.orders.front_mut()
    }

    pub fn pop_front(&mut self) -> Option<Order> {
        self.orders.pop_front()
    }

    pub fn remove(&mut self, order_id: OrderId) -> Option<Order> {
        let index = self.orders.iter().position(|order| order.id == order_id)?;
        self.orders.remove(index)
    }

    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    pub fn snapshot(&self, price: Price) -> Level {
        Level {
            price,
            quantity: self.depth(),
            order_count: self.orders.len(),
        }
    }

    fn depth(&self) -> Quantity {
        self.orders
            .iter()
            .map(|order| order.remaining_quantity)
            .sum()
    }
}
