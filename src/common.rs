use jiff::{Timestamp, Zoned, tz::TimeZone};
use std::{cell::RefCell, fmt, rc::Rc, time::Duration};

/// Time zone the session report's timestamps are stated in. See README.md, "Time zone".
pub const TIME_ZONE_NAME: &str = "America/Toronto";

/// Sessions whose overlap with the interval of interest is less than this
/// are excluded from the calculations.
pub const OVERLAP_THRESHOLD: Duration = Duration::from_secs(60);

pub const EV_POWER_FACTOR: f64 = 0.95;
pub const EVOLUTE_BREAKER_KW_RATING: f64 = 6.7;
pub const EVOLUTE_BREAKER_KVA_RATING: f64 = 7.5;

pub(crate) fn time_zone() -> TimeZone {
    TimeZone::get(TIME_ZONE_NAME).expect("America/Toronto should be a valid time-zone name")
}

// ---------------------------------------------------------------------------
// Anomalies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyKind {
    /// `Active_Charge_Time` is zero so its `Avg_power` cell shows `#DIV/0!`.
    ZeroActiveChargeTime,
    /// The start fell in the DST fold and both offsets reproduce the reported end,
    /// so the record was duplicated. See README.md, "Time zone".
    DstAmbiguousDuplicated,
    /// The start fell in the DST gap, i.e. a wall time that never occurred.
    /// Resolved forward to the instant just after the gap.
    DstGapShifted,
    /// The start fell in the DST fold and *neither* offset reproduces the reported end.
    /// The earlier offset was assumed.
    DstUnresolvable,
    /// Session ends before it starts.
    EndBeforeStart,
    /// Session intersects with the interval of interest but the overlap is less than
    /// [`OVERLAP_THRESHOLD`].
    IntersectsBelowThreshold,
}

/// A single row that needs review. Does not abort the conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomaly {
    /// Excel row number.
    pub row: usize,
    pub session_id: String,
    pub kind: AnomalyKind,
}

impl fmt::Display for AnomalyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ZeroActiveChargeTime => "zero Active_Charge_Time",
            Self::DstAmbiguousDuplicated => "ambiguous DST fold; record duplicated as EDT and EST",
            Self::DstGapShifted => "local time falls in the DST gap; resolved forward",
            Self::DstUnresolvable => "DST fold matches neither offset; assumed the earlier one",
            Self::EndBeforeStart => "session ends before it starts",
            Self::IntersectsBelowThreshold => {
                "session intersects with the interval of interest but the overlap is less than OVERLAP_THRESHOLD"
            }
        };
        f.write_str(s)
    }
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "row {} ({}): {}", self.row, self.session_id, self.kind)
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

pub(crate) type RSession = Rc<RefCell<Session>>;

#[derive(Debug)]
/// Charging session
pub struct Session {
    /// From `session report`.
    pub id: String,
    /// 1-based CSV data row, excluding the header.
    pub row: usize,
    /// Conection start date-time (UTC) from `session report`.
    pub conn_start: Timestamp,
    /// Non-adjusted conection end date-time (UTC) from `session report`.
    pub raw_conn_end: Timestamp,
    /// Adjusted conection end date-time (UTC) from `session report`.
    pub conn_end: Timestamp,
    /// Active charge time from `session report`.
    /// May differ from `conn_end - conn_start` due to `conn_end` for various reasons, including
    /// ingestion adjustment.
    pub charge_time: Duration,
    /// From `session report`.
    pub energy_use: f64,
    /// `energy_use / charge_time in hours`.
    pub avg_power: f64,
    /// Anomalies associated with this session.
    pub anomalies: Vec<Anomaly>,
}

impl Session {
    /// Connection start in local time (ET).
    pub fn conn_start_local(&self) -> Zoned {
        Zoned::new(self.conn_start, time_zone())
    }

    /// Non-adjusted conection end in local time (ET).
    pub fn raw_conn_end_local(&self) -> Zoned {
        Zoned::new(self.raw_conn_end, time_zone())
    }

    /// Adjusted conection end in local time (ET).
    pub fn conn_end_local(&self) -> Zoned {
        Zoned::new(self.conn_end, time_zone())
    }
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Session {}

impl PartialOrd for Session {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.id.partial_cmp(&other.id)
    }
}

impl Ord for Session {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}
