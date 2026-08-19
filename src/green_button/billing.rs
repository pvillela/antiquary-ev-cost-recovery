//! Billing periods, and how many hours one is supposed to contain.
//!
//! A Toronto Hydro billing period runs from the start of the day after `bill_end_day` of one
//! month to the end of `bill_end_day` of the next, in **standard time**, and is labelled by that
//! closing date. The June 2026 invoice states its period as `MAY 23 2026 TO JUN 23 2026` over
//! `31` days, which is the same span read from the meter-reading instants rather than the
//! calendar days.
//!
//! Standard time, not prevailing local time: the boundary is at 00:00 EST all year and does not
//! move when the clocks do. That is what the invoices say. Cutting at prevailing local midnight
//! instead reproduces 6 of 19 invoices; cutting on a fixed EST clock reproduces all 19 to the
//! milli-kWh, and matches the `Number of Days` each bill states in every period rather than in 16
//! of them. `docs/hydro_bills/archive/dst-energy-anomaly-pre-fix.md` is the
//! derivation.
//!
//! Only the boundary is on standard time. Time-of-Use periods, the 07:00-19:00 demand window and
//! the holiday calendar stay on prevailing local time, because those are stated in the clock a
//! customer reads and the bills' on-peak and mid-peak energy confirms it.
//!
//! Which day the bill closes on belongs to the bill rather than to the meter data, so nothing here
//! decides it: every entry point takes it as `bill_end_day` and the caller supplies
//! `hydro_bills::BILL_END_DAY`. That keeps the rule for cutting readings in one place while
//! leaving the fact it is cut on in the other.

use crate::green_button::METER_INTERVAL;
use crate::hydro_bills::bill_start_day;
use crate::time::{standard_date, standard_midnight};
use jiff::{Timestamp, civil::Date, civil::date};

/// One billing period, as an instant range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BillingPeriod {
    /// The closing date the period is labelled by, always `bill_end_day` of its month.
    pub ending: Date,
    /// Standard-time midnight starting [`bill_start_day`] of the previous month.
    pub start: Timestamp,
    /// Standard-time midnight starting [`bill_start_day`] of this month; exclusive.
    pub end: Timestamp,
}

impl BillingPeriod {
    /// The period a given instant falls in, on a calendar closing on `bill_end_day` each month.
    pub fn containing(at: Timestamp, bill_end_day: i8) -> Self {
        Self::ending_on(period_ending(standard_date(at), bill_end_day), bill_end_day)
    }

    /// The period labelled by a given closing date, on a calendar closing on `bill_end_day` each
    /// month.
    ///
    /// # Panics
    ///
    /// Panics if `ending` is not `bill_end_day` of a month. That is a caller mixing up two
    /// calendars rather than bad input, since a closing date can only have come from a period
    /// built on the same day.
    pub fn ending_on(ending: Date, bill_end_day: i8) -> Self {
        assert_eq!(
            ending.day(),
            bill_end_day,
            "a billing period is labelled by day {bill_end_day} of the month it ends in"
        );
        let (py, pm) = previous_month(ending.year(), ending.month());
        Self {
            ending,
            start: standard_midnight(date(py, pm, bill_start_day(bill_end_day))),
            end: standard_midnight(date(
                ending.year(),
                ending.month(),
                bill_start_day(bill_end_day),
            )),
        }
    }

    /// How many hourly intervals a complete period contains.
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

    pub fn contains(&self, at: Timestamp) -> bool {
        self.start <= at && at < self.end
    }
}

/// The closing date that labels the period a local date falls in. Past `bill_end_day`, the date
/// belongs to the period ending next month.
fn period_ending(d: Date, bill_end_day: i8) -> Date {
    if d.day() > bill_end_day {
        let (y, m) = next_month(d.year(), d.month());
        date(y, m, bill_end_day)
    } else {
        date(d.year(), d.month(), bill_end_day)
    }
}

fn next_month(year: i16, month: i8) -> (i16, i8) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn previous_month(year: i16, month: i8) -> (i16, i8) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

// cargo test --lib -- green_button::billing::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use crate::hydro_bills::BILL_END_DAY;
    use crate::time::{local_hour, standard_midnight};

    /// The boundary is standard-time midnight between the 23rd and the 24th, so the last hour of
    /// the 23rd belongs to the period ending that day and the first hour of the 24th starts the
    /// next.
    #[test]
    fn the_period_boundary_is_standard_midnight_on_the_24th() {
        let last = standard_midnight(date(2026, 6, 24)) - std::time::Duration::from_secs(3600);
        let first = standard_midnight(date(2026, 6, 24));
        assert_eq!(
            BillingPeriod::containing(last, BILL_END_DAY).ending,
            date(2026, 6, 23)
        );
        assert_eq!(
            BillingPeriod::containing(first, BILL_END_DAY).ending,
            date(2026, 7, 23)
        );
    }

    /// The hour the change was made for. During daylight saving, 00:00-01:00 on the closing day
    /// reads as the 24th on a wall clock but is still the 23rd on the meter's, so it closes the
    /// period rather than opening the next one.
    ///
    /// Under the old prevailing-local boundary this hour fell the other way, which is the whole of
    /// the discrepancy against the invoices.
    #[test]
    fn the_midnight_hour_of_the_closing_day_ends_the_period_it_is_in() {
        let midnight_edt = local_hour(date(2026, 6, 24), 0);
        assert_eq!(
            BillingPeriod::containing(midnight_edt, BILL_END_DAY).ending,
            date(2026, 6, 23)
        );
        // An hour later it is 01:00 EDT, which is 00:00 EST: the next period has begun.
        let an_hour_later = midnight_edt + std::time::Duration::from_secs(3600);
        assert_eq!(
            BillingPeriod::containing(an_hour_later, BILL_END_DAY).ending,
            date(2026, 7, 23)
        );
    }

    #[test]
    fn periods_roll_over_the_year_boundary() {
        assert_eq!(
            BillingPeriod::containing(local_hour(date(2025, 12, 28), 12), BILL_END_DAY).ending,
            date(2026, 1, 23)
        );
        let january = BillingPeriod::ending_on(date(2026, 1, 23), BILL_END_DAY);
        assert_eq!(january.start, standard_midnight(date(2025, 12, 24)));
        assert_eq!(january.end, standard_midnight(date(2026, 1, 24)));
    }

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

    /// The closing day is the caller's to choose. Every other test here passes [`BILL_END_DAY`],
    /// so none of them would notice the argument being taken and then ignored.
    #[test]
    fn a_different_closing_day_moves_the_boundary() {
        let june = BillingPeriod::ending_on(date(2026, 6, 15), 15);
        assert_eq!(june.start, standard_midnight(date(2026, 5, 16)));
        assert_eq!(june.end, standard_midnight(date(2026, 6, 16)));
        // Standard-time midnight starting the 16th is the first instant of the period after it.
        assert_eq!(
            BillingPeriod::containing(standard_midnight(date(2026, 6, 16)), 15).ending,
            date(2026, 7, 15)
        );
        assert_eq!(
            BillingPeriod::containing(local_hour(date(2026, 6, 15), 23), 15).ending,
            date(2026, 6, 15)
        );
    }

    #[test]
    fn contains_is_half_open() {
        let p = BillingPeriod::ending_on(date(2026, 6, 23), BILL_END_DAY);
        assert!(p.contains(p.start));
        assert!(!p.contains(p.end));
    }
}
