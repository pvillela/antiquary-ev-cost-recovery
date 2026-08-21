use jiff::{
    Timestamp,
    civil::{Date, DateTime},
    tz::{Offset, TimeZone},
};
use std::{sync::LazyLock, time::Duration};

// ---------------------------------------------------------------------------
// Date/time
// ---------------------------------------------------------------------------

/// Time zone the session report's timestamps are stated in. See docs/time/README.md, "Time zone".
///
/// Public because both binaries and several doc comments name it, and a reader who finds
/// "in local time" in a message needs somewhere to learn which zone that is.
pub const TIME_ZONE_NAME: &str = "America/Toronto";

/// The offsets [`TIME_ZONE_NAME`] uses, under the names a reader of a Toronto Hydro bill will
/// recognise. Naming one resolves a wall time that occurs twice.
///
/// Here rather than in `sessions` because it is a property of the zone, and the zone is shared.
pub const TZ_OFFSETS: [(&str, i8); 2] = [("EST", -5), ("EDT", -4)];

static TIME_ZONE: LazyLock<TimeZone> = LazyLock::new(|| {
    TimeZone::get(TIME_ZONE_NAME).expect("America/Toronto should be a valid time-zone name")
});

/// Resolved once. Every local-time question in the crate goes through here, so there is one answer
/// to "which zone" rather than one per module.
pub fn time_zone() -> TimeZone {
    TIME_ZONE.clone()
}

pub(crate) fn duration(start: Timestamp, end: Timestamp) -> Duration {
    Duration::try_from(end.duration_since(start))
        .unwrap_or_else(|_| panic!("interval ends at {} before it starts at {}", end, start))
}

/// The local calendar date an instant falls on.
pub fn local_date(ts: Timestamp) -> Date {
    ts.to_zoned(time_zone()).date()
}

/// The local wall-clock reading of an instant, for the workbook's local-time columns.
pub(crate) fn local_datetime(ts: Timestamp) -> DateTime {
    ts.to_zoned(time_zone()).datetime()
}

/// The instant a given local hour begins on a given local date.
///
/// # Panics
///
/// Panics if the local time falls in a daylight-saving gap or fold. Callers pass 0, 7, 11, 17 or
/// 19; Ontario's transitions are at 02:00, so none of them can.
pub(crate) fn local_hour(d: Date, hour: u8) -> Timestamp {
    d.at(hour as i8, 0, 0, 0)
        .to_zoned(time_zone())
        .expect("callers pass hours that never fall in a daylight-saving transition")
        .timestamp()
}

/// The instant a local date begins.
pub(crate) fn local_midnight(d: Date) -> Timestamp {
    local_hour(d, 0)
}

// ---------------------------------------------------------------------------
// Standard time
// ---------------------------------------------------------------------------
//
// Toronto Hydro cuts a billing period on standard time all year round: the meter's own clock does
// not observe daylight saving, and neither does the period boundary read off it. Prevailing local
// time is still what everything else here means by "local" -- Time-of-Use periods, the 07:00-19:00
// demand window and the holiday calendar are all stated in the clock a customer reads, and they
// keep `local_date` and `local_hour` above.
//
// The evidence for the rule, and for that division, is in
// `docs/hydro_bills/archive/dst-energy-anomaly-pre-fix.md`: an EST-fixed window reproduces all 19
// invoices to the milli-kWh, while a prevailing-local one matches 6, and the on-peak and mid-peak
// energy the bills state is reproduced only by TOU periods left on prevailing time.

/// The offset a billing period is cut on, under the name a bill reader will recognise.
///
/// The standard-time entry of [`TZ_OFFSETS`], named rather than indexed so that the reason is
/// visible at the use site. `test::the_billing_offset_is_the_standard_time_one` pins it to the
/// entry it is meant to be, so reordering that array cannot silently move every period boundary.
pub const BILLING_OFFSET: (&str, i8) = TZ_OFFSETS[0];

static BILLING_ZONE: LazyLock<TimeZone> =
    LazyLock::new(|| TimeZone::fixed(Offset::constant(BILLING_OFFSET.1)));

/// The zone billing periods are cut in: a fixed offset, with no daylight-saving rule at all.
fn billing_zone() -> TimeZone {
    BILLING_ZONE.clone()
}

/// The standard-time calendar date an instant falls on.
///
/// Differs from [`local_date`] for instants in the hour after midnight during daylight saving:
/// 2025-06-24T00:30 EDT is still the 23rd on a standard-time clock.
pub(crate) fn standard_date(ts: Timestamp) -> Date {
    ts.to_zoned(billing_zone()).date()
}

/// The instant a standard-time date begins.
///
/// Unlike [`local_midnight`] this can never fail: a fixed offset has no gap for a wall time to
/// fall into and no fold for it to be ambiguous in.
pub(crate) fn standard_midnight(d: Date) -> Timestamp {
    d.at(0, 0, 0, 0)
        .to_zoned(billing_zone())
        .expect("a fixed offset has neither gaps nor folds")
        .timestamp()
}

// ---------------------------------------------------------------------------
// Time grids
// ---------------------------------------------------------------------------
//
// A grid is a step, and these two functions are everything the crate does with one. The step
// itself belongs to whichever module has a reason for its value: `sessions::TIME_GRID_STEP` is
// the resolution session boundaries are reported at, `green_button::METER_INTERVAL` the interval
// the meter records. Neither is a property of time.

/// Rounds a `Timestamp` down to the nearest multiple of `step`, counting from the Unix epoch.
///
/// The defining property, which `test::truncation_brackets_its_input` states and everything
/// built on this relies on:
///
/// ```text
/// truncate_to(ts, step) <= ts < truncate_to(ts, step) + step
/// ```
///
/// That is the `Givens` line of `docs/sessions/time-reporting-uncertainty.md`, and it is what
/// makes `adj_conn_start <= real_start` true.
///
/// # Panics
///
/// If `step` is zero, or so large that the truncated instant falls outside the representable
/// range. Neither is reachable from any caller in this crate.
pub fn truncate_to(ts: Timestamp, step: Duration) -> Timestamp {
    let step_secs = step.as_secs() as i64;
    assert!(
        step_secs > 0,
        "a time grid step must be positive, got {step:?}"
    );
    let secs = ts.as_second();
    // `rem_euclid`, not `%`: the remainder must be non-negative so that a pre-epoch instant
    // truncates backwards like every other one. With `%` a negative timestamp would round towards
    // zero, i.e. forwards, and break the bracket above.
    let truncated = secs - secs.rem_euclid(step_secs);
    Timestamp::from_second(truncated)
        .unwrap_or_else(|_| panic!("truncating {ts:?} to step {step:?} left the valid range"))
}

/// Whether an instant lies exactly on the grid `step` defines.
///
/// The companion of [`truncate_to`]: `is_on_grid(ts, step)` is true exactly when
/// `truncate_to(ts, step) == ts`. Callers use it to ask whether truncation *would* move an
/// instant, so the two must agree.
pub fn is_on_grid(ts: Timestamp, step: Duration) -> bool {
    let step_secs = step.as_secs() as i64;
    assert!(
        step_secs > 0,
        "a time grid step must be positive, got {step:?}"
    );
    ts.as_second().rem_euclid(step_secs) == 0
}

// cargo test --lib -- time::base::test --nocapture
#[cfg(test)]
mod test {
    use super::*;

    use jiff::civil::date;

    const MINUTE: Duration = Duration::from_secs(60);

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// [`BILLING_OFFSET`] is taken from [`TZ_OFFSETS`] by index, so this pins down which entry it
    /// is meant to be. Reordering that array would otherwise move every billing period boundary by
    /// an hour with nothing to say so.
    #[test]
    fn the_billing_offset_is_the_standard_time_one() {
        assert_eq!(BILLING_OFFSET, ("EST", -5));
        assert!(TZ_OFFSETS.contains(&BILLING_OFFSET));
    }

    /// Standard-time midnight is a fixed UTC-5 all year, where local midnight follows the clocks.
    /// In winter the two agree; in summer standard-time midnight is an hour later in UTC.
    #[test]
    fn standard_midnight_does_not_follow_daylight_saving() {
        // January: the prevailing offset is already -5, so the two coincide.
        assert_eq!(
            standard_midnight(date(2026, 1, 24)),
            local_midnight(date(2026, 1, 24))
        );
        assert_eq!(
            standard_midnight(date(2026, 1, 24)),
            ts("2026-01-24T05:00:00Z")
        );
        // June: prevailing local midnight is 04:00Z, standard-time midnight stays at 05:00Z.
        assert_eq!(
            local_midnight(date(2026, 6, 24)),
            ts("2026-06-24T04:00:00Z")
        );
        assert_eq!(
            standard_midnight(date(2026, 6, 24)),
            ts("2026-06-24T05:00:00Z")
        );
    }

    /// The hour between the two midnights belongs to the previous day on a standard-time clock.
    /// This is the hour every billing-period difference in `docs/hydro_bills/` came down to.
    #[test]
    fn the_midnight_hour_belongs_to_the_previous_standard_day() {
        let t = ts("2026-06-24T04:30:00Z"); // 00:30 EDT on the 24th
        assert_eq!(local_date(t), date(2026, 6, 24));
        assert_eq!(standard_date(t), date(2026, 6, 23));
    }

    /// Every standard-time day is exactly 24 hours, including the two the clocks change on. That
    /// is what makes a billing period always a whole number of days.
    #[test]
    fn standard_time_days_are_always_24_hours() {
        for (y, m, d) in [(2026, 3, 8), (2025, 11, 2), (2026, 6, 15)] {
            let start = standard_midnight(date(y, m, d));
            let next = standard_midnight(date(y, m, d) + jiff::Span::new().days(1));
            assert_eq!(
                next.as_second() - start.as_second(),
                24 * 3600,
                "{y}-{m}-{d}"
            );
        }
    }

    /// An instant already on the grid does not move, so truncation is idempotent.
    #[test]
    fn truncation_leaves_an_aligned_instant_alone() {
        for s in ["2026-06-15T20:00:00Z", "1970-01-01T00:00:00Z"] {
            let t = ts(s);
            assert_eq!(truncate_to(t, MINUTE), t, "{s}");
            assert_eq!(truncate_to(truncate_to(t, MINUTE), MINUTE), t, "{s} twice");
        }
    }

    /// Seconds are dropped, never rounded: the result is the step at or below the input.
    #[test]
    fn truncation_moves_backwards_never_forwards() {
        for (input, expected) in [
            ("2026-06-15T20:00:01Z", "2026-06-15T20:00:00Z"),
            ("2026-06-15T20:00:59Z", "2026-06-15T20:00:00Z"),
            ("2026-06-15T20:01:00Z", "2026-06-15T20:01:00Z"),
        ] {
            assert_eq!(truncate_to(ts(input), MINUTE), ts(expected), "{input}");
        }
    }

    /// The property everything else rests on, checked over every second of a minute rather than at
    /// a few chosen points.
    #[test]
    fn truncation_brackets_its_input() {
        let base = ts("2026-06-15T20:00:00Z");
        for offset in 0..600 {
            let t = base + Duration::from_secs(offset);
            let truncated = truncate_to(t, MINUTE);
            assert!(
                truncated <= t,
                "{t} truncated to {truncated}, which is later"
            );
            assert!(
                t < truncated + MINUTE,
                "{t} is not below {truncated} + step"
            );
        }
    }

    /// The two functions must agree, since callers use one to predict the other.
    #[test]
    fn is_on_grid_agrees_with_truncate_to() {
        let base = ts("2026-06-15T20:00:00Z");
        for offset in 0..300 {
            let t = base + Duration::from_secs(offset);
            assert_eq!(
                is_on_grid(t, MINUTE),
                truncate_to(t, MINUTE) == t,
                "disagreement at {t}"
            );
            // Whatever went in, what comes out is on the grid.
            assert!(is_on_grid(truncate_to(t, MINUTE), MINUTE), "{t}");
        }
    }

    /// A pre-epoch instant truncates backwards like any other.
    ///
    /// This is why the implementation uses `rem_euclid` rather than `%`. With `%` the remainder
    /// would be negative here and the instant would move *forwards*, breaking the bracket above
    /// for every timestamp before 1970. No caller reaches these dates today; the Excel epoch
    /// (1899-12-30) is one, and a corrupt feed is another.
    #[test]
    fn truncation_is_correct_before_the_unix_epoch() {
        assert_eq!(
            truncate_to(ts("1899-12-30T00:00:30Z"), MINUTE),
            ts("1899-12-30T00:00:00Z")
        );
        // The revealing case: `%` would give 1969-12-31T23:59:00Z, which is later than the input.
        let t = ts("1969-12-31T23:58:30Z");
        let truncated = truncate_to(t, MINUTE);
        assert_eq!(truncated, ts("1969-12-31T23:58:00Z"));
        assert!(truncated <= t);
    }

    /// The step is a parameter, and the callers do pass more than one.
    #[test]
    fn other_steps_work_the_same_way() {
        let t = ts("2026-06-15T20:37:42Z");
        assert_eq!(truncate_to(t, Duration::from_secs(1)), t);
        assert_eq!(
            truncate_to(t, Duration::from_secs(900)),
            ts("2026-06-15T20:30:00Z")
        );
        assert_eq!(
            truncate_to(t, Duration::from_secs(3600)),
            ts("2026-06-15T20:00:00Z")
        );
        assert!(is_on_grid(
            ts("2026-06-15T20:00:00Z"),
            Duration::from_secs(3600)
        ));
        assert!(!is_on_grid(t, Duration::from_secs(3600)));
    }
}

// ---------------------------------------------------------------------------
// Interval
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd)]
/// Time interval. Must be on the time grid defined by [`crate::sessions::TIME_GRID_STEP`].
pub struct Interval {
    pub start: Timestamp,
    pub duration: Duration,
}

impl Interval {
    pub fn new(start: Timestamp, duration: Duration) -> Interval {
        Self { start, duration }
    }

    pub fn from_start_end(start: Timestamp, end: Timestamp) -> Interval {
        let duration = duration(start, end);
        Self { start, duration }
    }

    pub fn end(&self) -> Timestamp {
        self.start + self.duration
    }

    pub fn is_empty(&self) -> bool {
        self.duration == Duration::ZERO
    }

    pub fn intersection(&self, other: &Interval) -> Self {
        let start = self.start.max(other.start);
        let end = self.end().min(other.end());
        let duration = if start <= end {
            duration(start, end)
        } else {
            Duration::ZERO
        };
        Self { start, duration }
    }
}
