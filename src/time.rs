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
