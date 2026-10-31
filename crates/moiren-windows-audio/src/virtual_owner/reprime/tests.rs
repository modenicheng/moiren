use super::*;

#[test]
fn reprime_duration_rejects_multiplication_and_tick_conversion_overflow() {
    assert_eq!(
        ceil_ticks(u128::MAX, 2, 1),
        Err(LatencyBudgetError::ArithmeticOverflow)
    );
    assert_eq!(
        ceil_ticks(u128::from(u64::MAX) + 1, 1, 1),
        Err(LatencyBudgetError::ArithmeticOverflow)
    );
}

#[test]
fn reprime_duration_rounds_up_and_rejects_zero_rate() {
    assert_eq!(ceil_ticks(1, 10_000_000, 48_000), Ok(209));
    assert_eq!(ceil_ticks(48, 10_000_000, 48_000), Ok(10_000));
    assert_eq!(
        ceil_ticks(1, 10_000_000, 0),
        Err(LatencyBudgetError::InvalidDeliveryBound)
    );
}
