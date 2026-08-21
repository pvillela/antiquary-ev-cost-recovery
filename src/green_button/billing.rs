//! How many meter readings a billing period is supposed to contain.
//!
//! What a billing period *is* lives in [`hydro_bill::billing_period`](crate::hydro_bill), because
//! it is a fact about the bill. This is the one thing about a period that is a fact about the feed
//! instead, and it is here for that reason: the count is in [`METER_INTERVAL`]s, which is
//! `green_button`'s unit and nothing the bill knows about.
//!
//! An inherent method rather than a free function, because `period.expected_intervals()` is how it
//! reads at the two call sites and the split is an artefact of where the constant lives.

use crate::green_button::METER_INTERVAL;
use crate::hydro_bill::BillingPeriod;

impl BillingPeriod {
    /// How many meter intervals a complete period contains.
    ///
    /// Still computed as the elapsed time between the two boundaries rather than as days times 24,
    /// though on a standard-time clock the two now always agree: a fixed offset has no short or
    /// long days, so every period is a whole number of 24-hour days and matches the `Number of
    /// Days` its invoice states.
    ///
    /// It was not always so. While the boundary was at prevailing local midnight this returned 671
    /// for the period ending 2026-03-23 and 745 for the one ending 2025-11-23, and those were
    /// treated as complete. They were the symptom that the boundary was wrong — the invoices state
    /// 28 and 31 days, meaning 672 and 744 hours. Deriving the count from the instants is kept
    /// because it stays correct however the boundary is defined.
    pub fn expected_intervals(&self) -> i64 {
        (self.end.as_second() - self.start.as_second()) / METER_INTERVAL.as_secs() as i64
    }
}

// cargo test --lib -- green_button::billing::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use crate::hydro_bill::BILL_END_DAY;
    use jiff::civil::date;

    /// The interval counts the invoices state, as `Number of Days` times 24. The two
    /// daylight-saving periods are in here deliberately: they are 672 and 744, not the 671 and 745
    /// a prevailing-local boundary produced.
    #[test]
    fn expected_counts_match_the_invoices() {
        let cases = [
            (date(2026, 6, 23), 744),  // 31 days
            (date(2026, 5, 23), 720),  // 30 days
            (date(2026, 3, 23), 672),  // 28 days, clocks forward inside it
            (date(2025, 11, 23), 744), // 31 days, clocks back inside it
        ];
        for (ending, expected) in cases {
            assert_eq!(
                BillingPeriod::ending_on(ending, BILL_END_DAY).expected_intervals(),
                expected,
                "period ending {ending}"
            );
        }
    }

    /// Every period is exactly 24 hours per calendar day, clock changes included. On a fixed
    /// offset there is nothing to make a day short or long, so this is now an equality rather than
    /// the range it had to be before.
    #[test]
    fn every_period_is_exactly_24_hours_per_day() {
        for year in 2024..2030 {
            for month in 1..=12 {
                let p = BillingPeriod::ending_on(date(year, month, 23), BILL_END_DAY);
                let secs = p.end.as_second() - p.start.as_second();
                assert_eq!(secs % METER_INTERVAL.as_secs() as i64, 0, "{p:?}");
                let hours = p.expected_intervals();
                assert_eq!(hours % 24, 0, "{hours} hours ending {}", p.ending);
                assert!(
                    (28 * 24..=31 * 24).contains(&hours),
                    "{hours} hours ending {}",
                    p.ending
                );
            }
        }
    }
}
