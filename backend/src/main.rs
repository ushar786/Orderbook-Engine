mod api;
mod db;

use std::{env, error::Error, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::Router;
use db::Database;
use orderbook_engine::engine::OrderBook;
use tokio::{net::TcpListener, signal, sync::broadcast};
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::api::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "orderbook_engine=info,tower_http=info,axum=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_path = env::var("ORDERBOOK_DB").unwrap_or_else(|_| "data/orderbook.db".to_string());
    let addr: SocketAddr = env::var("ORDERBOOK_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()?;

    let db = Database::open(db_path)?;
    let (events, _) = broadcast::channel(4096);
    let state = Arc::new(AppState {
        book: tokio::sync::Mutex::new(OrderBook::new("BTC-USD")),
        db: tokio::sync::Mutex::new(db),
        events,
    });

    let api = api::router();
    let frontend_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend");
    let frontend_index = frontend_dir.join("index.html");
    let app = Router::new()
        .nest("/api", api)
        .route("/ws", axum::routing::get(api::ws_handler))
        .with_state(state)
        .fallback_service(
            ServeDir::new(frontend_dir).not_found_service(ServeFile::new(frontend_index)),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "orderbook engine listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to install signal handler");
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
