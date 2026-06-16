use std::sync::Arc;

use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
};

use orderbook_engine::model::EngineEvent;

use crate::api::AppState;

pub async fn handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_events(socket, state))
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
