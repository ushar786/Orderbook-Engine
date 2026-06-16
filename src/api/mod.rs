mod errors;
mod routes;
mod websocket;

use tokio::sync::{Mutex, broadcast};

use orderbook_engine::{engine::OrderBook, model::EngineEvent};

use crate::db::Database;

pub use routes::router;
pub use websocket::handler as ws_handler;

pub struct AppState {
    pub book: Mutex<OrderBook>,
    pub db: Mutex<Database>,
    pub events: broadcast::Sender<EngineEvent>,
}
