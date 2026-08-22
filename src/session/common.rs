use crate::time::{Interval, duration, time_zone, truncate_to};

use super::site_load::{Load, ev_load, ev_real_power_kw, transformer_load};
use jiff::{Timestamp, Zoned};
use std::{
    collections::BTreeMap,
    fmt::{self, Debug},
    iter::Sum,
    ops::{Add, Div, Mul},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

/// Resolution this software works session boundaries to.
///
/// **Ours, not Evolute's.** Evolute currently reports `Conn_DateTime_Start` and `Conn_DateTime_End`
/// truncated to whole minutes, and this is set to match; but the two are different quantities, and
/// the document that derives everything built on this calls them `EV_STEP` and `OUR_STEP` for that
/// reason. See `docs/session/time-reporting-uncertainty.md`.
///
/// The distinction decides what to do if Evolute ever reports seconds. **Do not follow them down
/// to 1 second while reports of both resolutions are still processed together.** This constant is
/// global; `EV_STEP` belongs to a report. A calculation spanning a minute-resolution report and a
/// second-resolution one has no single right value for it, so it has to sit at the coarsest
/// resolution still in scope. Finer reporting is a reason to narrow the allowances below, not to
/// move the grid.
///
/// The constraint lifts once every report in scope reports seconds. 1 second is a legal grid — it
/// divides [`SEGMENT_DURATION`], and [`LEGAL_START_MINUTES`](super::LEGAL_START_MINUTES) still
/// lands on it — so from then on the change is available, though it moves every figure in the
/// golden files.
///
/// Every allowance the software makes for the reporting's truncation is this one value:
///
/// - Added to the reported session end to give `adj_conn_end`, the session's exclusive end.
/// - The width of the window a sound record's `Conn_start + Conn_Duration` must land in — one step
///   early, one step and a second late. See `duration_is_consistent`.
///
/// `Conn_Duration` and `Active_Charge_Time` are *not* truncated; they carry seconds. That asymmetry
/// is what makes the DST fold inference possible, and it is why the window above has a width at all.
///
/// See docs/session/README.md, "Boundaries and the time grid".
pub const TIME_GRID_STEP: Duration = Duration::from_secs(60);

/// The width of the [`Segment`]s an interval of interest is partitioned into.
///
/// The duration of the interval of interest **must** be a positive multiple of this, and
/// [`crate::session::interval_estimates`] panics otherwise. Not a convention: rounding the segment count up
/// would tile past the interval's end and count sessions falling outside it, and rounding down
/// would leave part of it unestimated. Neither error would show in any figure the report prints,
/// which is why the check is an assertion rather than an accommodation.
///
/// The two legal interval lengths — 15 minutes and 1 hour — are both multiples, so nothing coming
/// through [`crate::session::checked_interval`] can trip it.
pub const SEGMENT_DURATION: Duration = Duration::from_mins(15);

/// Continuous use breaker kW rating.
pub const BREAKER_RATING_KW: f64 = ev_real_power_kw();

// ---------------------------------------------------------------------------
// Reported time, adjusted
// ---------------------------------------------------------------------------
//
// The three functions below are the code counterpart of
// `docs/session/time-reporting-uncertainty.md`, which derives all of them. They are free
// functions rather than methods so the write path, which has a CSV record and not yet a
// [`Session`], calls the same code the read path does. Two definitions of `adj_conn_end` is how
// the two drifted apart last time.

/// `adj_start` of the document: the reported start truncated to the time grid, so the true start
/// lies in `[adj_conn_start, adj_conn_start + TIME_GRID_STEP)`.
pub(crate) fn adj_conn_start_of(conn_start: Timestamp) -> Timestamp {
    truncate_to(conn_start, TIME_GRID_STEP)
}

/// `adj_end` of the document: `our_truncate(rep_end + 1s) + OUR_STEP`.
///
/// The `+ 1s` is not padding. The reported end is truncated, *and* it is not known whether the
/// reporting includes or excludes its last second, so the true end may lie a second beyond the
/// minute the report names. Dropping it makes the bound too tight by up to one whole step for any
/// `conn_end` carrying seconds; they agree only while every reported end lands on the minute.
pub(crate) fn adj_conn_end_of(conn_end: Timestamp) -> Timestamp {
    truncate_to(conn_end + Duration::from_secs(1), TIME_GRID_STEP) + TIME_GRID_STEP
}

/// Whether a record's reported start, end and duration can all be true at once.
///
/// Three checks, and any failure raises [`AnomalyKind::InconsistentDuration`]:
///
/// ```text
/// 1.  rep_start <= rep_end
/// 2.  rep_start + conn_duration  <  rep_end + TIME_GRID_STEP + 1s
/// 3.  rep_end - TIME_GRID_STEP   <  rep_start + conn_duration
/// ```
///
/// Checks 2 and 3 are the document's consistency checks 1 and 2, the second rearranged. Neither is
/// chosen: they are what truncation to `TIME_GRID_STEP` accounts for and nothing more, so widening
/// either lets a real fault through and narrowing either flags a sound record.
///
/// Check 1 is explicit because the document's own check 3, `adj_start <= adj_end`, is too weak to
/// stand in for it: with `rep_start = 10:01:00` and `rep_end = 10:00:00` both sides truncate to
/// `10:01:00`, so a one-minute inversion passes. It only bites beyond roughly two steps. That
/// matters because [`Session::intersects`] panics on an inverted span and documents exclusion by
/// this very test as the reason it cannot happen — an inverted record with a small
/// `conn_duration` satisfies both of the other two checks and would reach it.
pub(crate) fn duration_is_consistent(
    conn_start: Timestamp,
    conn_end: Timestamp,
    conn_duration: Duration,
) -> bool {
    let implied_end = conn_start + conn_duration;
    conn_start <= conn_end
        && implied_end < conn_end + TIME_GRID_STEP + Duration::from_secs(1)
        && conn_end - TIME_GRID_STEP < implied_end
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// A shared [`Session`].
///
/// Public because it appears in public signatures — [`Segment::sessions`],
/// [`Sessions::sessions`], and the API's estimating calls — and naming the type a caller has
/// to write is the point of an alias. The sharing is what lets one session belong to several
/// segments, and to an [`Anomaly`], without being copied.
pub type RSession = Rc<Session>;

#[derive(Debug)]
/// Charging session
pub struct Session {
    /// Path to source data file.
    pub path: Rc<PathBuf>,
    /// Row number in the source data file. The header occupies row 1, so the lowest possible value
    /// is 2.
    pub row: usize,
    /// From `session report`.
    pub id: String,
    /// `conn_start_utc`: connection start date-time from `session report`.
    pub conn_start: Timestamp,
    /// `conn_end_utc`: connection end date-time as reported, truncated to the minute.
    ///
    /// Held for reporting only. Every calculation wants [`Session::adj_conn_end`], which is the
    /// bound that actually contains the session.
    pub conn_end: Timestamp,
    /// `Conn_Duration` from `session report`: the physical elapsed time of the connection, which is
    /// what makes the DST fold inference possible. See docs/time/README.md, "Time zone".
    pub conn_duration: Duration,
    /// Active charge time from `session report`.
    ///
    /// Differs from `adj_conn_end - conn_start` by the padding on `adj_conn_end`, and from
    /// `conn_duration` by about a second. It does **not** measure charging as distinct from
    /// connection. Evolute, 22 Jul 2026:
    ///
    /// > All 3 will show as almost the same, with Active charging being off by maybe 1 second due
    /// > to rounding as it is on a slightly different timer. These fields are here for grant
    /// > reporting, but for our system we do not track them differently.
    ///
    /// The reason previously given here — a car that stays connected without drawing power — was
    /// wrong, and the correction matters: it is what makes a zero `Active_Charge_Time` a reporting
    /// fault rather than an idle connection. See `Questions_for_Evolute.md`, "Answers received".
    pub charge_time: Duration,
    /// From `session report`.
    pub energy_use: f64,
    /// Anomalies associated with this session.
    pub anomalies: Vec<AnomalyKind>,
}

impl Session {
    /// `adj_conn_start_utc`: see `adj_conn_start_of`, which this defers to.
    pub fn adj_conn_start(&self) -> Timestamp {
        adj_conn_start_of(self.conn_start)
    }

    /// `adj_conn_end_utc`: see `adj_conn_end_of`, which this defers to.
    ///
    /// This is the end the estimating logic uses throughout, so that
    /// `[adj_conn_start, adj_conn_end)` is the tightest half-open span guaranteed to contain the
    /// real connection. See docs/session/README.md, "Sessions and segments".
    pub fn adj_conn_end(&self) -> Timestamp {
        adj_conn_end_of(self.conn_end)
    }

    /// Whether the session overlaps with an interval.
    ///
    /// # Panics
    ///
    /// If `adj_conn_end` precedes `conn_start`. That is a precondition, not a defensive check: a
    /// session whose span is inverted has fields that contradict each other, is flagged
    /// [`AnomalyKind::InconsistentDuration`] on conversion, and is sorted into
    /// [`Sessions::excluded`] — so it never reaches the estimating logic at all.
    ///
    /// What establishes that is check 1 of [`duration_is_consistent`], `conn_start <= conn_end`,
    /// which is there for this reason. It is not implied by the other two: a one-minute inversion
    /// with a near-zero duration satisfies both of them, and before check 1 existed such a record
    /// reached here and panicked.
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

    /// Whether two records sharing an `id` describe *different* sessions.
    ///
    /// The question [`MergedSessions::merge_sessions`] asks of every pair of records with the same
    /// `Charge_Session_ID`. `false` means the two are one session reported in two overlapping
    /// files, and one copy is dropped. `true` means they are not the same session — either Evolute
    /// reused the id, or the two files disagree about one — and both are kept.
    ///
    /// The fields compared are the ones every estimate is built from: where the session sits on the
    /// timeline, how long it charged, and how much energy it took. `row` and `path` are excluded on
    /// purpose — the same session is at different rows in different files, which is precisely the
    /// case this has to see through.
    pub(crate) fn is_inconsistent_duplicate(&self, other: &Session) -> bool {
        self.id == other.id
            && (self.adj_conn_start() != other.adj_conn_start()
                || self.adj_conn_end() != other.adj_conn_end()
                || self.charge_time != other.charge_time
                || self.energy_use != other.energy_use)
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

// `Session` deliberately has no `PartialEq`, `Ord` or `Hash`. It had them so that [`Segment`] could
// hold a `BTreeSet`, and they compared `id` alone — which made segment membership, and every figure
// drawn from it, rest on `Charge_Session_ID` being unique. It is not: Evolute's June 2026 report
// carries `S37487` on two unrelated sessions a week apart. `Segment` holds a `Vec` for that reason,
// and nothing else needs to ask whether two sessions are equal.

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

/// Several files' sessions as one list, with what is wrong across them.
pub struct MergedSessions {
    /// Every session, in the order the lists were given, less the records collapsed as identical.
    pub sessions: Vec<RSession>,
    /// Anomalies that are not properties of any single record and so are not on
    /// [`Session::anomalies`]. Currently [`AnomalyKind::DuplicateId`] only.
    pub anomalies: Vec<Anomaly>,
}

impl MergedSessions {
    /// Flattens session lists, collapsing records that appear identically in more than one, and
    /// flags every surviving session whose `id` another one shares.
    ///
    /// Two rules, and the distinction between them is the whole of this function:
    ///
    /// - Same `id` and every compared field equal — see [`Session::is_inconsistent_duplicate`] —
    ///   is one session reported in two overlapping files. One copy is kept. Counting both would
    ///   inflate every figure derived from it, which is why a billing period spanning two monthly
    ///   reports cannot simply concatenate them.
    /// - Same `id` with any field differing is *not* one session. `Charge_Session_ID` is not unique
    ///   in Evolute's reports — the June 2026 report carries `S37487` on two sessions a week apart
    ///   — so such records are kept and estimated from, and flagged
    ///   [`AnomalyKind::DuplicateId`] so a reader can see the id was reused. That flag cannot
    ///   distinguish a reused id from two files genuinely disagreeing about one session; both look
    ///   the same from here, and both are worth seeing.
    ///
    /// One list in means there is nothing to collapse across files, and this is detection alone.
    /// That is how a single-file read gets the same flagging: it comes through here too.
    pub(crate) fn merge_sessions(session_lists: Vec<Vec<RSession>>) -> Self {
        let mut sessions: Vec<RSession> = Vec::new();
        for list in session_lists {
            for session in list {
                // Linear against what is already kept. The comparison is on the compared fields,
                // not on the id, so no map keyed by id would serve: an id may legitimately name
                // several distinct sessions, which is the case that produced this function.
                let already_kept = sessions
                    .iter()
                    .any(|kept| kept.id == session.id && !kept.is_inconsistent_duplicate(&session));
                if !already_kept {
                    sessions.push(session);
                }
            }
        }

        let anomalies = duplicate_id_anomalies(&sessions);
        Self {
            sessions,
            anomalies,
        }
    }
}

/// One [`AnomalyKind::DuplicateId`] per session whose `id` another session in the list also
/// carries.
///
/// Symmetric: every member of a colliding group is flagged, not only the later ones. Which record
/// was read first says nothing about which is the odd one out, and a reader looking up either row
/// should find the flag there.
fn duplicate_id_anomalies(sessions: &[RSession]) -> Vec<Anomaly> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for session in sessions {
        *counts.entry(session.id.as_str()).or_default() += 1;
    }

    sessions
        .iter()
        .filter(|s| counts[s.id.as_str()] > 1)
        .map(|s| Anomaly {
            session: s.clone(),
            kind: AnomalyKind::DuplicateId,
        })
        .collect()
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
/// [`IntervalEstimates`](super::IntervalEstimates) names the same segment three times over — once in the full listing,
/// and again as the maximum of each derivation — so sharing rather than copying is what keeps those
/// references *the same segment* rather than three equal ones. A reader can then ask whether the
/// two derivations peaked together with [`std::rc::Rc::ptr_eq`], instead of comparing clock times
/// and hoping.
///
/// The saved copying is a secondary benefit and a real one: a `Segment` carries a list of its
/// sessions, and cloning that duplicates every entry.
pub type RSegment = Rc<Segment>;

// No `PartialEq`: comparing two segments means comparing their sessions, and `Session` has no
// equality for the reasons given above it.
#[derive(Debug, Clone)]
/// A sub-interval of the interval-of-interest over which power estimates are computed.
pub struct Segment {
    pub interval: Interval,
    /// The sessions intersecting this segment, in the order the report states them.
    ///
    /// A `Vec` rather than a set. A set would need [`Session`] to answer whether two sessions are
    /// the same, and the only answer available was `id`, which Evolute reuses — so the set was
    /// quietly making segment membership depend on a uniqueness that does not hold. It was never
    /// doing the work either: `segments_for_ioi` offers each session to each segment once, so
    /// there is nothing to deduplicate.
    pub sessions: Vec<RSession>,
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
        self.sessions.push(session);
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
    /// The test is `duration_is_consistent`, which carries the derivation. Three checks, and any
    /// failure raises this:
    ///
    /// ```text
    /// 1.  rep_start <= rep_end
    /// 2.  rep_start + conn_duration  <  rep_end + TIME_GRID_STEP + 1s
    /// 3.  rep_end - TIME_GRID_STEP   <  rep_start + conn_duration
    /// ```
    ///
    /// The window checks 2 and 3 draw is not chosen — it is forced, being exactly what truncation
    /// to [`TIME_GRID_STEP`] accounts for and nothing more. It is asymmetric: one second wider on
    /// the late side, because the reported end is not only truncated but also of unknown last-
    /// second convention. Every bound is strict.
    ///
    /// Check 1 is not redundant. A record whose end precedes its start by one minute, with a
    /// duration near zero, satisfies both of the others.
    ///
    /// Every direction is a fault, and all of them exclude the session from the estimates: if a
    /// record's own fields disagree by more than the reporting can explain, neither its duration
    /// nor the span the estimating logic would place it on can be relied on.
    ///
    /// See `docs/session/time-reporting-uncertainty.md` and docs/session/README.md, "Other".
    InconsistentDuration,
    /// The start fell in the DST fold and both offsets reproduce the reported end,
    /// so the record was duplicated. See docs/time/README.md, "Time zone".
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
    /// if at all, by [`AnomalyKind::InconsistentDuration`]. See docs/time/README.md, "Time zone".
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
    /// docs/session/README.md, "Assumptions". A reader ordinarily finds the energy-based figures at or below the
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
    /// Another session in the same list carries the same `Charge_Session_ID`.
    ///
    /// `Charge_Session_ID` is not unique in Evolute's reports: the June 2026 report carries
    /// `S37487` on two sessions a week apart. Informational only — every session so flagged takes
    /// part in the estimates exactly as it would otherwise, since two records sharing an id are
    /// two sessions until something says otherwise.
    ///
    /// Raised symmetrically, on every member of a colliding group. It cannot distinguish a reused
    /// id from two overlapping reports disagreeing about one session: both are records sharing an
    /// id and differing in their figures, which is all the merge can see. Both are worth a
    /// reader's eye.
    DuplicateId,

    /// The reported start or end does not land on a whole [`TIME_GRID_STEP`].
    ///
    /// Informational only. Every allowance this software makes for the reporting's truncation
    /// assumes the reported times are truncated to that step. If Evolute starts reporting seconds,
    /// they no longer are, and the allowances become too wide rather than wrong — a session gets a
    /// padded end it does not need, and the consistency window admits records it should reject.
    /// Nothing crashes and no figure looks odd, which is exactly why it needs saying.
    ///
    /// A property of the record's own times, so it travels on [`Session::anomalies`] and survives
    /// the round trip through a workbook honestly: the times it describes are the ones written.
    ///
    /// Expect it on every row or on none. A report that has switched resolution has switched it
    /// throughout, so whatever renders these should say how many rather than list them all.
    OffGridTimes,

    /// A workbook column disagrees with what the [`Session`] methods recompute from it, or does not
    /// hold a value of the right kind at all.
    ///
    /// The sheet is stale or was edited. The recomputed value always wins and no figure changes;
    /// this only says the stored one no longer matches.
    ///
    /// A fact about the workbook rather than about the session: the CSV that workbook was written
    /// from disagrees with nothing, and a second workbook of the same sessions need not disagree
    /// either. So it lives on [`Sessions::anomalies`], where the findings that belong to a file
    /// rather than to a record go.
    ///
    /// Says only that a column disagreed. Which one, what it held and what was recomputed are not
    /// carried: an [`AnomalyKind`] is a bare token, and [`Anomaly`] is a session and a kind.
    WorkbookDiscrepancy,
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
            Self::DuplicateId => "DuplicateId",
            Self::OffGridTimes => "OffGridTimes",
            Self::WorkbookDiscrepancy => "WorkbookDiscrepancy",
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
            "DuplicateId" => Self::DuplicateId,
            "OffGridTimes" => Self::OffGridTimes,
            "WorkbookDiscrepancy" => Self::WorkbookDiscrepancy,
            _ => return None,
        })
    }
}

/// A session that needs review. Never fatal: the conversion still writes the row, and the
/// estimating logic still produces a figure. Used by both sides — see
/// [`crate::session::ConversionReport`] and [`crate::session::IntervalEstimates`].
///
/// Holds the session itself rather than a copy of a field or two off it. Copying `id` and `row` out
/// meant every consumer that wanted anything else — the average power beside the flag, the file the
/// row is in — had to find its way back to the session through a key, and no key available is one
/// Evolute guarantees: ids repeat, and `(path, row)` is shared by the two halves of a DST fold.
/// Holding the `Rc` is what removes that question.
#[derive(Debug, Clone)]
pub struct Anomaly {
    pub session: RSession,
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
                "reported start, end and duration contradict each other by more than truncation \
                 to the minute can explain; the session is excluded from every estimate"
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
            Self::DuplicateId => {
                "another session in the report carries the same Charge_Session_ID; the id is not \
                 unique in Evolute's reports, so both sessions still count towards every estimate"
            }
            Self::OffGridTimes => {
                "the reported start or end does not land on a whole minute, so the report's \
                 resolution has become finer than this software's time grid; nothing is wrong with \
                 the record, but the padding and the consistency window are now wider than the \
                 data needs"
            }
            Self::WorkbookDiscrepancy => {
                "a stored column in the workbook disagrees with what this software recomputes from \
                 the row, so the sheet is stale or was edited; the recomputed value is the one used"
            }
        };
        f.write_str(s)
    }
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "row {} ({}): {}",
            self.session.row, self.session.id, self.kind
        )
    }
}

// ---------------------------------------------------------------------------
// Session reports
// ---------------------------------------------------------------------------

/// Sessions, grouped by how the peak power contribution logic must treat them, and what else is
/// known about them.
///
/// Named for the objects rather than for the context around them, the way
/// [`Readings`](crate::green_button::Readings) is, so the two sources read the same way.
///
/// This is where a finding goes when it is not a property of any one session: a relation between
/// records, or a fact about the file they were read from. What is true of a record itself goes on
/// [`Session::anomalies`] instead, and travels with it — including out to a workbook and back
/// through the `anomalies` column.
///
/// Returned by both readers — [`crate::session::csv::session_list`] from the CSV and
/// [`crate::session::excel::session_list`] from a workbook written from it — because the grouping
/// is a property of the sessions, not of the file they were read out of. The writing direction
/// returns a [`crate::session::ConversionReport`] instead.
///
/// It was `SessionReport` until this crate had three things called a report: the document a
/// [`Display`](std::fmt::Display) writes, the CSV Evolute exports, and this. Only the CSV is still
/// called one.
#[derive(Debug)]
pub struct Sessions {
    /// Sessions with a finite average power. This is what the peak power contribution logic
    /// consumes unaltered. A session with zero `Energy_Use` belongs here — its `avg_kw` is
    /// legitimately zero, and it still occupies a breaker.
    pub sessions: Vec<RSession>,
    /// Sessions with zero `Active_Charge_Time`, so [`Session::charge_time`] is zero and energy
    /// over charge time is infinite or `NaN`. Kept out of `session` because those values would
    /// swamp or poison any segment they entered.
    ///
    /// Surfaced rather than dropped because such a row is almost certainly a **reporting fault**
    /// and someone should see it. That is a correction: the reason given here used to be that
    /// energy delivered in no time at all is what a demand charge bills on, which read the field
    /// as a real measurement of charging. Evolute has since stated that the three duration fields
    /// track the same thing to within about a second and are not measured separately, so a zero
    /// beside a non-zero `Energy_Use` is a contradiction in the report rather than an event. See
    /// `Questions_for_Evolute.md`, "Answers received". [`Session::avg_kw`] substitutes a finite
    /// figure so the row can still be listed. See docs/session/README.md, "Other".
    pub spikes: Vec<RSession>,
    /// Sessions flagged [`AnomalyKind::InconsistentDuration`]: their reported start, end and
    /// duration contradict each other, so they cannot be placed on a timeline at all. Excluded
    /// from the estimates and returned only for review. See docs/session/README.md, "Other".
    pub excluded: Vec<RSession>,
    /// Anomalies that are not properties of any single record, and so are not reachable through
    /// [`Session::anomalies`]. Currently [`AnomalyKind::DuplicateId`] only.
    ///
    /// Separate from the sessions because such an anomaly is a relation between records rather than
    /// a fault in one: an id is a duplicate only relative to another session, and which of the two
    /// is at fault is not a question the data answers.
    pub anomalies: Vec<Anomaly>,
    /// The files these sessions were read from, in the order they were read.
    ///
    /// Not derived from the sessions. Every [`Session`] carries its own `path`, but a file holding
    /// no session — or none that survived — would then leave no trace, and "this file contributed
    /// nothing" is exactly what a reader checking a period against two monthly exports needs to be
    /// able to see.
    pub sources: Vec<PathBuf>,
    /// Where the run logs were written, one per file read. Each always exists, and says either
    /// that nothing was found or what was. Their contents depend on which reader produced this
    /// report — see their docs.
    ///
    /// A vector because a report can be built from several files at once — see
    /// `Sessions::from_session_lists` — and each source file has a log of its own, written
    /// beside it.
    pub log_paths: Vec<PathBuf>,
}

impl Sessions {
    /// Merges the session lists and sorts what survives into the three buckets.
    ///
    /// One call rather than two because every caller wants both and neither step is useful without
    /// the other. Merging decides which records are the same session — see
    /// [`MergedSessions::merge_sessions`], which is also where a shared `Charge_Session_ID` is
    /// noticed — and bucketing decides what each surviving session is fit for. A caller passing one
    /// list has nothing to merge across files and gets the flagging alone, which is how a
    /// single-file read is the same code path as a two-file one.
    ///
    /// Bucketing is kept here, and out of both readers, so that a session read from a CSV and the
    /// same session read back from the workbook written from it cannot land in different buckets.
    /// The tests are applied in this order, strongest first:
    ///
    /// 1. Flagged [`AnomalyKind::InconsistentDuration`] — [`Sessions::excluded`]. Such a
    ///    session takes no part in the estimates whatever its charge time, and letting one through
    ///    would put an inverted session in front of the segmenting logic, whose endpoints would
    ///    then arrive out of order.
    /// 2. Zero `Active_Charge_Time` — [`Sessions::spikes`].
    /// 3. Everything else — [`Sessions::sessions`].
    ///
    /// Order within each bucket is the order given, which for both readers is report order.
    pub(crate) fn from_session_lists(
        session_lists: Vec<Vec<RSession>>,
        sources: Vec<PathBuf>,
        log_paths: Vec<PathBuf>,
    ) -> Self {
        let MergedSessions {
            sessions,
            anomalies,
        } = MergedSessions::merge_sessions(session_lists);
        let mut report = Self {
            sessions: Vec::new(),
            spikes: Vec::new(),
            excluded: Vec::new(),
            anomalies,
            sources,
            log_paths,
        };
        for session in sessions {
            if session
                .anomalies
                .contains(&AnomalyKind::InconsistentDuration)
            {
                report.excluded.push(session);
            } else if session.charge_time.is_zero() {
                report.spikes.push(session);
            } else {
                report.sessions.push(session);
            }
        }
        report
    }
}

// cargo test --lib -- session::common::test
#[cfg(test)]
mod test {
    use super::*;

    fn session(path: &str, row: usize, id: &str, start: &str, energy_use: f64) -> RSession {
        let conn_start: Timestamp = start.parse().expect("an RFC 3339 timestamp");
        let conn_duration = Duration::from_secs(3600);
        Rc::new(Session {
            path: Rc::new(PathBuf::from(path)),
            row,
            id: id.to_owned(),
            conn_start,
            conn_end: conn_start + conn_duration,
            conn_duration,
            charge_time: conn_duration,
            energy_use,
            anomalies: Vec::new(),
        })
    }

    fn ids(sessions: &[RSession]) -> Vec<&str> {
        sessions.iter().map(|s| s.id.as_str()).collect()
    }

    fn flagged(anomalies: &[Anomaly]) -> Vec<(&str, usize)> {
        anomalies
            .iter()
            .map(|a| {
                assert_eq!(a.kind, AnomalyKind::DuplicateId);
                (a.session.id.as_str(), a.session.row)
            })
            .collect()
    }

    /// The case merging exists for: two monthly reports overlap, and the session in both is one
    /// session. Counting it twice would inflate every figure drawn from it.
    #[test]
    fn a_record_present_identically_in_two_files_is_kept_once() {
        let may = vec![
            session("May.csv", 2, "S1", "2026-05-30T12:00:00Z", 4.0),
            session("May.csv", 3, "S2", "2026-05-31T12:00:00Z", 5.0),
        ];
        // The same two sessions as the next report states them, at its own row numbers.
        let june = vec![
            session("June.csv", 2, "S1", "2026-05-30T12:00:00Z", 4.0),
            session("June.csv", 3, "S2", "2026-05-31T12:00:00Z", 5.0),
            session("June.csv", 4, "S3", "2026-06-01T12:00:00Z", 6.0),
        ];

        let merged = MergedSessions::merge_sessions(vec![may, june]);
        assert_eq!(ids(&merged.sessions), ["S1", "S2", "S3"]);
        // The first file's copy is the one kept, so the rows are its.
        assert_eq!(merged.sessions[0].row, 2);
        assert_eq!(*merged.sessions[0].path, PathBuf::from("May.csv"));
        assert!(
            merged.anomalies.is_empty(),
            "one session reported twice is not a duplicate id"
        );
    }

    /// `Charge_Session_ID` is not unique in Evolute's reports — the June 2026 report carries
    /// `S37487` on two sessions a week apart. Both count, and both are flagged.
    #[test]
    fn a_reused_id_within_one_file_keeps_both_sessions_and_flags_them() {
        let june = vec![
            session("June.csv", 2, "S37487", "2026-06-18T15:29:00Z", 0.0),
            session("June.csv", 3, "S1", "2026-06-20T12:00:00Z", 4.0),
            session("June.csv", 4, "S37487", "2026-06-25T10:08:00Z", 1.0),
        ];

        let merged = MergedSessions::merge_sessions(vec![june]);
        assert_eq!(
            ids(&merged.sessions),
            ["S37487", "S1", "S37487"],
            "two sessions sharing an id are two sessions"
        );
        // Symmetric: both members of the colliding pair, not only the later one.
        assert_eq!(flagged(&merged.anomalies), [("S37487", 2), ("S37487", 4)]);
    }

    /// Two files disagreeing about one session looks exactly like a reused id from here, and is
    /// treated the same way: nothing is dropped, and both records are flagged for a reader to
    /// judge.
    #[test]
    fn files_that_disagree_about_a_session_keep_both_records() {
        let may = vec![session("May.csv", 2, "S1", "2026-05-30T12:00:00Z", 4.0)];
        let june = vec![session("June.csv", 9, "S1", "2026-05-30T12:00:00Z", 4.5)];

        let merged = MergedSessions::merge_sessions(vec![may, june]);
        assert_eq!(merged.sessions.len(), 2);
        assert_eq!(flagged(&merged.anomalies), [("S1", 2), ("S1", 9)]);
    }

    /// A single list has nothing to collapse across files, so merging one is detection alone. That
    /// is how both readers get the flagging without a second code path.
    #[test]
    fn merging_one_list_leaves_it_alone() {
        let sessions = vec![
            session("June.csv", 2, "S1", "2026-06-01T12:00:00Z", 4.0),
            session("June.csv", 3, "S2", "2026-06-02T12:00:00Z", 5.0),
        ];

        let merged = MergedSessions::merge_sessions(vec![sessions]);
        assert_eq!(ids(&merged.sessions), ["S1", "S2"]);
        assert!(merged.anomalies.is_empty());
    }
}
