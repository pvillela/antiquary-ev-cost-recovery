use crate::site_load::{Load, ev_load, transformer_load};
use jiff::{Timestamp, Zoned, tz::TimeZone};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fmt::{self, Debug},
    iter::Sum,
    ops::{Add, Div, Mul},
    rc::Rc,
    time::Duration,
};

/// Time zone the session report's timestamps are stated in. See README.md, "Time zone".
pub const TIME_ZONE_NAME: &str = "America/Toronto";

/// Resolution the session report states session boundaries at: `Conn_DateTime_Start` and
/// `Conn_DateTime_End` are truncated to whole minutes. `Conn_Duration` and `Active_Charge_Time`
/// are *not* — they carry seconds, which is what makes the DST fold inference possible.
///
/// Every allowance the software makes for that truncation is this one value, so all of them move
/// together should Evolute ever report seconds:
///
/// - Added to the reported session end to give `Adj_conn_end`, the session's exclusive end.
/// - The half-width of the band a sound record's `Conn_start + Conn_Duration` must land in.
/// - The width of a *narrow* group, the only width at which a group can be
///   [dubious](crate::SessionGroup::is_dubious).
///
/// Must divide 15 minutes without leaving a remainder, or an end-point clamped into the interval
/// of interest lands off the grid and group durations stop being multiples of this value. That is
/// a requirement on the report format rather than on this software.
///
/// See README.md, "Boundaries and the time grid".
pub const SESSION_BOUNDARY_RESOLUTION: Duration = Duration::from_secs(60);

/// The duration of the interval of interest should be a multiple of this.
pub const SEGMENT_DURATION: Duration = Duration::from_mins(15);

pub(crate) fn time_zone() -> TimeZone {
    TimeZone::get(TIME_ZONE_NAME).expect("America/Toronto should be a valid time-zone name")
}

pub(crate) fn duration(start: Timestamp, end: Timestamp) -> Duration {
    Duration::try_from(end.duration_since(start))
        .unwrap_or_else(|_| panic!("interval ends at {} before it starts at {}", end, start))
}

// ---------------------------------------------------------------------------
// Interval
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd)]
/// Time interval. Must be on the time grid defined by [`SESSION_BOUNDARY_RESOLUTION`].
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
    /// `Conn_start_UTC`: connection start date-time from `session report`, truncated to the
    /// minute like every reported time, so the true start lies in
    /// `[conn_start, conn_start + SESSION_BOUNDARY_RESOLUTION)`.
    pub conn_start: Timestamp,
    /// `Conn_end_UTC`: connection end date-time as reported, truncated to the minute.
    ///
    /// Held for reporting only. Every calculation wants [`Session::adj_conn_end`], which is the
    /// bound that actually contains the session.
    pub conn_end: Timestamp,
    /// `Adj_conn_end_UTC`: [`Session::conn_end`] padded by one [`SESSION_BOUNDARY_RESOLUTION`],
    /// which makes it the session's **exclusive** end — the true end lies in
    /// `[adj_conn_end - SESSION_BOUNDARY_RESOLUTION, adj_conn_end)`.
    ///
    /// This is the end the grouping and estimating logic uses throughout, so that
    /// `[conn_start, adj_conn_end)` is the tightest half-open span guaranteed to contain the real
    /// connection. See README.md, "Session boundaries".
    pub adj_conn_end: Timestamp,
    /// `Conn_Duration` from `session report`: the physical elapsed time of the connection, which is
    /// what makes the DST fold inference possible. See README.md, "Time zone".
    pub conn_duration: Duration,
    /// Active charge time from `session report`.
    ///
    /// May differ from `adj_conn_end - conn_start` for several reasons: the padding on
    /// `adj_conn_end`, and a car that stays connected without drawing power.
    pub charge_time: Duration,
    /// From `session report`.
    pub energy_use: f64,
    /// `energy_use / charge_time in hours`.
    pub avg_kw: f64,
    /// Anomalies associated with this session.
    pub anomalies: Vec<AnomalyKind>,
}

impl Session {
    /// Whether the session overlaps with an interval.
    pub(crate) fn intersects(&self, interval: &Interval) -> bool {
        let sess_itvl = Interval::from_start_end(self.conn_start, self.adj_conn_end);
        let overlap = sess_itvl.intersection(&interval);
        overlap.is_empty()
    }

    /// Reported connection start in local time (ET).
    pub fn conn_start_local(&self) -> Zoned {
        Zoned::new(self.conn_start, time_zone())
    }

    /// Reported connection end in local time (ET).
    pub fn conn_end_local(&self) -> Zoned {
        Zoned::new(self.conn_end, time_zone())
    }

    /// Adjusted, exclusive connection end in local time (ET).
    pub fn adj_conn_end_local(&self) -> Zoned {
        Zoned::new(self.adj_conn_end, time_zone())
    }

    /// Session duration from `conn_start` to `adj_conn_end`
    pub fn adj_duration(&self) -> Duration {
        duration(self.conn_start, self.adj_conn_end)
    }

    /// Average power draw in kW: [`Self::energy_use`] / ([`Self::charge_time`] in hours).
    pub fn avg_kw(&self) -> f64 {
        self.energy_use / self.charge_time.as_secs_f64() * 3600.0
    }

    /// The session's overlap with an interval.
    pub(crate) fn interval_overlap(&self, interval: &Interval) -> SessionOverlap {
        let sess_itvl = Interval::from_start_end(self.conn_start, self.adj_conn_end);
        let overlap = sess_itvl.intersection(&interval);
        if overlap.is_empty() {
            return SessionOverlap::empty();
        }

        let left = if self.conn_start == overlap.start {
            Bracket::new(overlap.start, overlap.start + SESSION_BOUNDARY_RESOLUTION)
        } else {
            Bracket::exact(overlap.start)
        };

        let right = if self.adj_conn_end == overlap.end() {
            Bracket::new(overlap.end() - SESSION_BOUNDARY_RESOLUTION, overlap.end())
        } else {
            Bracket::exact(overlap.end())
        };

        SessionOverlap { left, right }
    }

    /// The duration of the session's overlap with `interval` divided by `interval`'s
    /// duration.
    pub(crate) fn interval_overlap_ratio(&self, interval: &Interval) -> Bracket<f64> {
        let overlap = self.interval_overlap(&interval);
        let overlap_dur = overlap.duration();
        overlap_dur.map(|v| v.as_secs_f64() / interval.duration.as_secs_f64())
    }

    /// Average power (in kW) of this session over `interval`.
    pub(crate) fn interval_avg_kw(&self, interval: &Interval) -> Bracket<f64> {
        let overlap_ratio = self.interval_overlap_ratio(interval);
        overlap_ratio.map(|v| v * self.avg_kw)
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

/// A [`Session`]'s overlap with an [`Interval`], including quantification of
/// overlap uncertainty due to [`SESSION_BOUNDARY_RESOLUTION`].
pub(crate) struct SessionOverlap {
    left: Bracket<Timestamp>,
    right: Bracket<Timestamp>,
}

impl SessionOverlap {
    pub fn new(left: Bracket<Timestamp>, right: Bracket<Timestamp>) -> Self {
        Self { left, right }
    }

    pub fn empty() -> Self {
        Self {
            left: Bracket::new(Timestamp::MAX, Timestamp::MAX),
            right: Bracket::new(Timestamp::MIN, Timestamp::MIN),
        }
    }

    pub fn duration(&self) -> Bracket<Duration> {
        let min = if self.left.max < self.right.min {
            duration(self.left.max, self.right.min)
        } else {
            Duration::ZERO
        };
        let max = duration(self.left.min, self.right.max);
        Bracket::new(min, max)
    }
}

// ---------------------------------------------------------------------------
// Bracket
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
/// Value subject to uncertainty due to [`SESSION_BOUNDARY_RESOLUTION`].
pub struct Bracket<T: Clone> {
    /// Minimum value.
    pub min: T,
    /// Maximum value.
    pub max: T,
}

impl<T: Clone> Bracket<T> {
    /// Instantiate `Self``.
    pub fn new(min: T, max: T) -> Self
    where
        T: Debug + PartialOrd,
    {
        assert!(min <= max, "min={min:?} must be <= max={max:?}");
        Self { min, max }
    }

    /// Instantiates an exact instance.
    pub fn exact(value: T) -> Self {
        Self {
            min: value.clone(),
            max: value,
        }
    }

    pub fn map<U: Clone>(&self, mut f: impl FnMut(&T) -> U) -> Bracket<U> {
        let min = f(&self.min);
        let max = f(&self.max);
        Bracket { min, max }
    }
}

impl<T: Clone + Default> Default for Bracket<T> {
    fn default() -> Self {
        Self {
            min: Default::default(),
            max: Default::default(),
        }
    }
}

impl<T: Clone + Add<Output = T>> Add for Bracket<T> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            min: self.min + rhs.min,
            max: self.max + rhs.max,
        }
    }
}

impl<T: Clone + Mul<f64, Output = T>> Mul<f64> for Bracket<T> {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self {
            min: self.min * rhs,
            max: self.max * rhs,
        }
    }
}

impl Bracket<f64> {
    pub fn mid(&self) -> f64 {
        (self.min + self.max) / 2.0
    }
}

impl Div<f64> for Bracket<f64> {
    type Output = Self;

    fn div(self, rhs: f64) -> Self::Output {
        Self {
            min: self.min / rhs,
            max: self.max / rhs,
        }
    }
}

impl Sum for Bracket<f64> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        let mut sum = Bracket::default();
        for item in iter {
            sum = sum + item;
        }
        sum
    }
}

impl Mul<u32> for Bracket<Duration> {
    type Output = Self;

    fn mul(self, rhs: u32) -> Self::Output {
        Self {
            min: self.min * rhs,
            max: self.max * rhs,
        }
    }
}

impl Sum for Bracket<Duration> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        let mut sum = Bracket::default();
        for item in iter {
            sum = sum + item;
        }
        sum
    }
}

impl Add for Load {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Load {
            real_kw: self.real_kw + rhs.real_kw,
            reactive_kvar: self.reactive_kvar + rhs.reactive_kvar,
            distortion_kvar: self.distortion_kvar + rhs.distortion_kvar,
        }
    }
}

// ---------------------------------------------------------------------------
// Segment
// ---------------------------------------------------------------------------

pub type RSegment = Rc<Segment>;

#[derive(Debug, Clone)]
/// A sub-interval of the interval-of-interest over which power estimates are computed.
pub struct Segment {
    pub interval: Interval,
    pub sessions: BTreeSet<RSession>,
}

impl Segment {
    pub(crate) fn new(start: Timestamp, duration: Duration) -> Self {
        Self {
            interval: Interval::new(start, duration),
            sessions: Default::default(),
        }
    }

    pub fn start(&self) -> Timestamp {
        self.interval.start
    }

    pub fn end(&self) -> Timestamp {
        self.interval.end()
    }

    pub fn agg_count(&self) -> Bracket<f64> {
        self.sessions
            .iter()
            .map(|s| s.borrow().interval_overlap_ratio(&self.interval))
            .sum()
    }

    pub fn agg_kw(&self) -> Bracket<f64> {
        self.sessions
            .iter()
            .map(|s| s.borrow().interval_avg_kw(&self.interval))
            .sum()
    }

    pub fn count_based_load(&self) -> Bracket<Load> {
        let secondary = self.agg_count().map(|v| ev_load().scaled(*v));
        secondary + secondary.map(|v| transformer_load(*v))
    }

    pub fn energy_based_load(&self) -> Bracket<Load> {
        let single_ev_real_kw = ev_load().real_kw;
        let scaling = self.agg_kw().map(|v| v / single_ev_real_kw);
        let secondary = scaling.map(|v| ev_load().scaled(*v));

        // Below 2 lines correspond to `secondary + transforer_load(secondary)` in the implementation
        // of `site_load::site_load`.`
        let xfmr_load = secondary.map(|v| transformer_load(*v));
        secondary + xfmr_load
    }

    pub(crate) fn add_session(&mut self, session: RSession) {
        self.sessions.insert(session);
    }
}

// ---------------------------------------------------------------------------
// Anomalies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyKind {
    /// `Active_Charge_Time` is zero so its `Avg_power` cell shows `#DIV/0!`.
    ZeroActiveChargeTime,
    /// `Conn_start + Conn_Duration` misses the reported `Conn_DateTime_End` by a full
    /// [`SESSION_BOUNDARY_RESOLUTION`] or more, in one direction or the other, so the reported
    /// start, end and duration are mutually inconsistent.
    ///
    /// The tolerance is what makes this a real test rather than a formality, and it is not chosen
    /// — it is forced. Truncation puts the true start somewhere in `[Conn_start, Adj_conn_start)`
    /// and the true end somewhere in `[Conn_end, Adj_conn_end)`: two half-open windows one
    /// [`SESSION_BOUNDARY_RESOLUTION`] wide, the same convention the software uses everywhere
    /// else. An honest `Conn_Duration` spans some instant of the first to some instant of the
    /// second, so the record is sound exactly when the first window, shifted by `Conn_Duration`,
    /// still meets the second:
    ///
    /// ```text
    /// sound  <=>  Conn_start + Conn_Duration  <  Adj_conn_end
    ///        and  Conn_start + Conn_Duration  >  Conn_end - SESSION_BOUNDARY_RESOLUTION
    /// ```
    ///
    /// The band is open at both ends, unlike every other interval here, because both windows are
    /// half-open at the same end — it is an instance of the convention, not an exception to it.
    /// `Adj_conn_end` is the upper bound, now as the exclusive one.
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
    /// to but never reaching one [`SESSION_BOUNDARY_RESOLUTION`], in either direction. Missing it
    /// is therefore normal; missing it by *a minute
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
    /// The session's average power exceeds [`EVOLUTE_BREAKER_KW_RATING`], which the hardware is
    /// supposed to make impossible.
    ///
    /// Informational only: the session still takes part in every estimate, since nothing about the
    /// figure says *which* of `Energy_Use` and `Active_Charge_Time` is wrong, or whether either is.
    /// [`AnomalyKind::InconsistentDuration`] remains the only kind that excludes a session.
    ///
    /// It matters because two things quietly assume it cannot happen. The breaker-spec figures are
    /// a session count times a single rating, so a session drawing more than that rating breaks the
    /// assumption they rest on — see README.md, "Assumptions". And the report states its bracket as
    /// running from the consumption-based figure up to the breaker-spec one, which inverts if a
    /// group's aggregate average power exceeds its member count times the rating.
    ///
    /// The comparison is against the rating exactly, with no tolerance, which is what makes this
    /// flag a complete account of that inversion: it takes a member above the rating to push a
    /// group's aggregate past its member count times the rating, and every such member is flagged.
    /// A tolerance would leave a band of sessions that invert the bracket silently.
    ///
    /// One consequence of exactness: a session meant to sit exactly at the rating may or may not be
    /// flagged, according to how its `Energy_Use / Active_Charge_Time` rounds in binary floating
    /// point. That is the price of the guarantee above, and it errs towards reporting.
    ExcessiveAvgPower,
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
            Self::ExcessiveAvgPower => "ExcessiveAvgPower",
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
            "ExcessiveAvgPower" => Self::ExcessiveAvgPower,
            _ => return None,
        })
    }
}

/// A single row that needs review. Never fatal: the conversion still writes the row, and the
/// estimating logic still produces a figure. Used by both sides — see
/// [`crate::ConversionReport`] and [`crate::PowerEstimatesReport`].
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
            Self::ZeroActiveChargeTime => {
                "zero Active_Charge_Time, so the session delivered its energy in no time at all \
                 and has no finite average power; the estimating logic substitutes one, and the \
                 session is worth reviewing individually"
            }
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
            Self::ExcessiveAvgPower => {
                "average power above the Evolute breaker rating, which the hardware should not \
                 allow; the session still counts towards every estimate, but the breaker-spec \
                 figures assume no session draws more than that rating"
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
