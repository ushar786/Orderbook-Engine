use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MatchError {
    #[error("quantity must be greater than zero")]
    InvalidQuantity,
    #[error("quote quantity must be greater than zero")]
    InvalidQuoteQuantity,
    #[error("order side is not supported for this order type")]
    UnsupportedSide,
    #[error("limit orders require a price greater than zero")]
    MissingLimitPrice,
    #[error("price is outside configured tick size")]
    InvalidTick,
    #[error("quantity is outside configured lot size")]
    InvalidLot,
    #[error("order quantity exceeds configured risk limit")]
    MaxOrderQuantityExceeded,
    #[error("order notional exceeds configured risk limit")]
    MaxOrderNotionalExceeded,
    #[error("open order count exceeds configured risk limit")]
    MaxOpenOrdersExceeded,
    #[error("kill switch is active")]
    KillSwitchActive,
    #[error("order not found")]
    OrderNotFound,
}
