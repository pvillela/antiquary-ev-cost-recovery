use jiff::{Timestamp, tz::TimeZone};
use std::{sync::LazyLock, time::Duration};

// ---------------------------------------------------------------------------
// Date/time
// ---------------------------------------------------------------------------

/// Time zone the session report's timestamps are stated in. See README.md, "Time zone".
const TIME_ZONE_NAME: &str = "America/Toronto";

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

/// Resolution the session report states session boundaries at: `Conn_DateTime_Start` and
/// `Conn_DateTime_End` are truncated to whole minutes. `Conn_Duration` and `Active_Charge_Time`
/// are *not* — they carry seconds, which is what makes the DST fold inference possible.
///
/// Every allowance the software makes for that truncation is this one value, so all of them move
/// together should Evolute ever report seconds:
///
/// - Added to the reported session end to give `adj_conn_end`, the session's exclusive end.
/// - The half-width of the band a sound record's `Conn_start + Conn_Duration` must land in.
///
/// This constant defines the step of the time grid on which this software relies.
///
/// Must divide [`SEGMENT_DURATION`] without leaving a remainder. Otherwise, [`Segment`]s
/// partitioning the interval of interest will not be on the time grid.
///
/// See README.md, "Boundaries and the time grid".
pub const TIME_GRID_STEP: Duration = Duration::from_secs(60);

/// Rounds down a `Timestamp` by the specified `step`.
pub fn round_down_timestamp(ts: Timestamp, step: Duration) -> Timestamp {
    let ts_nanos = ts.as_nanosecond();
    let rd_ts_nanos = ts_nanos - ts_nanos.rem_euclid(step.as_nanos() as i128);
    Timestamp::from_nanosecond(rd_ts_nanos).unwrap_or_else(|_| {
        panic!("rounding down Timestamp {ts:?} by step {step:?} that is too large")
    })
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
