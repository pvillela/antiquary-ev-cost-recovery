use jiff::{Timestamp, Zoned, tz::TimeZone};
use std::{cell::RefCell, fmt, rc::Rc, time::Duration};

/// Time zone the session report's timestamps are stated in. See README.md, "Time zone".
pub const TIME_ZONE_NAME: &str = "America/Toronto";

/// Added to the reported session end time. See README.md, "Session boundaries".
pub const CONNECTION_END_ADJUSTMENT: Duration = Duration::from_secs(59);

/// Boundary margin of the interval of interest. Reported session times are truncated to whole
/// minutes, so a session whose only overlap with the interval falls within this distance of a
/// boundary cannot be trusted to overlap it at all. A session takes part in the estimates only if
/// it is active somewhere in the interval reduced by this amount at each end; the margin applies
/// *only* at the boundaries, so a short session lying inside the interval is included.
/// See README.md, "Session boundaries".
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
    /// `Conn_start + Conn_Duration` misses the reported `Conn_DateTime_End` by 60 seconds or more,
    /// in one direction or the other, so the reported start, end and duration are mutually
    /// inconsistent.
    ///
    /// The tolerance is what makes this a real test rather than a formality. Writing the true start
    /// as `S + α` and the true end as `E + β`, with `α, β ∈ [0, 60)` because the report truncates
    /// both to the whole minute, an honest `Conn_Duration` gives `S + Conn_Duration = E + (β − α)`.
    /// So a sound record lands anywhere in `[E − 59s, E + 59s]` and never outside it. The upper
    /// bound is `Adj_conn_end`, which already carries those 59 seconds; the lower bound is the same
    /// distance below the reported end.
    ///
    /// Both directions are faults, and both exclude the session from the estimates: if a record's
    /// own fields disagree by more than the reporting can explain, neither its duration nor the
    /// span the grouping logic would place it on can be relied on. The overshoot direction also
    /// subsumes a session that ends before it starts — with `Conn_DateTime_End` a minute or more
    /// before `Conn_DateTime_Start`, no non-negative duration satisfies the test.
    ///
    /// See README.md, "Other".
    InconsistentDuration,
    /// The start fell in the DST fold and both offsets reproduce the reported end,
    /// so the record was duplicated. See README.md, "Time zone".
    DstAmbiguousDuplicated,
    /// The start fell in the DST gap, i.e. a wall time that never occurred.
    /// Resolved forward to the instant just after the gap.
    DstGapShifted,
    /// `Conn_DateTime_Start` falls in the DST fold, and for neither the EDT nor the EST reading
    /// does `Conn_start + Conn_Duration` land within a minute of the reported `Conn_DateTime_End`.
    ///
    /// The test is a tolerance rather than an equality, and that is what makes failing it mean
    /// something. The reported timestamps are truncated to the whole minute while `Conn_Duration`
    /// carries seconds, so even for a sound record the implied end misses the reported one — by up
    /// to 59 seconds, in either direction. Missing it is therefore normal; missing it by *a minute
    /// or more under both readings* is not, especially as the two readings sit a full hour apart,
    /// so one of them is ordinarily well inside the tolerance. When neither is, the record's own
    /// fields disagree by more than truncation can account for: whatever `Conn_Duration` measures
    /// on this row, it is not the elapsed time the inference assumes it to be.
    ///
    /// The earlier (EDT) reading is assumed so the row can still be processed, but this session's
    /// UTC timestamps may be an hour early.
    ///
    /// Only fold starts are checked this way; the same inconsistency on any other date is caught,
    /// if at all, by [`AnomalyKind::InconsistentDuration`]. See README.md, "Time zone".
    DstUnresolvable,
    /// Session intersects the interval of interest, but only within [`OVERLAP_THRESHOLD`] of a
    /// boundary. Reported times are truncated to whole minutes, so an overlap that small cannot be
    /// trusted; the session is excluded from the estimates.
    ///
    /// Unlike every other kind, this one depends on which interval of interest was chosen, so it
    /// cannot be recorded in the workbook's `Anomalies` column. It is added by the grouping logic.
    IntersectsBoundaryMarginOnly,
}

impl AnomalyKind {
    /// The variant name, as written to the workbook's `Anomalies` column. Deliberately distinct
    /// from [`fmt::Display`], which is free-form prose for humans and may be reworded at will;
    /// this is a wire format and must stay stable.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ZeroActiveChargeTime => "ZeroActiveChargeTime",
            Self::InconsistentDuration => "InconsistentDuration",
            Self::DstAmbiguousDuplicated => "DstAmbiguousDuplicated",
            Self::DstGapShifted => "DstGapShifted",
            Self::DstUnresolvable => "DstUnresolvable",
            Self::IntersectsBoundaryMarginOnly => "IntersectsBoundaryMarginOnly",
        }
    }

    /// Inverse of [`AnomalyKind::as_str`]. `None` for an unrecognised token.
    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "ZeroActiveChargeTime" => Self::ZeroActiveChargeTime,
            "InconsistentDuration" => Self::InconsistentDuration,
            "DstAmbiguousDuplicated" => Self::DstAmbiguousDuplicated,
            "DstGapShifted" => Self::DstGapShifted,
            "DstUnresolvable" => Self::DstUnresolvable,
            "IntersectsBoundaryMarginOnly" => Self::IntersectsBoundaryMarginOnly,
            _ => return None,
        })
    }
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
            Self::InconsistentDuration => {
                "Conn_start + Conn_Duration misses Conn_DateTime_End by a minute or more; start, \
                 end and duration are inconsistent"
            }
            Self::DstAmbiguousDuplicated => "ambiguous DST fold; record duplicated as EDT and EST",
            Self::DstGapShifted => "local time falls in the DST gap; resolved forward",
            Self::DstUnresolvable => {
                "DST fold: neither EDT nor EST reproduces the reported end, so the record is \
                 inconsistent; assumed EDT, timestamps may be an hour early"
            }
            Self::IntersectsBoundaryMarginOnly => {
                "session overlaps the interval of interest only within OVERLAP_THRESHOLD of a boundary"
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
    /// Row number in the Excel workbook. The header occupies row 1, so the lowest possible value
    /// is 2. This is *not* the CSV row: a record duplicated to resolve a DST fold occupies two
    /// workbook rows, so the two diverge from that point on.
    pub row: usize,
    /// Conection start date-time (UTC) from `session report`.
    pub conn_start: Timestamp,
    /// Non-adjusted conection end date-time (UTC) from `session report`.
    pub raw_conn_end: Timestamp,
    /// Adjusted conection end date-time (UTC) from `session report`.
    pub conn_end: Timestamp,
    /// `Conn_Duration` from `session report`: the physical elapsed time of the connection, which is
    /// what makes the DST fold inference possible. See README.md, "Time zone".
    pub conn_duration: Duration,
    /// Active charge time from `session report`.
    /// May differ from `conn_end - conn_start` due to `conn_end` for various reasons, including
    /// ingestion adjustment.
    pub charge_time: Duration,
    /// From `session report`.
    pub energy_use: f64,
    /// `energy_use / charge_time in hours`.
    pub avg_power: f64,
    /// Anomalies associated with this session.
    pub anomalies: Vec<AnomalyKind>,
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
