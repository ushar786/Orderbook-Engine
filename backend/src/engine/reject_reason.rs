use crate::{
    engine::{MatchError, book::RiskConfig},
    model::{NewOrder, OrderKind, Price, Quantity},
};

pub fn validate_order(
    request: &NewOrder,
    tick_size: Price,
    lot_size: Quantity,
    active_order_count: usize,
    active_account_order_count: usize,
    risk: &RiskConfig,
) -> Result<(), MatchError> {
    if !request.account_id.is_empty()
        && risk
            .blocked_accounts
            .iter()
            .any(|account_id| account_id == &request.account_id)
    {
        return Err(MatchError::AccountBlocked);
    }

    if request.kind == OrderKind::MarketByNotional {
        if request.side != crate::model::Side::Buy {
            return Err(MatchError::UnsupportedSide);
        }
        let quote_quantity = request
            .quote_quantity
            .filter(|quantity| *quantity > 0)
            .ok_or(MatchError::InvalidQuoteQuantity)?;
        if let Some(max_notional) = risk.max_order_notional
            && quote_quantity > max_notional
        {
            return Err(MatchError::MaxOrderNotionalExceeded);
        }
        return Ok(());
    }

    validate_base_quantity(request.quantity, lot_size, risk)?;

    if opens_order(request.kind, request.time_in_force) {
        if let Some(max_open_orders) = risk.max_open_orders
            && active_order_count >= max_open_orders
        {
            return Err(MatchError::MaxOpenOrdersExceeded);
        }
        if let Some(max_open_orders_per_account) = risk.max_open_orders_per_account
            && !request.account_id.is_empty()
            && active_account_order_count >= max_open_orders_per_account
        {
            return Err(MatchError::MaxAccountOpenOrdersExceeded);
        }
    }

    if matches!(
        request.kind,
        OrderKind::Limit | OrderKind::PostOnly | OrderKind::StopLimit
    ) {
        let price = request
            .price
            .filter(|price| *price > 0)
            .ok_or(MatchError::MissingLimitPrice)?;
        if !price.is_multiple_of(tick_size) {
            return Err(MatchError::InvalidTick);
        }
        if let Some(max_notional) = risk.max_order_notional {
            let notional = price.saturating_mul(request.quantity);
            if notional > max_notional {
                return Err(MatchError::MaxOrderNotionalExceeded);
            }
        }
    }
    if matches!(request.kind, OrderKind::StopLimit | OrderKind::StopMarket) {
        let stop_price = request
            .stop_price
            .filter(|price| *price > 0)
            .ok_or(MatchError::MissingStopPrice)?;
        if !stop_price.is_multiple_of(tick_size) {
            return Err(MatchError::InvalidTick);
        }
    }
    Ok(())
}

fn opens_order(kind: OrderKind, time_in_force: crate::model::TimeInForce) -> bool {
    matches!(
        (kind, time_in_force),
        (
            OrderKind::Limit | OrderKind::PostOnly | OrderKind::StopLimit | OrderKind::StopMarket,
            _
        ) | (OrderKind::Market, crate::model::TimeInForce::Gtc)
    )
}

fn validate_base_quantity(
    quantity: Quantity,
    lot_size: Quantity,
    risk: &RiskConfig,
) -> Result<(), MatchError> {
    if quantity == 0 {
        return Err(MatchError::InvalidQuantity);
    }
    if let Some(max_quantity) = risk.max_order_quantity
        && quantity > max_quantity
    {
        return Err(MatchError::MaxOrderQuantityExceeded);
    }
    if !quantity.is_multiple_of(lot_size) {
        return Err(MatchError::InvalidLot);
    }
    Ok(())
}
