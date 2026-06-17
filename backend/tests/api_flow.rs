#![allow(clippy::unwrap_used)]

use std::{
    sync::Arc,
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
    engine::OrderBook,
    model::{OrderHistoryEntry, OrderStatus},
};
use tokio::sync::{Mutex, broadcast};
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
        book: Mutex::new(OrderBook::new("BTC-USD")),
        db: Mutex::new(Database::open(db_path).unwrap()),
        events,
    });
    let app = Router::new()
        .nest("/api", api::router())
        .with_state(Arc::clone(&state));
    (app, state)
}

#[tokio::test]
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

    let db = state.db.lock().await;
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

#[tokio::test]
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

    let db = state.db.lock().await;
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

#[tokio::test]
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

    let db = state.db.lock().await;
    assert_eq!(
        db.order_history(1).unwrap().last().unwrap().status,
        OrderStatus::Cancelled
    );
    assert_eq!(
        db.order_history(2).unwrap().last().unwrap().status,
        OrderStatus::Cancelled
    );
}

async fn post_order(app: Router, payload: &'static str) -> serde_json::Value {
    request_json(app, Method::POST, "/api/orders", Some(payload)).await
}

async fn request_json(
    app: Router,
    method: Method,
    uri: &str,
    payload: Option<&'static str>,
) -> serde_json::Value {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = if let Some(payload) = payload {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(payload)
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
