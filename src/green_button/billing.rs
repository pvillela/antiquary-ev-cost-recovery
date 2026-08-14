//! Billing periods, and how many hours one is supposed to contain.
//!
//! A Toronto Hydro billing period runs from the start of the 24th of one month to the end of the
//! 23rd of the next, in **local** time, and is labelled by that 23rd. The June 2026 invoice states
//! its period as `MAY 23 2026 TO JUN 23 2026` over `31` days, which is the same span read from the
//! meter-reading instants rather than the calendar days.

use jiff::{Timestamp, civil::Date, civil::date};

use crate::green_button::{local_date, local_midnight};

const SECS_PER_HOUR: i64 = 3600;

/// One billing period, as an instant range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BillingPeriod {
    /// The 23rd the period is labelled by.
    pub ending: Date,
    /// Local midnight starting the 24th of the previous month.
    pub start: Timestamp,
    /// Local midnight starting the 24th of this month; exclusive.
    pub end: Timestamp,
}

impl BillingPeriod {
    /// The period a given instant falls in.
    pub fn containing(at: Timestamp) -> Self {
        Self::ending_on(period_ending(local_date(at)))
    }

    /// The period labelled by a given 23rd.
    ///
    /// # Panics
    ///
    /// Panics if `ending` is not the 23rd of a month.
    pub fn ending_on(ending: Date) -> Self {
        assert_eq!(
            ending.day(),
            23,
            "a billing period is labelled by the 23rd it ends on"
        );
        let (py, pm) = previous_month(ending.year(), ending.month());
        Self {
            ending,
            start: local_midnight(date(py, pm, 24)),
            end: local_midnight(date(ending.year(), ending.month(), 24)),
        }
    }

    /// How many hourly intervals a complete period contains.
    ///
    /// Computed as the elapsed time between the two local midnights, **not** as days times 24.
    /// That distinction is the whole point: the period ending 2026-03-23 spans 28 calendar days
    /// but only 671 hours, because the clocks went forward inside it, and the one ending
    /// 2025-11-23 spans 745 because they went back. Both are complete periods, and a day-count
    /// rule would flag them as short or long.
    pub fn expected_intervals(&self) -> i64 {
        (self.end.as_second() - self.start.as_second()) / SECS_PER_HOUR
    }

    pub fn contains(&self, at: Timestamp) -> bool {
        self.start <= at && at < self.end
    }
}

/// The 23rd that labels the period a local date falls in. On or after the 24th, the date belongs
/// to the period ending next month.
fn period_ending(d: Date) -> Date {
    if d.day() >= 24 {
        let (y, m) = next_month(d.year(), d.month());
        date(y, m, 23)
    } else {
        date(d.year(), d.month(), 23)
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

// cargo test --package green-button --lib -- billing::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use crate::green_button::local_hour;

    /// The boundary is local midnight between the 23rd and the 24th, so the last hour of the 23rd
    /// belongs to the period ending that day and the first hour of the 24th starts the next.
    #[test]
    fn the_period_boundary_is_local_midnight_on_the_24th() {
        let last = local_hour(date(2026, 6, 23), 23);
        let first = local_hour(date(2026, 6, 24), 0);
        assert_eq!(BillingPeriod::containing(last).ending, date(2026, 6, 23));
        assert_eq!(BillingPeriod::containing(first).ending, date(2026, 7, 23));
    }

    #[test]
    fn periods_roll_over_the_year_boundary() {
        assert_eq!(
            BillingPeriod::containing(local_hour(date(2025, 12, 28), 12)).ending,
            date(2026, 1, 23)
        );
        let january = BillingPeriod::ending_on(date(2026, 1, 23));
        assert_eq!(january.start, local_midnight(date(2025, 12, 24)));
        assert_eq!(january.end, local_midnight(date(2026, 1, 24)));
    }

    /// The four interval counts the reference workbook actually contains. 671 and 745 are the
    /// daylight-saving periods and are complete despite not being a multiple of 24.
    #[test]
    fn expected_counts_match_the_reference_workbook() {
        let cases = [
            (date(2026, 6, 23), 744),  // 31 days
            (date(2026, 5, 23), 720),  // 30 days
            (date(2026, 3, 23), 671),  // 28 days, clocks forward
            (date(2025, 11, 23), 745), // 31 days, clocks back
        ];
        for (ending, expected) in cases {
            assert_eq!(
                BillingPeriod::ending_on(ending).expected_intervals(),
                expected,
                "period ending {ending}"
            );
        }
    }

    /// Every period is a whole number of hours and within a day of 24 times its calendar length --
    /// a guard against an arithmetic slip that happens to work for the four cases above.
    #[test]
    fn every_period_is_a_whole_number_of_hours_near_24_per_day() {
        for year in 2024..2030 {
            for month in 1..=12 {
                let p = BillingPeriod::ending_on(date(year, month, 23));
                let secs = p.end.as_second() - p.start.as_second();
                assert_eq!(secs % SECS_PER_HOUR, 0, "{p:?}");
                let hours = p.expected_intervals();
                assert!(
                    (671..=745).contains(&hours),
                    "{hours} hours ending {}",
                    p.ending
                );
            }
        }
    }

    #[test]
    fn contains_is_half_open() {
        let p = BillingPeriod::ending_on(date(2026, 6, 23));
        assert!(p.contains(p.start));
        assert!(!p.contains(p.end));
    }
}
