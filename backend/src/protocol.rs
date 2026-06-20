use serde::{Deserialize, Serialize};

use crate::model::{BookSnapshot, EngineEvent, NewOrder, OrderId, ReplaceOrder, Trade};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboundMessage {
    SubmitOrder {
        data: NewOrder,
    },
    ReplaceOrder {
        order_id: OrderId,
        data: ReplaceOrder,
    },
    CancelOrder {
        order_id: OrderId,
    },
    SubscribeBook {
        depth: Option<usize>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboundMessage {
    Book { data: BookSnapshot },
    Event { data: EngineEvent },
    Trade { data: Trade },
    Error { message: String },
}

impl From<EngineEvent> for OutboundMessage {
    fn from(value: EngineEvent) -> Self {
        match value {
            EngineEvent::Book { data } => Self::Book { data },
            EngineEvent::Trade { data } => Self::Trade { data },
            event => Self::Event { data: event },
        }
    }
}
