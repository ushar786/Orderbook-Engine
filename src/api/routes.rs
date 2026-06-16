use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};

use orderbook_engine::model::{BookSnapshot, EngineEvent, NewOrder, OrderAck, OrderId, Trade};

use crate::api::{AppState, errors::ApiError};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health))
        .route("/book", get(book))
        .route("/orders", post(submit_order))
        .route("/orders/{id}", delete(cancel_order))
        .route("/trades", get(trades))
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
        db.orders().record_ack(&outcome.ack)?;
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
        db.orders().record_cancel(&cancelled)?;
    }

    let _ = state.events.send(EngineEvent::Cancel { order_id });
    let _ = state.events.send(EngineEvent::Book { data: snapshot });
    Ok(StatusCode::NO_CONTENT)
}

async fn trades(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TradeQuery>,
) -> Result<Json<Vec<Trade>>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(200);
    let db = state.db.lock().await;
    Ok(Json(db.trades().recent(limit)?))
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
