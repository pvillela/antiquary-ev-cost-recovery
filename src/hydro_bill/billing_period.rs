//! What a Toronto Hydro billing period is: which day it closes on, which instants it spans, and
//! which calendar dates those instants fall on.
//!
//! A billing period runs from the start of the day after `bill_end_day` of one month to the end of
//! `bill_end_day` of the next, in **standard time**, and is labelled by that closing date. The June
//! 2026 invoice states its period as `MAY 23 2026 TO JUN 23 2026` over `31` days, which is the same
//! span read from the meter-reading instants rather than the calendar days.
//!
//! Standard time, not prevailing local time: the boundary is at 00:00 EST all year and does not
//! move when the clocks do. That is what the invoices say. Cutting at prevailing local midnight
//! instead reproduces 6 of 19 invoices; cutting on a fixed EST clock reproduces all 19 to the
//! milli-kWh, and matches the `Number of Days` each bill states in every period rather than in 16
//! of them. `docs/hydro_bill/archive/dst-energy-anomaly-pre-fix.md` is the derivation.
//!
//! Only the boundary is on standard time. Time-of-Use periods, the 07:00-19:00 demand window and
//! the holiday calendar stay on prevailing local time, because those are stated in the clock a
//! customer reads and the bills' on-peak and mid-peak energy confirms it.
//!
//! # Why this is in `hydro_bill`
//!
//! A billing period is a fact about the bill. Everything else divided by one — the meter readings
//! `green_button` groups, the session reports `sessions` reads, the estimates `api` produces — is
//! cut this way because the bill cuts it this way, so the rule belongs where the bill is and the
//! rest of the crate reads it from here.
//!
//! It was in three places before it was here: [`BILL_END_DAY`] in `hydro_bill`, [`BillingPeriod`]
//! in `green_button`, and [`billing_period_dates`] in `api::pure`. Each move outward was a small
//! step, and together they meant that answering "what is a billing period" needed three files in
//! three modules.
//!
//! What is *not* here is anything that reads a period rather than defining one. `green_button`
//! knows how many meter intervals one should hold, because that is a question about the feed;
//! `api::pure::session_report` knows which monthly reports cover one, because that is a question
//! about file names.
//!
//! # Choosing the closing day
//!
//! Which day the bill closes on belongs to the bill rather than to the meter data, so
//! [`BillingPeriod`] does not decide it: both entry points take it as `bill_end_day` and the caller
//! supplies [`BILL_END_DAY`]. That keeps the rule for cutting readings in one place while leaving
//! the fact it is cut on in the other.

use crate::time::{standard_date, standard_midnight};
use jiff::{Timestamp, civil::Date, civil::date};
use std::error::Error;
use std::fmt;

/// The day of the month a Toronto Hydro billing period ends on.
///
/// The bill states its own period as `MAY 23 2026 TO JUN 23 2026`, so this is read off every
/// invoice rather than chosen.
///
/// Changing it moves every billing period boundary in the crate. Nothing suggests Toronto Hydro
/// will, but the number was in three places before it was here, and three places is how a change
/// like that goes half-applied.
pub const BILL_END_DAY: i8 = 23;

/// The largest day of the month a billing period may close on.
///
/// A period starts the day *after* it closes, in the previous month — see [`bill_start_day`] — so a
/// closing day of 28 would put the start on February 29th, a date that does not exist in three
/// years out of four. Capping at 27 keeps every period's start a real date in every month.
pub const MAX_BILL_END_DAY: i8 = 27;

/// The day of the month a billing period starts on, given the day one ends on: the day after.
///
/// A function rather than a second constant, so this states one fact and one relationship rather
/// than two facts that could drift apart. `const` so it still serves where a constant is wanted.
/// Call it as `bill_start_day(BILL_END_DAY)`.
pub const fn bill_start_day(end_day: i8) -> i8 {
    end_day + 1
}

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

/// The date given does not label a billing period: it is not [`BILL_END_DAY`] of its month.
///
/// A struct rather than a one-variant enum, so a function that can fail only this way says exactly
/// that and a caller has nothing to match on. The error enums of operations that call
/// [`billing_period_dates`] embed it rather than restating it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotABillingPeriodEnding {
    pub ending: Date,
}

impl fmt::Display for NotABillingPeriodEnding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ending = self.ending;
        write!(
            f,
            "{ending} does not name a billing period: one is labelled by day {BILL_END_DAY} of the \
             month it ends in"
        )
    }
}

impl Error for NotABillingPeriodEnding {}

/// The first and last calendar dates the billing period labelled by `billing_period_ending` spans,
/// both inclusive.
///
/// The calendar view of [`BillingPeriod`], which holds the same period as instants. A caller states
/// a period by its dates and a feed is cut by its instants, and this is the one place the two are
/// tied together.
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

// cargo test --lib -- hydro_bill::billing_period::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use crate::time::local_hour;

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

    /// Every closing day up to the cap starts its period on a date that exists in every month.
    /// February is the one that bites, and only in three years out of four.
    #[test]
    fn every_permitted_closing_day_starts_on_a_real_date() {
        for end_day in 1..=MAX_BILL_END_DAY {
            for year in 2024..2030 {
                let p = BillingPeriod::ending_on(date(year, 3, end_day), end_day);
                assert_eq!(
                    standard_date(p.start),
                    date(year, 2, bill_start_day(end_day)),
                    "closing day {end_day} in {year}"
                );
            }
        }
    }

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

    /// The two views agree: the dates `billing_period_dates` reports are the ones the instants in
    /// [`BillingPeriod`] fall on.
    #[test]
    fn the_calendar_view_and_the_instant_view_describe_one_period() {
        for month in 1..=12 {
            let ending = date(2026, month, BILL_END_DAY);
            let (first, last) = billing_period_dates(ending).unwrap();
            let period = BillingPeriod::ending_on(ending, BILL_END_DAY);
            assert_eq!(first, standard_date(period.start));
            assert_eq!(last, period.ending);
            // `end` is exclusive, so the last date is the day before the one it lands on.
            assert_eq!(
                standard_date(period.end),
                date(ending.year(), ending.month(), bill_start_day(BILL_END_DAY))
            );
        }
    }
}
