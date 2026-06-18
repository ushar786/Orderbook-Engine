use std::sync::{
    Arc, Mutex as StdMutex,
    atomic::{AtomicBool, Ordering},
};

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
use tokio::{sync::broadcast, task::spawn_blocking};

use crate::{
    db::{Database, DbError},
    engine::{EngineCommand, EngineCommandError, EngineCommandResult, EngineWorker, MatchError},
    model::{
        BookSnapshot, EngineEvent, EngineMetrics, EngineSnapshot, KillSwitchStatus, NewOrder,
        Order, OrderAck, OrderHistoryEntry, OrderId, OrderStatus, ReplaceAck, ReplaceOrder,
        ReplayReport, SnapshotCheckpoint, Trade,
    },
};

#[derive(Debug)]
pub struct AppState {
    pub engine: Arc<EngineWorker>,
    pub db: StdMutex<Database>,
    pub events: broadcast::Sender<EngineEvent>,
    pub kill_switch: AtomicBool,
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
        .route("/metrics", get(metrics))
        .route("/replay", post(replay_from_journal))
        .route(
            "/engine-snapshot",
            get(engine_snapshot).post(restore_engine_snapshot),
        )
        .route(
            "/engine-snapshot/checkpoint",
            get(latest_snapshot_checkpoint).post(record_snapshot_checkpoint),
        )
        .route(
            "/risk/kill-switch",
            get(kill_switch_status).post(set_kill_switch),
        )
}

async fn mass_cancel_orders(
    State(state): State<Arc<AppState>>,
) -> Result<Json<crate::model::MassCancelAck>, ApiError> {
    let EngineCommandResult::MassCancel(report) =
        with_engine(state.clone(), EngineCommand::MassCancel).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    let ack = match report.events.first() {
        Some(EngineEvent::MassCancel { data }) => data.clone(),
        _ => crate::model::MassCancelAck {
            cancelled_order_ids: Vec::new(),
        },
    };

    let events = report.events.clone();
    let db_events = events.clone();
    let histories = report.histories;
    let cancelled = report.cancelled;
    with_db(state.clone(), move |db| {
        for order in &cancelled {
            db.record_cancel(order)?;
        }
        for (order_id, history) in &histories {
            db.record_order_history(*order_id, history)?;
        }
        db.record_events(&db_events)
    })
    .await?;

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
    if state.kill_switch.load(Ordering::Relaxed) {
        return Err(MatchError::KillSwitchActive.into());
    }
    let EngineCommandResult::Replace(report) =
        with_engine(state.clone(), EngineCommand::Replace(order_id, request)).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };

    let replace_ack = ReplaceAck {
        cancelled_order_id: order_id,
        replacement: report.outcome.ack.clone(),
    };

    let db_ack = report.outcome.ack.clone();
    let db_events = report.outcome.events.clone();
    let histories = report.histories;
    with_db(state.clone(), move |db| {
        db.record_ack(&db_ack)?;
        for (history_order_id, history) in &histories {
            db.record_order_history(*history_order_id, history)?;
        }
        db.record_events(&db_events)
    })
    .await?;

    for event in report.outcome.events {
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
) -> Result<Json<BookSnapshot>, ApiError> {
    let depth = query.depth.unwrap_or(25).min(100);
    let EngineCommandResult::Snapshot(snapshot) =
        with_engine(state, EngineCommand::Snapshot { depth }).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    Ok(Json(snapshot))
}

async fn submit_order(
    State(state): State<Arc<AppState>>,
    Json(request): Json<NewOrder>,
) -> Result<Json<OrderAck>, ApiError> {
    if state.kill_switch.load(Ordering::Relaxed) {
        return Err(MatchError::KillSwitchActive.into());
    }
    let EngineCommandResult::Submit(report) =
        with_engine(state.clone(), EngineCommand::Submit(request)).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };

    let db_ack = report.outcome.ack.clone();
    let db_events = report.outcome.events.clone();
    let histories = report.histories;
    with_db(state.clone(), move |db| {
        db.record_ack(&db_ack)?;
        for (order_id, history) in &histories {
            db.record_order_history(*order_id, history)?;
        }
        db.record_events(&db_events)
    })
    .await?;

    for event in report.outcome.events {
        let _ = state.events.send(event);
    }

    Ok(Json(report.outcome.ack))
}

async fn active_orders(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<OrderWithStatus>>, ApiError> {
    let EngineCommandResult::ActiveOrders(orders) =
        with_engine(state, EngineCommand::ActiveOrders).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    Ok(Json(
        orders
            .into_iter()
            .map(|(order, status)| OrderWithStatus { order, status })
            .collect(),
    ))
}

async fn cancel_order(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Result<StatusCode, ApiError> {
    let EngineCommandResult::Cancel(report) =
        with_engine(state.clone(), EngineCommand::Cancel(order_id)).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };

    let db_events = report.events.clone();
    let events = report.events;
    let cancelled = report.cancelled;
    let history = report.history;
    with_db(state.clone(), move |db| {
        db.record_cancel(&cancelled)?;
        db.record_order_history(order_id, &history)?;
        db.record_events(&db_events)
    })
    .await?;

    for event in events {
        let _ = state.events.send(event);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn order_history(
    State(state): State<Arc<AppState>>,
    Path(order_id): Path<OrderId>,
) -> Result<Json<Vec<OrderHistoryEntry>>, ApiError> {
    match with_engine(state.clone(), EngineCommand::OrderHistory(order_id)).await? {
        EngineCommandResult::OrderHistory(memory_history) if !memory_history.is_empty() => {
            return Ok(Json(memory_history));
        }
        EngineCommandResult::OrderHistory(_) => {}
        _ => return Err(ApiError::Internal("unexpected engine response".to_string())),
    }

    Ok(Json(
        with_db(state.clone(), move |db| db.order_history(order_id)).await?,
    ))
}

async fn trades(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TradeQuery>,
) -> Result<Json<Vec<Trade>>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(200);
    Ok(Json(
        with_db(state.clone(), move |db| db.recent_trades(limit)).await?,
    ))
}

async fn metrics(State(state): State<Arc<AppState>>) -> Result<Json<EngineMetrics>, ApiError> {
    let EngineCommandResult::Metrics(metrics) = with_engine(state, EngineCommand::Metrics).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    Ok(Json(metrics))
}

async fn replay_from_journal(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ReplayReport>, ApiError> {
    let checkpoint = with_db(state.clone(), Database::latest_snapshot).await?;
    let events = if let Some(checkpoint) = &checkpoint {
        let event_journal_id = checkpoint.event_journal_id;
        with_db(state.clone(), move |db| {
            db.events_after_id(event_journal_id)
        })
        .await?
    } else {
        with_db(state.clone(), Database::events).await?
    };
    let EngineCommandResult::Replay(report) =
        with_engine(state.clone(), EngineCommand::Replay { checkpoint, events }).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    let report = ReplayReport {
        event_count: report.event_count,
        checkpoint_sequence: report.checkpoint_sequence,
        active_order_count: report.active_order_count,
        sequence: report.sequence,
        snapshot: report.snapshot,
    };
    let _ = state.events.send(EngineEvent::Book {
        data: report.snapshot.clone(),
    });

    Ok(Json(report))
}

async fn engine_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<EngineSnapshot>, ApiError> {
    let EngineCommandResult::CaptureSnapshot(snapshot) =
        with_engine(state, EngineCommand::CaptureSnapshot).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    Ok(Json(snapshot))
}

async fn restore_engine_snapshot(
    State(state): State<Arc<AppState>>,
    Json(snapshot): Json<EngineSnapshot>,
) -> Result<Json<ReplayReport>, ApiError> {
    let EngineCommandResult::RestoreSnapshot(report) =
        with_engine(state.clone(), EngineCommand::RestoreSnapshot(snapshot)).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    let report = ReplayReport {
        event_count: report.event_count,
        checkpoint_sequence: report.checkpoint_sequence,
        active_order_count: report.active_order_count,
        sequence: report.sequence,
        snapshot: report.snapshot,
    };
    let _ = state.events.send(EngineEvent::Book {
        data: report.snapshot.clone(),
    });

    Ok(Json(report))
}

async fn record_snapshot_checkpoint(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SnapshotCheckpoint>, ApiError> {
    let EngineCommandResult::CaptureSnapshot(snapshot) =
        with_engine(state.clone(), EngineCommand::CaptureSnapshot).await?
    else {
        return Err(ApiError::Internal("unexpected engine response".to_string()));
    };
    let event_journal_id = with_db(state.clone(), Database::latest_event_journal_id).await?;
    let checkpoint = SnapshotCheckpoint {
        event_journal_id,
        snapshot,
    };
    let db_checkpoint = checkpoint.clone();
    with_db(state.clone(), move |db| db.record_snapshot(&db_checkpoint)).await?;
    Ok(Json(checkpoint))
}

async fn latest_snapshot_checkpoint(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Option<SnapshotCheckpoint>>, ApiError> {
    Ok(Json(
        with_db(state.clone(), Database::latest_snapshot).await?,
    ))
}

async fn kill_switch_status(State(state): State<Arc<AppState>>) -> Json<KillSwitchStatus> {
    Json(KillSwitchStatus {
        enabled: state.kill_switch.load(Ordering::Relaxed),
    })
}

async fn set_kill_switch(
    State(state): State<Arc<AppState>>,
    Json(status): Json<KillSwitchStatus>,
) -> Json<KillSwitchStatus> {
    state.kill_switch.store(status.enabled, Ordering::Relaxed);
    Json(status)
}

async fn with_db<T, F>(state: Arc<AppState>, operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&mut Database) -> Result<T, DbError> + Send + 'static,
{
    std::thread::spawn(move || {
        let mut db = state
            .db
            .lock()
            .map_err(|_| ApiError::Internal("database mutex poisoned".to_string()))?;
        operation(&mut db).map_err(ApiError::Db)
    })
    .join()
    .map_err(|err| ApiError::Internal(format!("database task failed: {err:?}")))?
}

async fn with_engine(
    state: Arc<AppState>,
    command: EngineCommand,
) -> Result<EngineCommandResult, ApiError> {
    spawn_blocking(move || state.engine.dispatch(command))
        .await
        .map_err(|err| ApiError::Internal(format!("engine task failed: {err}")))?
        .map_err(ApiError::Engine)
}

async fn stream_events(mut socket: WebSocket, state: Arc<AppState>) {
    let snapshot = match with_engine(state.clone(), EngineCommand::Snapshot { depth: 25 }).await {
        Ok(EngineCommandResult::Snapshot(snapshot)) => EngineEvent::Book { data: snapshot },
        _ => return,
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

#[derive(Debug)]
pub enum ApiError {
    Match(MatchError),
    Engine(EngineCommandError),
    Db(DbError),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Match(MatchError::OrderNotFound) => {
                (StatusCode::NOT_FOUND, MatchError::OrderNotFound.to_string())
            }
            ApiError::Match(err) => (StatusCode::BAD_REQUEST, err.to_string()),
            ApiError::Engine(EngineCommandError::Match(err)) => {
                (StatusCode::BAD_REQUEST, err.to_string())
            }
            ApiError::Engine(err) => {
                tracing::error!(error = %err, "engine command failure");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "engine failure".to_string(),
                )
            }
            ApiError::Db(err) => {
                tracing::error!(error = %err, debug = ?err, "database failure");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database failure".to_string(),
                )
            }
            ApiError::Internal(err) => {
                tracing::error!(error = %err, "internal api failure");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal failure".to_string(),
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
