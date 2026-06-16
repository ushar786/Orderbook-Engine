use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use orderbook_engine::engine::MatchError;

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

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}
