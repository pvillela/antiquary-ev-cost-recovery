use crate::{ApiError, PowerEstimates, green_button::PeriodValues, sessions::RSession};
use jiff::civil::Date;

pub fn peak_power(
    billing_period_ending: Date,
    gb_period_values: PeriodValues,
    sessions: &[RSession],
) -> Result<PowerEstimates, ApiError> {
    todo!()
}
