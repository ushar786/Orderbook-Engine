use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast};

use orderbook_engine::{
    engine::{MatchError, OrderBook},
    model::{BookSnapshot, EngineEvent, NewOrder, OrderAck, OrderHistoryEntry, OrderId, Trade},
};

use crate::db::Database;

pub struct AppState {
    pub book: Mutex<OrderBook>,
    pub db: Mutex<Database>,
    pub events: broadcast::Sender<EngineEvent>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/book", get(book))
        .route("/orders", post(submit_order))
        .route("/orders/{id}", delete(cancel_order))
        .route("/orders/{id}/history", get(order_history))
        .route("/trades", get(trades))
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_events(socket, state))
}

async fn health() -> Json<Health> {
    Json(Health { ok: true })
}

async fn book(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BookQuery>,
) -> Json<BookSnapshot> {
    let depth = query.depth.unwrap_or(25).min(100);
    let book = state.book.lock().await;
    Json(book.snapshot(depth))
}

async fn submit_order(
    State(state): State<Arc<AppState>>,
    Json(request): Json<NewOrder>,
) -> Result<Json<OrderAck>, ApiError> {
    let outcome = {
        let mut book = state.book.lock().await;
        book.submit(request)?
    };

    {
        let mut db = state.db.lock().await;
        db.record_ack(&outcome.ack)?;
    }

    for event in outcome.events {
        let _ = state.events.send(event);
    }

    Ok(Json(outcome.ack))
}

async fn cancel_order(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Result<StatusCode, ApiError> {
    let (cancelled, snapshot) = {
        let mut book = state.book.lock().await;
        let cancelled = book.cancel(order_id)?;
        let snapshot = book.snapshot(25);
        (cancelled, snapshot)
    };

    {
        let mut db = state.db.lock().await;
        db.record_cancel(&cancelled)?;
    }

    let _ = state.events.send(EngineEvent::Cancel { order_id });
    let _ = state.events.send(EngineEvent::Book { data: snapshot });
    Ok(StatusCode::NO_CONTENT)
}

async fn order_history(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Json<Vec<OrderHistoryEntry>> {
    let book = state.book.lock().await;
    Json(book.get_order_history(order_id))
}

async fn trades(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TradeQuery>,
) -> Result<Json<Vec<Trade>>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(200);
    let db = state.db.lock().await;
    Ok(Json(db.recent_trades(limit)?))
}

async fn stream_events(mut socket: WebSocket, state: Arc<AppState>) {
    let snapshot = {
        let book = state.book.lock().await;
        EngineEvent::Book {
            data: book.snapshot(25),
        }
    };
    if send_json(&mut socket, &snapshot).await.is_err() {
        return;
    }

    let mut rx = state.events.subscribe();
    while let Ok(event) = rx.recv().await {
        if send_json(&mut socket, &event).await.is_err() {
            break;
        }
    }
}

async fn send_json(socket: &mut WebSocket, event: &EngineEvent) -> Result<(), axum::Error> {
    socket
        .send(Message::Text(
            serde_json::to_string(event)
                .expect("engine events are serializable")
                .into(),
        ))
        .await
}

#[derive(Debug, Deserialize)]
struct BookQuery {
    depth: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TradeQuery {
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct Health {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug)]
pub enum ApiError {
    Match(MatchError),
    Db(rusqlite::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Match(MatchError::OrderNotFound) => {
                (StatusCode::NOT_FOUND, MatchError::OrderNotFound.to_string())
            }
            ApiError::Match(err) => (StatusCode::BAD_REQUEST, err.to_string()),
            ApiError::Db(err) => {
                tracing::error!(error = %err, "database failure");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database failure".to_string(),
                )
            }
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}

impl From<MatchError> for ApiError {
    fn from(value: MatchError) -> Self {
        Self::Match(value)
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Db(value)
    }
}
