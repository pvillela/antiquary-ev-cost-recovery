//! Ontario Time-of-Use price periods.
//!
//! The Ontario Energy Board publishes these as two weekday schedules that swap on-peak and
//! mid-peak between seasons, plus a blanket rule for weekends and holidays. Quoted verbatim from
//! <https://www.oeb.ca/consumer-information-and-protection/electricity-rates>:
//!
//! > **Winter (November 1 – April 30)** — Off-Peak: Weekdays 7 p.m. – 7 a.m. Weekends and holidays
//! > all day. Mid-Peak: Weekdays 11 a.m. – 5 p.m. On-Peak: Weekdays 7 a.m. – 11 a.m. and 5 p.m. –
//! > 7 p.m.
//! >
//! > **Summer (May 1 – October 31)** — Off-Peak: Weekdays 7 p.m. – 7 a.m. Weekends and holidays all
//! > day. Mid-Peak: Weekdays 7 a.m. – 11 a.m. and 5 p.m. – 7 p.m. On-Peak: Weekdays 11 a.m. –
//! > 5 p.m.
//!
//! Off-peak is season-invariant; only the labels on the midday block and the two shoulder blocks
//! trade places. Summer peaks midday, winter peaks morning and evening.
//!
//! Two things below are **implementation choices, not OEB policy**, because the OEB is silent on
//! both. The season changeover is taken to happen at local midnight on May 1 and November 1 — the
//! OEB states the seasons only as calendar dates. And the daylight-saving transitions get no
//! special handling: they occur at 02:00 local, every boundary here is at 07:00, 11:00, 17:00 or
//! 19:00, and 02:00 is inside the off-peak block in both seasons, so the spring gap and the autumn
//! fold cannot land on a boundary. That is a consequence of the two rule sets, not something the
//! OEB has ruled on.
//!
//! Note also what these periods are *not*. Toronto Hydro's `Peak kW 7-7` demand window is a
//! distribution-charge measurement window, not a pricing period; it happens to be exactly the
//! complement of the weekday off-peak block, which is why [`is_off_peak`] can serve both. See
//! `docs/maintenance-manual.md` for what would drive them apart.

use ev_peak_contrib::Interval;
use jiff::{Timestamp, civil::Date};

use crate::holidays;
use crate::{local_date, local_hour, local_midnight};

/// An Ontario Time-of-Use price period.
///
/// Three variants, matching the standard TOU plan. Ultra-Low Overnight is a *separate* opt-in plan
/// with four periods, no seasonality, and weekends split rather than uniformly off-peak — a
/// customer elects one plan or the other, never both. Should ULO ever be needed it wants its own
/// enum and its own partition function beside these, not a fourth variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tou {
    OnPeak,
    MidPeak,
    OffPeak,
}

impl Tou {
    /// The token written to the workbook. A stable wire format, like [`crate::Anomaly::as_str`]:
    /// these sheets are meant to be read back by column name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OnPeak => "OnPeak",
            Self::MidPeak => "MidPeak",
            Self::OffPeak => "OffPeak",
        }
    }

    /// Inverse of [`Tou::as_str`].
    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "OnPeak" => Self::OnPeak,
            "MidPeak" => Self::MidPeak,
            "OffPeak" => Self::OffPeak,
            _ => return None,
        })
    }
}

impl std::fmt::Display for Tou {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A day's price periods as `(start hour, period)` pairs, ascending, the first always starting at
/// hour zero and the last running to midnight.
///
/// Boundaries are **hour integers**. That is the point: a half-hour boundary is unrepresentable,
/// so the property the rest of the crate relies on — that an hour-long interval starting on the
/// hour lies wholly within one period — is enforced by the type rather than by a check somebody
/// has to remember to run.
type Schedule = &'static [(u8, Tou)];

const SUMMER_WEEKDAY: Schedule = &[
    (0, Tou::OffPeak),
    (7, Tou::MidPeak),
    (11, Tou::OnPeak),
    (17, Tou::MidPeak),
    (19, Tou::OffPeak),
];

const WINTER_WEEKDAY: Schedule = &[
    (0, Tou::OffPeak),
    (7, Tou::OnPeak),
    (11, Tou::MidPeak),
    (17, Tou::OnPeak),
    (19, Tou::OffPeak),
];

const ALL_DAY_OFF_PEAK: Schedule = &[(0, Tou::OffPeak)];

/// Summer runs May 1 to October 31 inclusive; the rest of the year is winter.
fn is_summer(d: Date) -> bool {
    (5..=10).contains(&d.month())
}

/// The price periods in force on a given local date.
fn day_schedule(d: Date) -> Schedule {
    let schedule = if holidays::is_full_day_off_peak(d) {
        ALL_DAY_OFF_PEAK
    } else if is_summer(d) {
        SUMMER_WEEKDAY
    } else {
        WINTER_WEEKDAY
    };
    debug_assert!(
        schedule[0].0 == 0
            && schedule.windows(2).all(|w| w[0].0 < w[1].0)
            && schedule.last().unwrap().0 < 24,
        "a schedule must start at hour zero and ascend within the day"
    );
    schedule
}

/// Splits `interval` into the price periods it spans.
///
/// The returned intervals form a **maximal** partition of the input: they are in chronological
/// order, contiguous, none is empty, their union is exactly `interval`, and adjacent stretches
/// sharing a period are merged into one. A Friday evening through to Monday morning in winter comes
/// back as three pieces — the Friday 17:00-19:00 on-peak block, one off-peak run spanning the whole
/// weekend, and Monday's 07:00 on-peak block — not as one piece per calendar day.
///
/// An empty interval yields an empty vector.
///
/// # Panics
///
/// Panics if the interval extends beyond the range jiff can represent a civil date in. `Interval`
/// cannot hold an inverted span by construction, so there is no error case for that.
pub fn tou_partition(interval: Interval) -> Vec<(Tou, Interval)> {
    if interval.is_empty() {
        return Vec::new();
    }

    let end = interval.end();
    let mut pieces: Vec<(Tou, Interval)> = Vec::new();
    let mut day = local_date(interval.start);

    while local_midnight(day) < end {
        let schedule = day_schedule(day);
        for (i, &(hour, tou)) in schedule.iter().enumerate() {
            let block_start = local_hour(day, hour);
            let block_end = match schedule.get(i + 1) {
                Some(&(next_hour, _)) => local_hour(day, next_hour),
                None => local_midnight(tomorrow(day)),
            };
            let piece = interval.intersection(&Interval::from_start_end(block_start, block_end));
            if piece.is_empty() {
                continue;
            }
            match pieces.last_mut() {
                // Merging is what makes the partition maximal, and it has to work across the day
                // boundary too -- a weekend is three calendar days of one off-peak run.
                Some((last_tou, last)) if *last_tou == tou && last.end() == piece.start => {
                    *last = Interval::from_start_end(last.start, piece.end());
                }
                _ => pieces.push((tou, piece)),
            }
        }
        day = tomorrow(day);
    }

    pieces
}

/// The single price period `interval` falls in, or `None` if it straddles two.
///
/// Every interval in a Green Button feed is an hour starting on the hour, and no TOU boundary
/// falls off the hour, so this returns `Some` for well-formed data. A `None` means the interval was
/// misaligned, which is recorded as [`crate::Anomaly::MisalignedInterval`] and excluded from peak
/// selection rather than resolved by picking the longest piece.
pub fn tou_of(interval: Interval) -> Option<Tou> {
    match tou_partition(interval).as_slice() {
        [(tou, _)] => Some(*tou),
        _ => None,
    }
}

/// Whether `interval` lies entirely outside Toronto Hydro's `[07:00, 19:00)` demand window.
///
/// Defined in terms of [`Tou::OffPeak`] rather than as its own hour comparison, so the demand
/// window and the pricing periods cannot drift apart and there is only one holiday calendar to get
/// right. The two are independent concepts that happen to coincide exactly; see the module note.
pub fn is_off_peak(interval: Interval) -> bool {
    tou_partition(interval)
        .iter()
        .all(|(tou, _)| *tou == Tou::OffPeak)
}

fn tomorrow(d: Date) -> Date {
    d.tomorrow()
        .expect("a meter reading never sits on the last representable date")
}

// cargo test --package green-button --lib -- tou::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;

    fn local(y: i16, m: i8, d: i8, h: i8) -> Timestamp {
        local_hour(date(y, m, d), h as u8)
    }

    /// The hour beginning at local `h`. Built from a duration rather than from `h + 1`, so that it
    /// works for hour 23 and so that a daylight-saving day's hours are real elapsed hours rather
    /// than whatever the local clock reads an hour later.
    fn hour(y: i16, m: i8, d: i8, h: i8) -> Interval {
        Interval::new(local(y, m, d, h), std::time::Duration::from_secs(3600))
    }

    fn span(from: (i16, i8, i8, i8), to: (i16, i8, i8, i8)) -> Interval {
        Interval::from_start_end(
            local(from.0, from.1, from.2, from.3),
            local(to.0, to.1, to.2, to.3),
        )
    }

    /// Summer peaks midday and winter peaks at the shoulders. Getting this backwards is the single
    /// easiest mistake to make here, and it would misprice every weekday of the year.
    #[test]
    fn the_seasons_swap_on_peak_and_mid_peak() {
        assert_eq!(tou_of(hour(2026, 6, 15, 13)), Some(Tou::OnPeak)); // summer midday
        assert_eq!(tou_of(hour(2026, 6, 15, 8)), Some(Tou::MidPeak)); // summer shoulder
        assert_eq!(tou_of(hour(2026, 1, 15, 13)), Some(Tou::MidPeak)); // winter midday
        assert_eq!(tou_of(hour(2026, 1, 15, 8)), Some(Tou::OnPeak)); // winter shoulder
    }

    /// Off-peak does not move between seasons.
    #[test]
    fn off_peak_is_season_invariant() {
        for (m, d) in [(6, 15), (1, 15)] {
            assert_eq!(tou_of(hour(2026, m, d, 3)), Some(Tou::OffPeak));
            assert_eq!(tou_of(hour(2026, m, d, 20)), Some(Tou::OffPeak));
            assert_eq!(tou_of(hour(2026, m, d, 6)), Some(Tou::OffPeak));
            assert_eq!(tou_of(hour(2026, m, d, 19)), Some(Tou::OffPeak));
        }
    }

    /// The season boundary is May 1 / November 1 at local midnight.
    #[test]
    fn the_season_changes_at_local_midnight_on_may_1_and_november_1() {
        assert_eq!(tou_of(hour(2026, 4, 30, 13)), Some(Tou::MidPeak)); // still winter
        assert_eq!(tou_of(hour(2026, 5, 1, 13)), Some(Tou::OnPeak)); // summer
        assert_eq!(tou_of(hour(2025, 10, 31, 13)), Some(Tou::OnPeak)); // still summer
        assert_eq!(tou_of(hour(2025, 11, 3, 13)), Some(Tou::MidPeak)); // winter (Nov 1 is a Sat)
    }

    /// Weekends are off-peak around the clock in both seasons.
    #[test]
    fn weekends_are_entirely_off_peak() {
        for h in 0..24 {
            assert_eq!(tou_of(hour(2026, 6, 13, h)), Some(Tou::OffPeak)); // Saturday
            assert_eq!(tou_of(hour(2026, 6, 14, h)), Some(Tou::OffPeak)); // Sunday
        }
    }

    /// The Civic Holiday is why this crate carries its own holiday calendar. A Monday in August
    /// that would otherwise be on-peak from 11:00 must be off-peak all day.
    #[test]
    fn the_civic_holiday_is_off_peak_all_day() {
        for h in 0..24 {
            assert_eq!(tou_of(hour(2025, 8, 4, h)), Some(Tou::OffPeak));
        }
        // The Monday before and after are ordinary summer weekdays.
        assert_eq!(tou_of(hour(2025, 7, 28, 13)), Some(Tou::OnPeak));
        assert_eq!(tou_of(hour(2025, 8, 11, 13)), Some(Tou::OnPeak));
    }

    /// Maximality: a weekend is one off-peak run, not one piece per calendar day.
    #[test]
    fn adjacent_pieces_sharing_a_period_are_merged_across_days() {
        // Fri 2026-01-16 18:00 through Mon 2026-01-19 08:00, in winter.
        let got = tou_partition(span((2026, 1, 16, 18), (2026, 1, 19, 8)));
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0].0, Tou::OnPeak);
        assert_eq!(
            got[0].1,
            Interval::from_start_end(local(2026, 1, 16, 18), local(2026, 1, 16, 19))
        );
        assert_eq!(got[1].0, Tou::OffPeak);
        assert_eq!(
            got[1].1,
            Interval::from_start_end(local(2026, 1, 16, 19), local(2026, 1, 19, 7))
        );
        assert_eq!(got[2].0, Tou::OnPeak);
        assert_eq!(
            got[2].1,
            Interval::from_start_end(local(2026, 1, 19, 7), local(2026, 1, 19, 8))
        );
    }

    /// The four guarantees the doc comment makes, over a span long enough to cross seasons,
    /// weekends, holidays and both daylight-saving transitions.
    #[test]
    fn the_partition_covers_the_input_exactly() {
        let whole = span((2025, 10, 24, 0), (2026, 6, 24, 0));
        let pieces = tou_partition(whole);
        assert!(!pieces.is_empty());
        assert!(
            pieces.iter().all(|(_, i)| !i.is_empty()),
            "no piece may be empty"
        );
        assert_eq!(
            pieces[0].1.start, whole.start,
            "must start where the input starts"
        );
        assert_eq!(
            pieces.last().unwrap().1.end(),
            whole.end(),
            "must end where the input ends"
        );
        for w in pieces.windows(2) {
            assert_eq!(w[0].1.end(), w[1].1.start, "pieces must be contiguous");
            assert_ne!(
                w[0].0, w[1].0,
                "adjacent pieces must differ, or they would have merged"
            );
        }
        let total: u64 = pieces.iter().map(|(_, i)| i.duration.as_secs()).sum();
        assert_eq!(
            total,
            whole.duration.as_secs(),
            "the union must equal the input"
        );
    }

    /// The property the workbook's TOU column depends on: every hour of real data resolves to
    /// exactly one period. Run across both daylight-saving transitions, where a local day is 23 or
    /// 25 hours long.
    #[test]
    fn every_whole_hour_of_a_dst_day_resolves_to_one_period() {
        let mut checked = 0;
        for (from, to) in [
            ((2026, 3, 8, 0), (2026, 3, 9, 0)),   // spring forward: 23 hours
            ((2025, 11, 2, 0), (2025, 11, 3, 0)), // fall back: 25 hours
        ] {
            let day = span(from, to);
            let mut at = day.start;
            while at < day.end() {
                let h = Interval::new(at, std::time::Duration::from_secs(3600));
                assert!(tou_of(h).is_some(), "{at} straddles a boundary");
                at = h.end();
                checked += 1;
            }
        }
        assert_eq!(checked, 48, "23 + 25 hours");
    }

    /// A misaligned interval is reported rather than resolved.
    #[test]
    fn an_interval_straddling_a_boundary_has_no_single_period() {
        // 06:30-07:30 on a summer weekday crosses the 07:00 off-peak/mid-peak boundary.
        let start = date(2026, 6, 15)
            .at(6, 30, 0, 0)
            .to_zoned(crate::TIME_ZONE.clone())
            .unwrap()
            .timestamp();
        let straddling = Interval::new(start, std::time::Duration::from_secs(3600));
        assert_eq!(tou_of(straddling), None);
        assert_eq!(tou_partition(straddling).len(), 2);
    }

    #[test]
    fn an_empty_interval_partitions_to_nothing() {
        let at = local(2026, 6, 15, 12);
        assert!(tou_partition(Interval::from_start_end(at, at)).is_empty());
    }

    /// The invariant the demand-window columns rely on: outside the window is exactly off-peak.
    #[test]
    fn the_demand_window_is_the_complement_of_off_peak() {
        for h in 0..24 {
            let i = hour(2026, 6, 15, h); // an ordinary summer weekday
            let in_window = (7..19).contains(&h);
            assert_eq!(is_off_peak(i), !in_window, "hour {h}");
        }
    }

    #[test]
    fn every_tou_token_round_trips() {
        for t in [Tou::OnPeak, Tou::MidPeak, Tou::OffPeak] {
            assert_eq!(Tou::from_token(t.as_str()), Some(t));
        }
    }
}
