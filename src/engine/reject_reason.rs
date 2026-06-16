use crate::{
    engine::MatchError,
    model::{NewOrder, OrderKind, Price, Quantity},
};

pub fn validate_order(
    request: &NewOrder,
    tick_size: Price,
    lot_size: Quantity,
) -> Result<(), MatchError> {
    if request.quantity == 0 {
        return Err(MatchError::InvalidQuantity);
    }
    if !request.quantity.is_multiple_of(lot_size) {
        return Err(MatchError::InvalidLot);
    }
    if request.kind == OrderKind::Limit {
        let price = request
            .price
            .filter(|price| *price > 0)
            .ok_or(MatchError::MissingLimitPrice)?;
        if !price.is_multiple_of(tick_size) {
            return Err(MatchError::InvalidTick);
        }
    }
    Ok(())
}
