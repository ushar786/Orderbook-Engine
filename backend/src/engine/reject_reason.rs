use crate::{
    engine::{MatchError, book::RiskConfig},
    model::{NewOrder, OrderKind, Price, Quantity},
};

pub fn validate_order(
    request: &NewOrder,
    tick_size: Price,
    lot_size: Quantity,
    active_order_count: usize,
    risk: &RiskConfig,
) -> Result<(), MatchError> {
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

    if matches!(request.kind, OrderKind::Limit | OrderKind::PostOnly) {
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
        if let Some(max_open_orders) = risk.max_open_orders
            && active_order_count >= max_open_orders
        {
            return Err(MatchError::MaxOpenOrdersExceeded);
        }
    }
    Ok(())
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
