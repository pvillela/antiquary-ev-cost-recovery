//! The Ontario holiday calendar, as the Ontario Energy Board defines it for Time-of-Use pricing.
//!
//! This is deliberately **not** the Employment Standards Act list, and deliberately not a holiday
//! crate. The two differ in ways that move money:
//!
//! * The **August Civic Holiday** is on the OEB's published TOU schedule and is *not* an ESA
//!   public holiday. Leaving it out would reclassify a summer weekday's 07:00-19:00 block as
//!   on/mid-peak and can shift a monthly peak.
//! * The ESA's substitute-day entitlement is negotiated per employee within a three- or
//!   twelve-month window. It is not a calendar rule and cannot be computed.
//! * The `holidays` Python package this replaces never shifts Canada Day, even when July 1 falls
//!   on a Sunday, and couples its `PUBLIC`/`OPTIONAL` categories such that dropping the Civic
//!   Holiday also drops correct Boxing Day substitutes.
//!
//! The OEB's substitution rule, quoted verbatim from
//! <https://www.oeb.ca/consumer-information-and-protection/electricity-rates/holiday-schedule-time-use-and-ultra-low>:
//!
//! > If a holiday falls on a weekend, the next weekday (that is not also a holiday) will have the
//! > holiday prices in effect all day.
//!
//! Note that this is **additive**: the weekend day is already off-peak because it is a weekend, and
//! the substitute weekday joins it. The holiday is not moved.
//!
//! Every date here is a rule, so the calendar needs no annual maintenance. It does assume the
//! current schedule applies throughout: see `docs/maintenance-manual.md` for what would force a
//! re-check.

use std::collections::BTreeSet;

use jiff::civil::{Date, Weekday, date};

/// One holiday on the OEB TOU schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Holiday {
    pub date: Date,
    /// `Some(base)` when this entry is a weekend substitute, carrying the date it stands in for.
    pub substitute_for: Option<Date>,
    pub name: &'static str,
}

/// Every OEB TOU holiday in `year`, in calendar order, including weekend substitutes.
///
/// A substitute always lands in the same year as the holiday it stands in for: the latest possible
/// base date is December 26, whose substitute can be no later than December 28.
///
/// # Panics
///
/// Panics if `year` is outside the range jiff can represent a civil date in. Every date is
/// constructed from a rule that is total over the representable years, so this cannot fire for any
/// year a meter reading could carry.
pub fn holidays(year: i16) -> Vec<Holiday> {
    let base = [
        (date(year, 1, 1), "New Year's Day"),
        (
            nth_weekday_of_month(year, 2, 3, Weekday::Monday),
            "Family Day",
        ),
        (good_friday(year), "Good Friday"),
        (monday_before(date(year, 5, 25)), "Victoria Day"),
        (date(year, 7, 1), "Canada Day"),
        (
            nth_weekday_of_month(year, 8, 1, Weekday::Monday),
            "Civic Holiday",
        ),
        (
            nth_weekday_of_month(year, 9, 1, Weekday::Monday),
            "Labour Day",
        ),
        (
            nth_weekday_of_month(year, 10, 2, Weekday::Monday),
            "Thanksgiving Day",
        ),
        (date(year, 12, 25), "Christmas Day"),
        (date(year, 12, 26), "Boxing Day"),
    ];

    let mut all: Vec<Holiday> = base
        .iter()
        .map(|&(date, name)| Holiday {
            date,
            substitute_for: None,
            name,
        })
        .collect();
    // Good Friday floats between March and April, so the rule order above is not calendar order.
    // The substitution below depends on calendar order: Christmas must claim its substitute before
    // Boxing Day looks for one, or the two would collide.
    all.sort();

    let mut occupied: BTreeSet<Date> = all.iter().map(|h| h.date).collect();
    let mut substitutes = Vec::new();
    for holiday in &all {
        if !is_weekend(holiday.date) {
            continue;
        }
        let mut candidate = holiday.date;
        loop {
            candidate = candidate
                .tomorrow()
                .expect("a substitute stays within the year");
            if !is_weekend(candidate) && !occupied.contains(&candidate) {
                break;
            }
        }
        occupied.insert(candidate);
        substitutes.push(Holiday {
            date: candidate,
            substitute_for: Some(holiday.date),
            name: holiday.name,
        });
    }

    all.extend(substitutes);
    all.sort();
    all
}

/// Whether `d` is an OEB TOU holiday.
///
/// Recomputes the year's ten rules on each call. That is a few hundred nanoseconds against the
/// tens of milliseconds spent parsing an 18 MB feed, so it is not worth a cache that could go
/// stale.
pub fn is_holiday(d: Date) -> bool {
    holidays(d.year()).iter().any(|h| h.date == d)
}

/// Whether every hour of `d` is off-peak on its own account — a weekend or a holiday.
pub fn is_full_day_off_peak(d: Date) -> bool {
    is_weekend(d) || is_holiday(d)
}

fn is_weekend(d: Date) -> bool {
    matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

fn nth_weekday_of_month(year: i16, month: i8, nth: i8, weekday: Weekday) -> Date {
    date(year, month, 1)
        .nth_weekday_of_month(nth, weekday)
        .expect("every rule here names a weekday that exists in its month")
}

/// The Monday strictly before `d`. `nth_weekday` does not count the date it is called on, which is
/// what the Victoria Day rule needs: when May 25 is itself a Monday, the holiday is the Monday a
/// week earlier.
fn monday_before(d: Date) -> Date {
    d.nth_weekday(-1, Weekday::Monday)
        .expect("a Monday precedes every representable date")
}

/// Good Friday is the Friday before Easter Sunday, so two days before it.
fn good_friday(year: i16) -> Date {
    easter_sunday(year)
        .yesterday()
        .and_then(|d| d.yesterday())
        .expect("Easter is never January 1")
}

/// Easter Sunday by the anonymous Gregorian computus (Meeus/Jones/Butcher).
///
/// Easter is the only movable date in the list, and it is the reason this module cannot be a
/// lookup table.
fn easter_sunday(year: i16) -> Date {
    let y = i32::from(year);
    let a = y % 19;
    let b = y / 100;
    let c = y % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    date(year, month as i8, day as i8)
}

// cargo test --package green-button --lib -- holidays::test --nocapture
#[cfg(test)]
mod test {
    use super::*;

    fn dates(year: i16) -> Vec<Date> {
        holidays(year).into_iter().map(|h| h.date).collect()
    }

    /// The OEB's published 2026 schedule, transcribed from its holiday-schedule page. Every one of
    /// these ten must appear; this is the single most direct check that the rules are right.
    #[test]
    fn the_published_2026_oeb_schedule_is_reproduced() {
        let got = dates(2026);
        for expected in [
            date(2026, 1, 1),   // New Year's Day, Thu
            date(2026, 2, 16),  // Family Day, Mon
            date(2026, 4, 3),   // Good Friday, Fri
            date(2026, 5, 18),  // Victoria Day, Mon
            date(2026, 7, 1),   // Canada Day, Wed
            date(2026, 8, 3),   // Civic Holiday, Mon
            date(2026, 9, 7),   // Labour Day, Mon
            date(2026, 10, 12), // Thanksgiving Day, Mon
            date(2026, 12, 25), // Christmas Day, Fri
            date(2026, 12, 28), // Boxing Day observed, Mon -- Dec 26 is a Saturday
        ] {
            assert!(got.contains(&expected), "{expected} missing from {got:?}");
        }
    }

    /// The sixteen dates spanned by the sample feed, which is the range the workbook covers.
    #[test]
    fn the_sample_feeds_range_matches_the_reference_implementation() {
        let mut got: Vec<Date> = [2024, 2025, 2026]
            .into_iter()
            .flat_map(dates)
            .filter(|d| *d >= date(2024, 11, 23) && *d < date(2026, 6, 24))
            .collect();
        got.sort();
        got.dedup();
        assert_eq!(
            got,
            vec![
                date(2024, 12, 25),
                date(2024, 12, 26),
                date(2025, 1, 1),
                date(2025, 2, 17),
                date(2025, 4, 18),
                date(2025, 5, 19),
                date(2025, 7, 1),
                date(2025, 8, 4),
                date(2025, 9, 1),
                date(2025, 10, 13),
                date(2025, 12, 25),
                date(2025, 12, 26),
                date(2026, 1, 1),
                date(2026, 2, 16),
                date(2026, 4, 3),
                date(2026, 5, 18),
            ]
        );
    }

    /// The Civic Holiday is the whole reason this module exists rather than an ESA holiday list.
    #[test]
    fn the_civic_holiday_is_the_first_monday_in_august() {
        assert!(dates(2025).contains(&date(2025, 8, 4)));
        assert!(dates(2026).contains(&date(2026, 8, 3)));
        assert!(dates(2027).contains(&date(2027, 8, 2)));
    }

    /// Christmas on a Saturday and Boxing Day on the Sunday after: the substitutes must not
    /// collide, and Boxing Day must skip past the day Christmas already claimed.
    #[test]
    fn a_christmas_boxing_day_weekend_yields_two_distinct_substitutes() {
        let got = dates(2027);
        assert!(got.contains(&date(2027, 12, 25))); // Sat, the holiday itself
        assert!(got.contains(&date(2027, 12, 26))); // Sun, the holiday itself
        assert!(got.contains(&date(2027, 12, 27))); // Mon, Christmas observed
        assert!(got.contains(&date(2027, 12, 28))); // Tue, Boxing Day observed
    }

    /// Canada Day on a Saturday. The OEB rule substitutes the next weekday; the Python library this
    /// replaces never shifted Canada Day at all, in either direction.
    #[test]
    fn canada_day_on_a_weekend_gets_a_substitute() {
        assert!(dates(2028).contains(&date(2028, 7, 3)));
        assert!(dates(2028).contains(&date(2028, 1, 3))); // New Year's Day, Sat Jan 1
    }

    /// Substitution is additive: the weekend date stays on the list rather than being replaced.
    #[test]
    fn substitution_keeps_the_original_weekend_date() {
        let got = holidays(2026);
        let boxing: Vec<_> = got.iter().filter(|h| h.name == "Boxing Day").collect();
        assert_eq!(boxing.len(), 2);
        assert_eq!(boxing[0].date, date(2026, 12, 26));
        assert_eq!(boxing[0].substitute_for, None);
        assert_eq!(boxing[1].date, date(2026, 12, 28));
        assert_eq!(boxing[1].substitute_for, Some(date(2026, 12, 26)));
    }

    /// Good Friday is the only movable date, so it gets its own check against known Easters.
    #[test]
    fn good_friday_tracks_easter() {
        assert_eq!(good_friday(2025), date(2025, 4, 18)); // Easter Apr 20
        assert_eq!(good_friday(2026), date(2026, 4, 3)); // Easter Apr 5
        assert_eq!(good_friday(2027), date(2027, 3, 26)); // Easter Mar 28
    }

    /// A substitute never lands on a weekend, and never on a day already spoken for.
    #[test]
    fn no_substitute_lands_on_a_weekend_or_another_holiday() {
        for year in 2020..2040 {
            let all = holidays(year);
            for h in all.iter().filter(|h| h.substitute_for.is_some()) {
                assert!(!is_weekend(h.date), "{} substitute on a weekend", h.date);
                assert_eq!(
                    all.iter().filter(|o| o.date == h.date).count(),
                    1,
                    "{} claimed twice in {year}",
                    h.date
                );
            }
        }
    }
}
