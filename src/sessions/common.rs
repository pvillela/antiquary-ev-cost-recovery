use crate::time::{Interval, TIME_GRID_STEP, duration, time_zone, truncate_to_time_grid};

use super::site_load::{Load, ev_load, ev_real_power_kw, transformer_load};
use jiff::{Timestamp, Zoned};
use std::{
    collections::BTreeSet,
    fmt::{self, Debug},
    iter::Sum,
    ops::{Add, Div, Mul},
    rc::Rc,
    time::Duration,
};

/// The width of the [`Segment`]s an interval of interest is partitioned into.
///
/// The duration of the interval of interest **must** be a positive multiple of this, and
/// [`crate::interval_estimates`] panics otherwise. Not a convention: rounding the segment count up
/// would tile past the interval's end and count sessions falling outside it, and rounding down
/// would leave part of it unestimated. Neither error would show in any figure the report prints,
/// which is why the check is an assertion rather than an accommodation.
///
/// The two legal interval lengths — 15 minutes and 1 hour — are both multiples, so nothing coming
/// through [`crate::checked_interval`] can trip it.
pub const SEGMENT_DURATION: Duration = Duration::from_mins(15);

/// Continuous use breaker kW rating.
pub const BREAKER_RATING_KW: f64 = ev_real_power_kw();

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

pub(crate) type RSession = Rc<Session>;

#[derive(Debug)]
/// Charging session
pub struct Session {
    /// From `session report`.
    pub id: String,
    /// Row number in the Excel workbook. The header occupies row 1, so the lowest possible value
    /// is 2. This is *not* the CSV row: a record duplicated to resolve a DST fold occupies two
    /// workbook rows, so the two diverge from that point on.
    pub row: usize,
    /// `conn_start_utc`: connection start date-time from `session report`.
    pub conn_start: Timestamp,
    /// `conn_end_utc`: connection end date-time as reported, truncated to the minute.
    ///
    /// Held for reporting only. Every calculation wants [`Session::adj_conn_end`], which is the
    /// bound that actually contains the session.
    pub conn_end: Timestamp,
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
    /// Anomalies associated with this session.
    pub anomalies: Vec<AnomalyKind>,
}

impl Session {
    /// `adj_conn_start_utc`: connection start date-time from `session report`, rounded down to
    /// [`TIME_GRID_STEP`], so the true start lies in
    /// `[adj_conn_start, adj_conn_start + TIME_GRID_STEP)`.
    pub fn adj_conn_start(&self) -> Timestamp {
        truncate_to_time_grid(self.conn_start)
    }

    /// `adj_conn_end_utc`: [`Session::conn_end`] padded by one [`TIME_GRID_STEP`],
    /// which makes it the session's **exclusive** end — the true end lies in
    /// `[adj_conn_end - TIME_GRID_STEP, adj_conn_end)`.
    ///
    /// This is the end the estimating logic uses throughout, so that
    /// `[conn_start, adj_conn_end)` is the tightest half-open span guaranteed to contain the real
    /// connection. See README.md, "Sessions and segments".
    pub fn adj_conn_end(&self) -> Timestamp {
        truncate_to_time_grid(self.conn_end) + TIME_GRID_STEP
    }

    /// Whether the session overlaps with an interval.
    ///
    /// # Panics
    ///
    /// If `adj_conn_end` precedes `conn_start`. That is a precondition, not a defensive check: a
    /// session whose span is inverted has fields that contradict each other, is flagged
    /// [`AnomalyKind::InconsistentDuration`] on conversion, and is sorted into
    /// [`crate::SessionReport::excluded`] — so it never reaches the estimating logic at all. The
    /// implication is not incidental: `conn_duration` is unsigned, so the soundness test's
    /// `conn_start + conn_duration < adj_conn_end` cannot hold unless `conn_start < adj_conn_end`.
    ///
    /// Panicking here is therefore the honest behaviour. Reaching it means an excluded session got
    /// somewhere it should not have, and that is worth a crash rather than a plausible answer. The
    /// one caller that legitimately holds excluded sessions — the report, which lists them on
    /// purpose — asks [`Self::lenient_intersects`] instead.
    pub(crate) fn intersects(&self, interval: &Interval) -> bool {
        let sess_itvl = Interval::from_start_end(self.adj_conn_start(), self.adj_conn_end());
        !sess_itvl.intersection(interval).is_empty()
    }

    /// [`Self::intersects`], but answering for a session whose span is inverted rather than
    /// panicking on it.
    ///
    /// For the reporting module alone, and for one question: whether an *excluded* session appears
    /// to fall in the interval of interest. That listing covers the whole workbook by design —
    /// filtering it would apply a judgement to exactly the timestamps that are in doubt — so the
    /// report has to answer for records the estimating logic never touches, including one whose
    /// reported end precedes its start.
    ///
    /// The answer is only ever "appears to", and README says so where the column is described. The
    /// two endpoints are read in whichever order puts them the right way round, which is the most
    /// that can be said for a record whose own fields disagree.
    ///
    /// Identical to [`Self::intersects`] for every session that is not inverted.
    pub(crate) fn lenient_intersects(&self, interval: &Interval) -> bool {
        let (lo, hi) = match self.adj_conn_start() <= self.adj_conn_end() {
            true => (self.adj_conn_start(), self.adj_conn_end()),
            false => (self.adj_conn_end(), self.adj_conn_start()),
        };
        !Interval::from_start_end(lo, hi)
            .intersection(interval)
            .is_empty()
    }

    /// Reported connection start in local time (ET).
    pub fn conn_start_local(&self) -> Zoned {
        Zoned::new(self.conn_start, time_zone())
    }

    /// Reported connection end in local time (ET).
    pub fn conn_end_local(&self) -> Zoned {
        Zoned::new(self.conn_end, time_zone())
    }

    /// Adjusted, inclusive connection start in local time (ET).
    pub fn adj_conn_start_local(&self) -> Zoned {
        Zoned::new(self.adj_conn_start(), time_zone())
    }

    /// Adjusted, exclusive connection end in local time (ET).
    pub fn adj_conn_end_local(&self) -> Zoned {
        Zoned::new(self.adj_conn_end(), time_zone())
    }

    /// Session duration from `adj_conn_start` to `adj_conn_end`
    pub fn adj_duration(&self) -> Duration {
        duration(self.adj_conn_start(), self.adj_conn_end())
    }

    /// Average power draw in kW: [`Self::energy_use`] / ([`Self::charge_time`] in hours).
    pub fn avg_kw(&self) -> f64 {
        let kw = self.energy_use / self.charge_time.as_secs_f64() * 3600.0;
        match kw.is_finite() {
            true => kw,
            false => {
                if self.energy_use == 0.0 {
                    0.0
                } else {
                    BREAKER_RATING_KW
                }
            }
        }
    }

    /// The session's overlap with an interval, or `None` when the two do not meet.
    ///
    /// `None` rather than a zero-width [`SessionOverlap`]: there is no pair of brackets that
    /// stands for "no overlap" without also standing for some instant, and a sentinel pair built
    /// from the extremes of the timestamp range only defers the problem to whoever measures its
    /// duration. The absence is in the type instead.
    pub(crate) fn interval_overlap(&self, interval: &Interval) -> Option<SessionOverlap> {
        let sess_itvl = Interval::from_start_end(self.adj_conn_start(), self.adj_conn_end());
        let overlap = sess_itvl.intersection(interval);
        if overlap.is_empty() {
            return None;
        }

        let left = if self.adj_conn_start() == overlap.start {
            Bracket::new(overlap.start, overlap.start + TIME_GRID_STEP)
        } else {
            Bracket::exact(overlap.start)
        };

        let right = if self.adj_conn_end() == overlap.end() {
            Bracket::new(overlap.end() - TIME_GRID_STEP, overlap.end())
        } else {
            Bracket::exact(overlap.end())
        };

        Some(SessionOverlap { left, right })
    }

    /// The duration of the session's overlap with `interval` divided by `interval`'s
    /// duration.
    pub(crate) fn interval_overlap_ratio(&self, interval: &Interval) -> Bracket<f64> {
        match self.interval_overlap(interval) {
            None => Bracket::exact(0.0),
            Some(overlap) => overlap
                .duration()
                .map(|v| v.as_secs_f64() / interval.duration.as_secs_f64()),
        }
    }

    /// Average power (in kW) of this session over `interval`.
    pub(crate) fn interval_avg_kw(&self, interval: &Interval) -> Bracket<f64> {
        let overlap_ratio = self.interval_overlap_ratio(interval);
        overlap_ratio.map(|v| v * self.avg_kw())
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
        Some(self.cmp(other))
    }
}

impl Ord for Session {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

/// A [`Session`]'s overlap with an [`Interval`], including quantification of
/// overlap uncertainty due to [`TIME_GRID_STEP`].
pub(crate) struct SessionOverlap {
    left: Bracket<Timestamp>,
    right: Bracket<Timestamp>,
}

impl SessionOverlap {
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
/// Value subject to uncertainty due to [`TIME_GRID_STEP`].
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

/// A shared [`Segment`].
///
/// [`crate::IntervalEstimates`] names the same segment three times over — once in the full listing,
/// and again as the maximum of each derivation — so sharing rather than copying is what keeps those
/// references *the same segment* rather than three equal ones. A reader can then ask whether the
/// two derivations peaked together with [`std::rc::Rc::ptr_eq`], instead of comparing clock times
/// and hoping.
///
/// The saved copying is a secondary benefit and a real one: a `Segment` carries a
/// `BTreeSet` of its sessions, and cloning that duplicates the tree.
pub type RSegment = Rc<Segment>;

#[derive(Debug, Clone, PartialEq)]
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
            .map(|s| s.interval_overlap_ratio(&self.interval))
            .sum()
    }

    pub fn agg_kw(&self) -> Bracket<f64> {
        self.sessions
            .iter()
            .map(|s| s.interval_avg_kw(&self.interval))
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
    /// `Active_Charge_Time` is zero so its `avg_kw` cell shows `#DIV/0!`.
    ZeroActiveChargeTime,
    /// `Conn_start + Conn_Duration` misses the reported `Conn_DateTime_End` by a full
    /// [`TIME_GRID_STEP`] or more, in one direction or the other, so the reported
    /// start, end and duration are mutually inconsistent.
    ///
    /// The tolerance is what makes this a real test rather than a formality, and it is not chosen
    /// — it is forced. Truncation puts the true start somewhere in `[Conn_start, Adj_conn_start)`
    /// and the true end somewhere in `[Conn_end, adj_conn_end)`: two half-open windows one
    /// [`TIME_GRID_STEP`] wide, the same convention the software uses everywhere
    /// else. An honest `Conn_Duration` spans some instant of the first to some instant of the
    /// second, so the record is sound exactly when the first window, shifted by `Conn_Duration`,
    /// still meets the second:
    ///
    /// ```text
    /// sound  <=>  Conn_start + Conn_Duration  <  adj_conn_end
    ///        and  Conn_start + Conn_Duration  >  Conn_end - TIME_GRID_STEP
    /// ```
    ///
    /// The band is open at both ends, unlike every other interval here, because both windows are
    /// half-open at the same end — it is an instance of the convention, not an exception to it.
    /// `adj_conn_end` is the upper bound, now as the exclusive one.
    ///
    /// Both directions are faults, and both exclude the session from the estimates: if a record's
    /// own fields disagree by more than the reporting can explain, neither its duration nor the
    /// span the estimating logic would place it on can be relied on. The overshoot direction also
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
    /// to but never reaching one [`TIME_GRID_STEP`], in either direction. Missing it
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
    /// The session's average power exceeds [`BREAKER_RATING_KW`], which the hardware is
    /// supposed to make impossible.
    ///
    /// Informational only: the session still takes part in every estimate, since nothing about the
    /// figure says *which* of `Energy_Use` and `Active_Charge_Time` is wrong, or whether either is.
    /// [`AnomalyKind::InconsistentDuration`] remains the only kind that excludes a session.
    ///
    /// It matters because the count-based figures are an aggregate session count times a single
    /// rating, so a session drawing more than that rating breaks the assumption they rest on — see
    /// README.md, "Assumptions". A reader ordinarily finds the energy-based figures at or below the
    /// count-based ones, and that ordering inverts exactly when a segment's `agg_kw` exceeds its
    /// `agg_count` times the rating.
    ///
    /// The comparison is against the rating exactly, with no tolerance, which is what makes this
    /// flag a complete account of that inversion: it takes a member above the rating to push a
    /// segment's `agg_kw` past its `agg_count` times the rating, and every such member is flagged.
    /// A tolerance would leave a band of sessions that invert the two silently.
    ///
    /// One consequence of exactness: a session meant to sit exactly at the rating may or may not be
    /// flagged, according to how its `Energy_Use / Active_Charge_Time` rounds in binary floating
    /// point. That is the price of the guarantee above, and it errs towards reporting.
    ExcessiveAvgKw,
}

impl AnomalyKind {
    /// The variant name, as written to the workbook's `anomalies` column. Deliberately distinct
    /// from [`fmt::Display`], which is free-form prose for humans and may be reworded at will;
    /// this is a wire format and must stay stable.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ZeroActiveChargeTime => "ZeroActiveChargeTime",
            Self::InconsistentDuration => "InconsistentDuration",
            Self::DstAmbiguousDuplicated => "DstAmbiguousDuplicated",
            Self::DstGapShifted => "DstGapShifted",
            Self::DstUnresolvable => "DstUnresolvable",
            Self::ExcessiveAvgKw => "ExcessiveAvgKw",
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
            "ExcessiveAvgKw" => Self::ExcessiveAvgKw,
            _ => return None,
        })
    }
}

/// A single row that needs review. Never fatal: the conversion still writes the row, and the
/// estimating logic still produces a figure. Used by both sides — see
/// [`crate::ConversionReport`] and [`crate::IntervalEstimates`].
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
            Self::ExcessiveAvgKw => {
                "average kilowatts above the Evolute breaker rating, which the hardware should not \
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
