use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MatchError {
    #[error("quantity must be greater than zero")]
    InvalidQuantity,
    #[error("limit orders require a price greater than zero")]
    MissingLimitPrice,
    #[error("price is outside configured tick size")]
    InvalidTick,
    #[error("quantity is outside configured lot size")]
    InvalidLot,
    #[error("order not found")]
    OrderNotFound,
}
