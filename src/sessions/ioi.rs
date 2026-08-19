//! What makes an interval of interest a *legal* one, in one place for every front-end.
//!
//! The library core stays permissive on purpose: [`interval_estimates`](crate::interval_estimates)
//! is happy with any interval, and exploratory callers and tests rely on that. What must not happen
//! is a *bill* being argued from an off-spec window, so every front-end that produces a quotable
//! figure comes through [`checked_interval`]. See docs/sessions/docs/sessions/README.md, "Interval of interest boundaries".
//!
//! The command line and the GUI ask the same questions in different orders — the one parses text
//! and reports what is wrong with it, the other offers only choices that are right — so what is
//! shared here is the rules themselves, not their presentation.

use crate::time::{Interval, TZ_OFFSETS, time_zone};
// Named only by the doc links below, which is enough to make them resolve and is why the import is
// here at all. Those three links were dead while `TIME_ZONE_NAME` was private.
#[allow(unused_imports)]
use crate::time::TIME_ZONE_NAME;
use jiff::{
    SignedDuration, Timestamp, civil,
    tz::{Offset, TimeZone},
};

/// The four legal start minutes. See docs/sessions/docs/sessions/README.md, "Interval of interest boundaries".
pub const LEGAL_START_MINUTES: [i8; 4] = [0, 15, 30, 45];

/// The two lengths an interval of interest may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoiLength {
    /// 15 minutes, legal from any of [`LEGAL_START_MINUTES`].
    FifteenMinutes,
    /// 1 hour, legal only from `HH:00`.
    Hour,
}

impl IoiLength {
    pub fn minutes(self) -> i64 {
        match self {
            Self::FifteenMinutes => 15,
            Self::Hour => 60,
        }
    }

    /// How the length is spelled on the command line.
    pub fn label(self) -> &'static str {
        match self {
            Self::FifteenMinutes => "15m",
            Self::Hour => "1h",
        }
    }

    /// The length a start on `minute` gets when none is named: the only one it permits.
    pub fn default_for(minute: i8) -> Self {
        if minute == 0 {
            Self::Hour
        } else {
            Self::FifteenMinutes
        }
    }

    /// Whether a start on `minute` may run for this length.
    pub fn allowed_from(self, minute: i8) -> bool {
        self != Self::Hour || minute == 0
    }
}

/// What instants a local wall time names in [`TIME_ZONE_NAME`].
///
/// Every case falls out of one question asked per offset in [`TZ_OFFSETS`]: read the wall time *as if*
/// at that fixed offset, and check the zone really is at that offset on the instant you land on.
/// The number of offsets that survive says which situation this is, so the gap and the fold need no
/// special-casing and a designator can be checked against the date rather than merely believed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TzLocalMapping {
    /// Exactly one instant. Every wall time except during the two transitions each year.
    Unique(Timestamp),
    /// Two instants an hour apart, on the night DST ends, each paired with the name that picks it
    /// out.
    Twice(Vec<(&'static str, Timestamp)>),
    /// No instant: the clocks jump forward over this wall time when DST begins.
    Never,
}

/// Maps a local wall time to the instant or instants it names.
///
/// This reports the ambiguity rather than resolving it, because at this point there is nothing to
/// resolve it *with*: a user has named a wall time and that is all. The session reader faces the
/// same ambiguity with more evidence — an untruncated `Conn_Duration` — and settles it. See
/// `CsvSession::resolve` for why the two are deliberately separate.
pub fn map_local(dt: civil::DateTime) -> TzLocalMapping {
    let tz = time_zone();
    let candidates: Vec<(&'static str, Timestamp)> = TZ_OFFSETS
        .iter()
        .filter_map(|(name, hours)| {
            let offset = Offset::constant(*hours);
            let ts = dt.to_zoned(TimeZone::fixed(offset)).ok()?.timestamp();
            (tz.to_offset(ts) == offset).then_some((*name, ts))
        })
        .collect();

    match candidates.len() {
        0 => TzLocalMapping::Never,
        1 => TzLocalMapping::Unique(candidates[0].1),
        _ => TzLocalMapping::Twice(candidates),
    }
}

/// An hour that occurs at least once on `date`.
///
/// The hour the clocks jump over when DST begins is not among them: it names no instant, so there
/// is nothing to offer and nothing to explain. The hour repeated when DST ends is present once,
/// marked, because there the caller has a real question to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HourEntry {
    pub hour: i8,
    /// Whether choosing this hour needs an EST/EDT answer before it names an instant.
    pub ambiguous: bool,
}

/// The hours a front-end may offer on `date`, in order.
///
/// Each hour is judged from all four of [`LEGAL_START_MINUTES`] rather than from `HH:00` alone.
/// In [`TIME_ZONE_NAME`] the transitions fall on the hour and the four always agree, so this is
/// only insurance — and cheap insurance, because a zone whose minutes disagreed would still be
/// caught by [`checked_interval`] rather than yielding a figure for the wrong instant.
pub fn hours_of(date: civil::Date) -> Vec<HourEntry> {
    let mut entries = Vec::with_capacity(24);
    for hour in 0..24 {
        let mut occurs = false;
        let mut ambiguous = false;
        for minute in LEGAL_START_MINUTES {
            match map_local(date.at(hour, minute, 0, 0)) {
                TzLocalMapping::Never => {}
                TzLocalMapping::Unique(_) => occurs = true,
                TzLocalMapping::Twice(_) => {
                    occurs = true;
                    ambiguous = true;
                }
            }
        }
        if occurs {
            entries.push(HourEntry { hour, ambiguous });
        }
    }
    entries
}

/// Turns a local wall time into the instant it names, refusing rather than guessing when it names
/// none or two.
pub fn resolve_local(dt: civil::DateTime, designator: Option<&str>) -> Result<Timestamp, String> {
    let names =
        |cs: &[(&str, Timestamp)]| cs.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" or ");

    match (designator, map_local(dt)) {
        (_, TzLocalMapping::Never) => Err(format!(
            "{dt} never occurred in local time zone: the clocks jump forward over it when DST \
             begins. Pick a time outside the skipped hour."
        )),
        (None, TzLocalMapping::Unique(ts)) => Ok(ts),
        (None, TzLocalMapping::Twice(several)) => Err(format!(
            "{dt} occurs twice in local time zone, an hour apart, because DST ends that night. \
             Add {} to say which is meant.",
            names(&several)
        )),
        (Some(want), mapping) => {
            let cs: Vec<(&str, Timestamp)> = match mapping {
                TzLocalMapping::Unique(ts) => {
                    // The name that picks out a wall time occurring once is whichever offset the
                    // zone is actually on then.
                    let tz = time_zone();
                    TZ_OFFSETS
                        .iter()
                        .filter(|(_, hours)| tz.to_offset(ts) == Offset::constant(*hours))
                        .map(|(name, _)| (*name, ts))
                        .collect()
                }
                TzLocalMapping::Twice(several) => several,
                TzLocalMapping::Never => unreachable!("handled above"),
            };
            cs.iter()
                .find(|(name, _)| want.eq_ignore_ascii_case(name))
                .map(|(_, ts)| *ts)
                .ok_or_else(|| {
                    format!(
                        "{dt} is not {} in local time zone; that date is on {}.",
                        want.to_uppercase(),
                        names(&cs)
                    )
                })
        }
    }
}

/// Turns a legal local start into a UTC interval, enforcing README's boundary rules.
///
/// `length` defaults to the only one the start minute permits. A designator is checked against the
/// date rather than believed, so naming the wrong one is an error and not an estimate for the wrong
/// hour.
/// The [`Interval`] is built here rather than by each caller, so the boundary rules and the type
/// that carries their result stay in the one place this module exists to keep them.
pub fn checked_interval(
    start: civil::DateTime,
    length: Option<IoiLength>,
    designator: Option<&str>,
) -> Result<Interval, String> {
    if start.second() != 0 || start.subsec_nanosecond() != 0 {
        return Err(format!(
            "interval start {start} carries seconds; it must be a whole minute"
        ));
    }
    if !LEGAL_START_MINUTES.contains(&start.minute()) {
        return Err(format!(
            "interval start {start} is not on a 15-minute boundary; it must be HH:00, HH:15, \
             HH:30 or HH:45. The peak a demand charge bills is a 15-minute average, so an \
             interval that does not line up with those boundaries cannot be compared to a bill."
        ));
    }

    let length = length.unwrap_or_else(|| IoiLength::default_for(start.minute()));
    if !length.allowed_from(start.minute()) {
        return Err(format!(
            "an interval of 1 hour must start at HH:00, but {start} starts at :{:02}. An hour \
             is reported as the highest of the four 15-minute segments inside it, and those \
             segments have to line up with the quarter hours a demand charge is billed on.",
            start.minute()
        ));
    }

    let lo = resolve_local(start, designator)?;
    let hi = lo + SignedDuration::from_mins(length.minutes());
    Ok(Interval::from_start_end(lo, hi))
}

#[cfg(test)]
mod test {
    use super::*;

    fn utc(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn dt(s: &str) -> civil::DateTime {
        s.replace('T', " ").parse().unwrap()
    }

    fn itvl(lo: &str, hi: &str) -> Interval {
        Interval::from_start_end(utc(lo), utc(hi))
    }

    #[test]
    fn legal_intervals_resolve_to_utc() {
        // 16:00 EDT is 20:00Z in June.
        assert_eq!(
            checked_interval(dt("2026-06-01 16:00"), Some(IoiLength::Hour), None).unwrap(),
            itvl("2026-06-01T20:00:00Z", "2026-06-01T21:00:00Z")
        );
        assert_eq!(
            checked_interval(
                dt("2026-06-01 16:45"),
                Some(IoiLength::FifteenMinutes),
                None
            )
            .unwrap(),
            itvl("2026-06-01T20:45:00Z", "2026-06-01T21:00:00Z")
        );
    }

    /// Length defaults to the only one the start permits.
    #[test]
    fn length_defaults_by_start_minute() {
        assert_eq!(
            checked_interval(dt("2026-06-01 16:00"), None, None).unwrap(),
            checked_interval(dt("2026-06-01 16:00"), Some(IoiLength::Hour), None).unwrap()
        );
        assert_eq!(
            checked_interval(dt("2026-06-01 16:15"), None, None).unwrap(),
            checked_interval(
                dt("2026-06-01 16:15"),
                Some(IoiLength::FifteenMinutes),
                None
            )
            .unwrap()
        );
        assert_eq!(IoiLength::default_for(0), IoiLength::Hour);
        assert_eq!(IoiLength::default_for(45), IoiLength::FifteenMinutes);
    }

    #[test]
    fn off_spec_intervals_are_rejected() {
        // Not on a quarter hour.
        assert!(checked_interval(dt("2026-06-01 16:07"), None, None).is_err());
        // An hour must start on the hour.
        assert!(checked_interval(dt("2026-06-01 16:15"), Some(IoiLength::Hour), None).is_err());
        // Seconds are not a whole minute.
        assert!(checked_interval(dt("2026-06-01 16:00:30"), None, None).is_err());
    }

    /// The rule the GUI disables its 1-hour button by is the rule the interval is checked against.
    #[test]
    fn the_length_rule_is_stated_once() {
        for minute in LEGAL_START_MINUTES {
            let start = civil::date(2026, 6, 1).at(16, minute, 0, 0);
            for length in [IoiLength::FifteenMinutes, IoiLength::Hour] {
                assert_eq!(
                    length.allowed_from(minute),
                    checked_interval(start, Some(length), None).is_ok(),
                    "{length:?} from :{minute:02}"
                );
            }
        }
    }

    /// 2026-03-08 02:00 local never happened: DST skips from 02:00 to 03:00. Nothing can
    /// disambiguate a time that does not exist, so a designator does not help either.
    #[test]
    fn a_start_in_the_dst_gap_is_refused() {
        for designator in [None, Some("EST"), Some("EDT")] {
            assert!(
                checked_interval(dt("2026-03-08 02:00"), Some(IoiLength::Hour), designator)
                    .is_err()
            );
        }
    }

    /// 2026-11-01 01:00 local happens twice, an hour apart, when DST ends. Bare it is refused; with
    /// a designator it resolves, and the two readings are exactly an hour apart.
    #[test]
    fn a_fold_start_needs_a_designator_and_then_resolves() {
        let msg =
            checked_interval(dt("2026-11-01 01:00"), Some(IoiLength::Hour), None).unwrap_err();
        assert!(msg.contains("occurs twice"), "{msg}");
        assert!(msg.contains("EST") && msg.contains("EDT"), "{msg}");

        // 01:00 EDT is 05:00Z; 01:00 EST is 06:00Z.
        let edt =
            checked_interval(dt("2026-11-01 01:00"), Some(IoiLength::Hour), Some("EDT")).unwrap();
        let est =
            checked_interval(dt("2026-11-01 01:00"), Some(IoiLength::Hour), Some("EST")).unwrap();
        assert_eq!(edt.start, utc("2026-11-01T05:00:00Z"));
        assert_eq!(est.start, utc("2026-11-01T06:00:00Z"));
        assert_eq!(est.start.duration_since(edt.start).as_secs(), 3600);

        // The EDT hour ends where the EST hour begins: the interval spans the fold.
        assert_eq!(edt.end(), est.start);
        assert_eq!(est.end(), utc("2026-11-01T07:00:00Z"));
    }

    /// A designator is checked against the date rather than believed, so naming the wrong one is an
    /// error and not an estimate for the wrong hour. It is accepted, and redundant, the rest of the
    /// year.
    #[test]
    fn a_designator_is_validated_against_the_date() {
        let hour = Some(IoiLength::Hour);

        // June is on EDT, so EDT is redundant but correct and EST is simply wrong.
        assert_eq!(
            checked_interval(dt("2026-06-01 16:00"), hour, Some("EDT")).unwrap(),
            checked_interval(dt("2026-06-01 16:00"), hour, None).unwrap()
        );
        let msg = checked_interval(dt("2026-06-01 16:00"), hour, Some("EST")).unwrap_err();
        assert!(msg.contains("is not EST"), "{msg}");
        assert!(msg.contains("EDT"), "{msg}");

        // January is on EST, and the mirror image holds.
        assert_eq!(
            checked_interval(dt("2026-01-15 16:00"), hour, Some("EST")).unwrap(),
            checked_interval(dt("2026-01-15 16:00"), hour, None).unwrap()
        );
        assert!(checked_interval(dt("2026-01-15 16:00"), hour, Some("EDT")).is_err());

        // Case does not matter.
        assert_eq!(
            checked_interval(dt("2026-11-01 01:00"), hour, Some("edt")).unwrap(),
            checked_interval(dt("2026-11-01 01:00"), hour, Some("EDT")).unwrap()
        );
    }

    /// An ordinary date offers all 24 hours and asks nothing.
    #[test]
    fn an_ordinary_date_offers_every_hour() {
        let hours = hours_of(civil::date(2026, 6, 15));
        assert_eq!(hours.len(), 24);
        assert!(hours.iter().all(|h| !h.ambiguous));
        assert_eq!(hours[0].hour, 0);
        assert_eq!(hours[23].hour, 23);
    }

    /// The hour the clocks jump over is not offered at all: there is nothing to choose between.
    #[test]
    fn the_dst_gap_hour_is_not_offered() {
        let hours = hours_of(civil::date(2026, 3, 8));
        assert_eq!(hours.len(), 23);
        assert!(!hours.iter().any(|h| h.hour == 2), "02 should be absent");
        assert!(hours.iter().any(|h| h.hour == 1));
        assert!(hours.iter().any(|h| h.hour == 3));
        assert!(hours.iter().all(|h| !h.ambiguous));
    }

    /// The repeated hour is offered once and marked, because there the caller has a real question
    /// to answer.
    #[test]
    fn the_dst_fold_hour_is_offered_once_and_marked() {
        let hours = hours_of(civil::date(2026, 11, 1));
        assert_eq!(hours.len(), 24);
        let ambiguous: Vec<i8> = hours
            .iter()
            .filter(|h| h.ambiguous)
            .map(|h| h.hour)
            .collect();
        assert_eq!(ambiguous, vec![1]);
    }
}
