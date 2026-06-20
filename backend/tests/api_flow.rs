#![allow(clippy::unwrap_used)]

use std::{
    sync::{Arc, Mutex as StdMutex, atomic::AtomicBool},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use orderbook_engine::{
    api::{self, AppState},
    db::Database,
    engine::{EngineWorker, OrderBook},
    model::{OrderHistoryEntry, OrderStatus},
};
use tokio::sync::broadcast;
use tower::ServiceExt;

fn test_app() -> (Router, Arc<AppState>) {
    let db_path = std::env::temp_dir().join(format!(
        "orderbook-api-test-{}.db",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (events, _) = broadcast::channel(128);
    let state = Arc::new(AppState {
        engine: EngineWorker::start(OrderBook::new("BTC-USD")),
        db: StdMutex::new(Database::open(db_path).unwrap()),
        events,
        kill_switch: AtomicBool::new(false),
    });
    let app = Router::new()
        .nest("/api", api::router())
        .with_state(Arc::clone(&state));
    (app, state)
}

fn postgres_test_url() -> Option<String> {
    std::env::var("ORDERBOOK_TEST_POSTGRES")
        .ok()
        .filter(|url| url.starts_with("postgres://") || url.starts_with("postgresql://"))
}

#[tokio::test(flavor = "multi_thread")]
async fn api_persists_matching_history_and_events() {
    let (app, state) = test_app();

    let resting_sell = post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":9999,"quantity":2}"#,
    )
    .await;
    assert_eq!(resting_sell["status"], "resting");

    let crossing_buy = post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":10000,"quantity":5}"#,
    )
    .await;
    assert_eq!(crossing_buy["status"], "partially_filled");
    assert_eq!(crossing_buy["trades"][0]["quantity"], 2);

    let active = get_json(app.clone(), "/api/orders").await;
    assert_eq!(active.as_array().unwrap().len(), 1);
    assert_eq!(active[0]["order"]["remaining_quantity"], 3);

    let buy_history = get_json(app.clone(), "/api/orders/2/history").await;
    assert_eq!(buy_history.as_array().unwrap().len(), 2);
    assert_eq!(buy_history[1]["status"], "partially_filled");

    let cancel_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/orders/2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel_response.status(), StatusCode::NO_CONTENT);

    let active_after_cancel = get_json(app, "/api/orders").await;
    assert!(active_after_cancel.as_array().unwrap().is_empty());

    let mut db = state.db.lock().unwrap();
    assert_eq!(db.event_count().unwrap(), 7);
    assert_eq!(
        statuses(db.order_history(1).unwrap()),
        vec![
            OrderStatus::Accepted,
            OrderStatus::Resting,
            OrderStatus::Filled,
        ]
    );
    assert_eq!(
        statuses(db.order_history(2).unwrap()),
        vec![
            OrderStatus::Accepted,
            OrderStatus::PartiallyFilled,
            OrderStatus::Cancelled,
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn api_replaces_resting_order() {
    let (app, state) = test_app();

    let original = post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":9900,"quantity":10}"#,
    )
    .await;
    assert_eq!(original["status"], "resting");

    let replace = request_json(
        app.clone(),
        Method::PATCH,
        "/api/orders/1",
        Some(r#"{"side":"buy","type":"limit","price":9950,"quantity":4}"#),
    )
    .await;

    assert_eq!(replace["cancelled_order_id"], 1);
    assert_eq!(replace["replacement"]["status"], "resting");
    assert_eq!(replace["replacement"]["order"]["id"], 2);

    let active = get_json(app, "/api/orders").await;
    assert_eq!(active.as_array().unwrap().len(), 1);
    assert_eq!(active[0]["order"]["price"], 9950);
    assert_eq!(active[0]["order"]["remaining_quantity"], 4);

    let mut db = state.db.lock().unwrap();
    assert_eq!(
        statuses(db.order_history(1).unwrap()),
        vec![
            OrderStatus::Accepted,
            OrderStatus::Resting,
            OrderStatus::Cancelled,
        ]
    );
    assert_eq!(
        statuses(db.order_history(2).unwrap()),
        vec![OrderStatus::Accepted, OrderStatus::Resting]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn api_mass_cancels_active_orders() {
    let (app, state) = test_app();

    post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":9900,"quantity":10}"#,
    )
    .await;
    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":10100,"quantity":4}"#,
    )
    .await;

    let ack = request_json(app.clone(), Method::DELETE, "/api/orders", None).await;
    assert_eq!(ack["cancelled_order_ids"].as_array().unwrap().len(), 2);

    let active = get_json(app, "/api/orders").await;
    assert!(active.as_array().unwrap().is_empty());

    let mut db = state.db.lock().unwrap();
    assert_eq!(
        db.order_history(1).unwrap().last().unwrap().status,
        OrderStatus::Cancelled
    );
    assert_eq!(
        db.order_history(2).unwrap().last().unwrap().status,
        OrderStatus::Cancelled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn api_replays_event_journal_into_memory_book() {
    let (app, _state) = test_app();

    post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":10000,"quantity":5}"#,
    )
    .await;
    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":9990,"quantity":2}"#,
    )
    .await;

    let report = request_json(app.clone(), Method::POST, "/api/replay", None).await;

    assert_eq!(report["event_count"], 5);
    assert_eq!(report["active_order_count"], 1);
    assert_eq!(report["snapshot"]["bid_depth"], 3);

    let active = get_json(app, "/api/orders").await;
    assert_eq!(active.as_array().unwrap().len(), 1);
    assert_eq!(active[0]["order"]["remaining_quantity"], 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn api_exports_and_restores_engine_snapshot() {
    let (app, _state) = test_app();

    post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":10000,"quantity":5}"#,
    )
    .await;
    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":10100,"quantity":4}"#,
    )
    .await;
    let snapshot = get_json(app.clone(), "/api/engine-snapshot").await;

    request_json(app.clone(), Method::DELETE, "/api/orders", None).await;
    assert!(
        get_json(app.clone(), "/api/orders")
            .await
            .as_array()
            .unwrap()
            .is_empty()
    );

    let payload = serde_json::to_string(&snapshot).unwrap();
    let report = request_json(
        app.clone(),
        Method::POST,
        "/api/engine-snapshot",
        Some(&payload),
    )
    .await;

    assert_eq!(report["active_order_count"], 2);
    assert_eq!(report["snapshot"]["bid_depth"], 5);
    assert_eq!(report["snapshot"]["ask_depth"], 4);
    assert_eq!(
        get_json(app, "/api/orders").await.as_array().unwrap().len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn api_replay_uses_latest_snapshot_checkpoint() {
    let (app, _state) = test_app();

    post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":10000,"quantity":5}"#,
    )
    .await;
    let checkpoint = request_json(
        app.clone(),
        Method::POST,
        "/api/engine-snapshot/checkpoint",
        None,
    )
    .await;
    assert_eq!(checkpoint["snapshot"]["sequence"], 1);

    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":9990,"quantity":2}"#,
    )
    .await;

    let report = request_json(app.clone(), Method::POST, "/api/replay", None).await;

    assert_eq!(report["checkpoint_sequence"], 1);
    assert_eq!(report["event_count"], 3);
    assert_eq!(report["snapshot"]["bid_depth"], 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn api_kill_switch_blocks_new_orders_but_allows_cancel() {
    let (app, _state) = test_app();

    post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":10000,"quantity":5}"#,
    )
    .await;
    let status = request_json(
        app.clone(),
        Method::POST,
        "/api/risk/kill-switch",
        Some(r#"{"enabled":true}"#),
    )
    .await;
    assert_eq!(status["enabled"], true);

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/orders")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"side":"sell","type":"limit","price":10100,"quantity":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let cancel = app
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/api/orders/1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::NO_CONTENT);
}

#[tokio::test(flavor = "multi_thread")]
async fn api_reports_engine_metrics() {
    let (app, _state) = test_app();

    post_order(
        app.clone(),
        r#"{"side":"buy","type":"limit","price":10000,"quantity":5}"#,
    )
    .await;
    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":10100,"quantity":4}"#,
    )
    .await;

    let metrics = get_json(app, "/api/metrics").await;

    assert_eq!(metrics["symbol"], "BTC-USD");
    assert_eq!(metrics["sequence"], 2);
    assert_eq!(metrics["active_order_count"], 2);
    assert_eq!(metrics["bid_level_count"], 1);
    assert_eq!(metrics["ask_level_count"], 1);
    assert_eq!(metrics["bid_depth"], 5);
    assert_eq!(metrics["ask_depth"], 4);
    assert_eq!(metrics["order_history_count"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn api_accepts_post_only_orders_and_rejects_crossing_post_only() {
    let (app, _state) = test_app();

    let resting = post_order(
        app.clone(),
        r#"{"side":"buy","type":"post_only","price":9900,"quantity":5}"#,
    )
    .await;
    assert_eq!(resting["status"], "resting");
    assert_eq!(resting["order"]["kind"], "post_only");

    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":10100,"quantity":2}"#,
    )
    .await;
    let rejected = post_order(
        app,
        r#"{"side":"buy","type":"post_only","price":10100,"quantity":1}"#,
    )
    .await;
    assert_eq!(rejected["status"], "rejected");
    assert!(rejected["trades"].as_array().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn api_accepts_market_by_notional_buy_orders() {
    let (app, _state) = test_app();

    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":10000,"quantity":3}"#,
    )
    .await;
    post_order(
        app.clone(),
        r#"{"side":"sell","type":"limit","price":10100,"quantity":2}"#,
    )
    .await;

    let ack = post_order(
        app.clone(),
        r#"{"side":"buy","type":"market_by_notional","quote_quantity":40100}"#,
    )
    .await;

    assert_eq!(ack["status"], "filled");
    assert_eq!(ack["order"]["original_quote_quantity"], 40100);
    assert_eq!(ack["order"]["remaining_quote_quantity"], 0);
    assert_eq!(ack["trades"].as_array().unwrap().len(), 2);

    let book = get_json(app, "/api/book").await;
    assert_eq!(book["ask_depth"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn api_prevents_self_trades_by_account_id() {
    let (app, _state) = test_app();

    let resting = post_order(
        app.clone(),
        r#"{"account_id":"acct-a","side":"sell","type":"limit","price":10000,"quantity":2}"#,
    )
    .await;
    assert_eq!(resting["status"], "resting");
    assert_eq!(resting["order"]["account_id"], "acct-a");

    let rejected = post_order(
        app.clone(),
        r#"{"account_id":"acct-a","side":"buy","type":"limit","price":10000,"quantity":2}"#,
    )
    .await;
    assert_eq!(rejected["status"], "rejected");
    assert!(rejected["trades"].as_array().unwrap().is_empty());

    let active = get_json(app, "/api/orders").await;
    assert_eq!(active.as_array().unwrap().len(), 1);
    assert_eq!(active[0]["order"]["id"], resting["order"]["id"]);
}

async fn post_order(app: Router, payload: &'static str) -> serde_json::Value {
    request_json(app, Method::POST, "/api/orders", Some(payload)).await
}

async fn request_json(
    app: Router,
    method: Method,
    uri: &str,
    payload: Option<&str>,
) -> serde_json::Value {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = if let Some(payload) = payload {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(payload.to_string())
    } else {
        Body::empty()
    };
    let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

async fn get_json(app: Router, uri: &str) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn statuses(history: Vec<OrderHistoryEntry>) -> Vec<OrderStatus> {
    history.into_iter().map(|entry| entry.status).collect()
}

#[test]
fn postgres_backend_opens_when_configured() {
    let Some(url) = postgres_test_url() else {
        return;
    };
    let mut db = Database::open(url).unwrap();
    assert!(db.event_count().is_ok());
}
