use serde::{Deserialize, Serialize};

pub type OrderId = u64;
pub type Price = u64;
pub type Quantity = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderKind {
    Limit,
    Market,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeInForce {
    #[default]
    Gtc,
    Ioc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewOrder {
    pub side: Side,
    #[serde(rename = "type")]
    pub kind: OrderKind,
    pub quantity: Quantity,
    pub price: Option<Price>,
    #[serde(default)]
    pub time_in_force: TimeInForce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceOrder {
    pub side: Side,
    #[serde(rename = "type")]
    pub kind: OrderKind,
    pub quantity: Quantity,
    pub price: Option<Price>,
    #[serde(default)]
    pub time_in_force: TimeInForce,
}

impl From<ReplaceOrder> for NewOrder {
    fn from(value: ReplaceOrder) -> Self {
        Self {
            side: value.side,
            kind: value.kind,
            quantity: value.quantity,
            price: value.price,
            time_in_force: value.time_in_force,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: OrderId,
    pub side: Side,
    pub kind: OrderKind,
    pub time_in_force: TimeInForce,
    pub price: Option<Price>,
    pub original_quantity: Quantity,
    pub remaining_quantity: Quantity,
    pub created_at_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: u64,
    pub maker_order_id: OrderId,
    pub taker_order_id: OrderId,
    pub price: Price,
    pub quantity: Quantity,
    pub aggressor_side: Side,
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Level {
    pub price: Price,
    pub quantity: Quantity,
    pub order_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookSnapshot {
    pub symbol: String,
    pub sequence: u64,
    pub best_bid: Option<Price>,
    pub best_ask: Option<Price>,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderAck {
    pub order: Order,
    pub status: OrderStatus,
    pub trades: Vec<Trade>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceAck {
    pub cancelled_order_id: OrderId,
    pub replacement: OrderAck,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MassCancelAck {
    pub cancelled_order_ids: Vec<OrderId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderHistoryEntry {
    pub sequence: u64,
    pub status: OrderStatus,
    pub remaining_quantity: Quantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Accepted,
    Filled,
    PartiallyFilled,
    Resting,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EngineEvent {
    Book { data: BookSnapshot },
    Order { data: OrderAck },
    Replace { data: ReplaceAck },
    Trade { data: Trade },
    Cancel { order_id: OrderId },
    MassCancel { data: MassCancelAck },
}
