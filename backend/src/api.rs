use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast};

use crate::{
    db::{Database, DbError},
    engine::{MatchError, OrderBook},
    model::{
        BookSnapshot, EngineEvent, NewOrder, Order, OrderAck, OrderHistoryEntry, OrderId,
        OrderStatus, ReplaceAck, ReplaceOrder, Trade,
    },
};

#[derive(Debug)]
pub struct AppState {
    pub book: Mutex<OrderBook>,
    pub db: Mutex<Database>,
    pub events: broadcast::Sender<EngineEvent>,
}

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/book", get(book))
        .route(
            "/orders",
            get(active_orders)
                .post(submit_order)
                .delete(mass_cancel_orders),
        )
        .route("/orders/{id}", delete(cancel_order).patch(replace_order))
        .route("/orders/{id}/history", get(order_history))
        .route("/trades", get(trades))
}

async fn mass_cancel_orders(
    State(state): State<Arc<AppState>>,
) -> Result<Json<crate::model::MassCancelAck>, ApiError> {
    let (cancelled, histories, snapshot) = {
        let mut book = state.book.lock().await;
        let cancelled = book.cancel_all();
        let histories = cancelled
            .iter()
            .map(|order| (order.id, book.get_order_history(order.id)))
            .collect::<Vec<_>>();
        let snapshot = book.snapshot(25);
        (cancelled, histories, snapshot)
    };

    let events = OrderBook::mass_cancel_events(&cancelled, snapshot);
    let ack = match events.first() {
        Some(EngineEvent::MassCancel { data }) => data.clone(),
        _ => crate::model::MassCancelAck {
            cancelled_order_ids: Vec::new(),
        },
    };

    {
        let mut db = state.db.lock().await;
        for order in &cancelled {
            db.record_cancel(order)?;
        }
        for (order_id, history) in &histories {
            db.record_order_history(*order_id, history)?;
        }
        db.record_events(&events)?;
    }

    for event in events {
        let _ = state.events.send(event);
    }

    Ok(Json(ack))
}

async fn replace_order(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
    Json(request): Json<ReplaceOrder>,
) -> Result<Json<ReplaceAck>, ApiError> {
    let (outcome, histories) = {
        let mut book = state.book.lock().await;
        let outcome = book.replace(order_id, request)?;
        let mut histories = touched_histories(&book, &outcome.ack);
        histories.push((order_id, book.get_order_history(order_id)));
        (outcome, histories)
    };

    let replace_ack = ReplaceAck {
        cancelled_order_id: order_id,
        replacement: outcome.ack.clone(),
    };

    {
        let mut db = state.db.lock().await;
        db.record_ack(&outcome.ack)?;
        for (history_order_id, history) in &histories {
            db.record_order_history(*history_order_id, history)?;
        }
        db.record_events(&outcome.events)?;
    }

    for event in outcome.events {
        let _ = state.events.send(event);
    }

    Ok(Json(replace_ack))
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
    let (outcome, histories) = {
        let mut book = state.book.lock().await;
        let outcome = book.submit(request)?;
        let histories = touched_histories(&book, &outcome.ack);
        (outcome, histories)
    };

    {
        let mut db = state.db.lock().await;
        db.record_ack(&outcome.ack)?;
        for (order_id, history) in &histories {
            db.record_order_history(*order_id, history)?;
        }
        db.record_events(&outcome.events)?;
    }

    for event in outcome.events {
        let _ = state.events.send(event);
    }

    Ok(Json(outcome.ack))
}

async fn active_orders(State(state): State<Arc<AppState>>) -> Json<Vec<OrderWithStatus>> {
    let book = state.book.lock().await;
    let orders = book
        .active_orders()
        .into_iter()
        .map(|order| OrderWithStatus {
            status: latest_status(&book.get_order_history(order.id))
                .unwrap_or(OrderStatus::Resting),
            order,
        })
        .collect();
    Json(orders)
}

async fn cancel_order(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Result<StatusCode, ApiError> {
    let (cancelled, history, snapshot) = {
        let mut book = state.book.lock().await;
        let cancelled = book.cancel(order_id)?;
        let history = book.get_order_history(order_id);
        let snapshot = book.snapshot(25);
        (cancelled, history, snapshot)
    };

    let events = vec![
        EngineEvent::Cancel { order_id },
        EngineEvent::Book {
            data: snapshot.clone(),
        },
    ];

    {
        let mut db = state.db.lock().await;
        db.record_cancel(&cancelled)?;
        db.record_order_history(order_id, &history)?;
        db.record_events(&events)?;
    }

    for event in events {
        let _ = state.events.send(event);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn order_history(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Result<Json<Vec<OrderHistoryEntry>>, ApiError> {
    let book = state.book.lock().await;
    let memory_history = book.get_order_history(order_id);
    drop(book);
    if !memory_history.is_empty() {
        return Ok(Json(memory_history));
    }

    let mut db = state.db.lock().await;
    Ok(Json(db.order_history(order_id)?))
}

async fn trades(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TradeQuery>,
) -> Result<Json<Vec<Trade>>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(200);
    let mut db = state.db.lock().await;
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
    let Ok(payload) = serde_json::to_string(event) else {
        tracing::error!("failed to serialize engine event");
        return Ok(());
    };
    socket.send(Message::Text(payload.into())).await
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

#[derive(Debug, Serialize)]
struct OrderWithStatus {
    order: Order,
    status: OrderStatus,
}

fn latest_status(history: &[OrderHistoryEntry]) -> Option<OrderStatus> {
    history.last().map(|entry| entry.status)
}

fn touched_histories(book: &OrderBook, ack: &OrderAck) -> Vec<(OrderId, Vec<OrderHistoryEntry>)> {
    let mut order_ids = Vec::with_capacity(ack.trades.len() + 1);
    order_ids.push(ack.order.id);
    for trade in &ack.trades {
        if !order_ids.contains(&trade.maker_order_id) {
            order_ids.push(trade.maker_order_id);
        }
    }
    order_ids
        .into_iter()
        .map(|order_id| (order_id, book.get_order_history(order_id)))
        .collect()
}

#[derive(Debug)]
pub enum ApiError {
    Match(MatchError),
    Db(DbError),
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

impl From<DbError> for ApiError {
    fn from(value: DbError) -> Self {
        Self::Db(value)
    }
}
