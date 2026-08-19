use jiff::{
    Timestamp,
    civil::{Date, DateTime},
    tz::TimeZone,
};
use std::{sync::LazyLock, time::Duration};

// ---------------------------------------------------------------------------
// Date/time
// ---------------------------------------------------------------------------

/// Time zone the session report's timestamps are stated in. See README.md, "Time zone".
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
// Time grids
// ---------------------------------------------------------------------------
//
// A grid is a step, and these two functions are everything the crate does with one. The step
// itself belongs to whichever module has a reason for its value: `sessions::TIME_GRID_STEP` is
// the resolution session boundaries are reported at, `green_button::METER_INTERVAL` the interval
// the meter records. Neither is a property of time.

/// Rounds a `Timestamp` down to the nearest multiple of `step`, counting from the Unix epoch.
///
/// The defining property, which [`test::truncation_brackets_its_input`] states and everything
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

    const MINUTE: Duration = Duration::from_secs(60);

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
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
/// Time interval. Must be on the time grid defined by [`TIME_GRID_STEP`].
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
