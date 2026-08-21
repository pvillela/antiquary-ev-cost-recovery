//! What a billing period's closing date implies, as calendar dates.
//!
//! The instant-level arithmetic lives in [`BillingPeriod`], which is where the standard-time
//! boundary is defined. This is the API layer's view of it: the two calendar dates a caller states
//! a period by, and the check that the date it was given labels a period at all.

use crate::NotABillingPeriodEnding;
use crate::green_button::BillingPeriod;
use crate::hydro_bills::BILL_END_DAY;
use crate::time::standard_date;
use jiff::civil::Date;

/// The first and last calendar dates the billing period labelled by `billing_period_ending` spans,
/// both inclusive.
///
/// # Errors
///
/// [`NotABillingPeriodEnding`] if the date is not [`BILL_END_DAY`] of its month.
/// [`BillingPeriod::ending_on`] panics on such a date, and a caller's argument is a caller's
/// argument rather than a bug, so it is caught before reaching it.
pub fn billing_period_dates(
    billing_period_ending: Date,
) -> Result<(Date, Date), NotABillingPeriodEnding> {
    if billing_period_ending.day() != BILL_END_DAY {
        return Err(NotABillingPeriodEnding {
            ending: billing_period_ending,
        });
    }
    let period = BillingPeriod::ending_on(billing_period_ending, BILL_END_DAY);
    // The period boundary is on standard time, so the calendar dates it spans are the standard-time
    // ones. `period.end` is exclusive and lands on the day after the close, which is why the last
    // date is the closing date itself rather than anything read off `end`.
    Ok((standard_date(period.start), billing_period_ending))
}

#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;

    /// The billing period ending 23 June 2026 runs from 24 May to 23 June, on the standard-time
    /// clock the boundary is cut on.
    #[test]
    fn a_billing_period_runs_from_the_day_after_the_previous_close() {
        assert_eq!(
            billing_period_dates(date(2026, 6, 23)).unwrap(),
            (date(2026, 5, 24), date(2026, 6, 23))
        );
        // A period closing in January reaches back into the previous year.
        assert_eq!(
            billing_period_dates(date(2026, 1, 23)).unwrap(),
            (date(2025, 12, 24), date(2026, 1, 23))
        );
    }

    /// A date that is not a closing date is the caller's mistake, and is reported as such rather
    /// than reaching the panic in [`BillingPeriod::ending_on`].
    #[test]
    fn a_date_that_does_not_close_a_billing_period_is_refused() {
        let err = billing_period_dates(date(2026, 6, 30))
            .expect_err("30 June does not label a billing period");
        assert_eq!(err.ending, date(2026, 6, 30));
        assert!(err.to_string().contains("2026-06-30"), "{err}");
    }
}
