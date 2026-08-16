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
// Time grid
// ---------------------------------------------------------------------------

/// Resolution the session report states session boundaries at.
///
/// Currently, `Conn_DateTime_Start` and `Conn_DateTime_End` are truncated to whole minutes,
/// but it could change to whole seconds in the future. In the latter case, this value
/// should be changed to 1 second.
///
/// `Conn_Duration` and `Active_Charge_Time` are *not* truncaged — they carry seconds.
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

/// Rounds down a `Timestamp` to align with the time grid.
pub fn truncate_to_time_grid(ts: Timestamp) -> Timestamp {
    let step = TIME_GRID_STEP;
    let ts_nanos = ts.as_second();
    let rd_ts_secs = ts_nanos - ts_nanos.rem_euclid(step.as_secs() as i64);
    Timestamp::from_second(rd_ts_secs).unwrap_or_else(|_| {
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
