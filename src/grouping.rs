use crate::{
    AnomalyKind, EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS, RSession, SESSION_BOUNDARY_RESOLUTION,
    Session, quicksort,
};
use jiff::Timestamp;
use std::{
    cell::{Ref, RefCell},
    cmp::Ordering,
    collections::{BTreeSet, btree_set::Iter},
    fmt,
    rc::Rc,
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Anomalies that may be reported for [`SessionGroup`]
pub enum GroupAnomaly {
    /// The session group size exceeded [`crate::EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`] and was
    /// clamped to it. Contains the original group size.
    ClampedSessionGroup(usize),
}

impl GroupAnomaly {
    /// The variant name, matching [`crate::AnomalyKind::as_str`]'s role: a stable identifier,
    /// distinct from the free-form prose of [`fmt::Display`].
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ClampedSessionGroup(_) => "ClampedSessionGroup",
        }
    }
}

impl fmt::Display for GroupAnomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClampedSessionGroup(_) => write!(
                f,
                "the report claims more sessions were charging at once than a single panel \
                 should allow. The \"Clamped\" estimates use only the {} the panel could have \
                 run; the \"Direct\" estimates use all of them",
                EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS
            ),
        }
    }
}

#[derive(Debug, PartialEq)]
enum EndPoint {
    Left(EndPointData),
    Right(EndPointData),
}

impl EndPoint {
    fn data(&self) -> &EndPointData {
        match self {
            Self::Left(data) | Self::Right(data) => data,
        }
    }

    fn time(&self) -> Timestamp {
        self.data().time
    }

    /// `Left` before `Right` at the same instant. The sweep applies every end-point sharing an
    /// instant together, so this does not change which groups come out; it is here to make the
    /// order total and therefore the sort deterministic.
    fn rank(&self) -> u8 {
        match self {
            Self::Left(_) => 0,
            Self::Right(_) => 1,
        }
    }
}

/// Chronological, and only then by kind and session id.
///
/// This cannot be derived: a derived implementation compares the *variant* first, which would sort
/// every `Left` in the interval ahead of every `Right` and hand the sweep its end-points out of
/// time order.
impl PartialOrd for EndPoint {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(
            self.time()
                .cmp(&other.time())
                .then_with(|| self.rank().cmp(&other.rank()))
                .then_with(|| {
                    let this = self.data().session.as_ref().borrow();
                    let that = other.data().session.as_ref().borrow();
                    this.id.cmp(&that.id)
                }),
        )
    }
}

#[derive(Debug, Clone)]
struct EndPointData {
    time: Timestamp,
    session: RSession,
}

impl PartialEq for EndPointData {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time
            && self.session.as_ref().borrow().id == other.session.as_ref().borrow().id
    }
}

#[derive(Debug, Clone)]
struct SessionGroupCache {
    size: usize,
    agg_avg_power: f64,
    clamped_size: usize,
    clamped_agg_avg_power: f64,
}

#[derive(Debug)]
/// Collection of sessions that have non-trivial intersections with each other and with the interval
/// of interest.
pub struct SessionGroup {
    start: Timestamp,
    end: Timestamp,
    sessions: BTreeSet<RSession>,
    cache: Option<SessionGroupCache>,
}

pub struct SessionIter<'a>(Iter<'a, RSession>);

impl<'a> Iterator for SessionIter<'a> {
    type Item = Ref<'a, Session>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.next() {
            None => None,
            Some(rs) => Some(RefCell::borrow(rs)),
        }
    }
}

impl SessionGroup {
    pub fn start(&self) -> Timestamp {
        self.start
    }

    pub fn end(&self) -> Timestamp {
        self.end
    }

    pub fn session_iter<'a>(&'a self) -> SessionIter<'a> {
        SessionIter(self.sessions.iter())
    }

    fn read_cache<T>(&self, f: impl FnOnce(&SessionGroupCache) -> T) -> T {
        match &self.cache {
            Some(cache) => f(cache),
            None => panic!("SessionGroup cache not initialized"),
        }
    }

    /// Returns the unclamped aggregate average power for the group.
    pub fn agg_avg_power(&self) -> f64 {
        self.read_cache(|c| c.agg_avg_power)
    }

    /// Returns the aggregate average power of sessions in the group, excluding those that end
    /// early in the group's span to the extent that those early-ending sessions put the total
    /// session count above [`EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`].
    pub fn clamped_agg_avg_power(&self) -> f64 {
        self.read_cache(|c| c.clamped_agg_avg_power)
    }

    /// Returns the unclamped number of sessions in the group.
    pub fn size(&self) -> usize {
        self.read_cache(|c| c.size)
    }

    /// Returns the number of sessions in the group, clamped to [`EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`].
    pub fn clamped_size(&self) -> usize {
        self.read_cache(|c| c.clamped_size)
    }

    /// Anomalies this group carries. The group's `sessions` are never filtered — clamping lives
    /// entirely in the derived figures — so [`SessionGroup::size`] keeps reporting the real count
    /// and the anomaly can carry it.
    ///
    /// A clamped group means more sessions were reported as concurrent than one panel's PLC will
    /// run at once, so either a second panel is installed or the report is wrong. Either way the
    /// clamped estimates understate this group and the `direct` ones should be read alongside
    /// them. See [`crate::PowerEstimates`].
    pub fn anomalies(&self) -> Vec<GroupAnomaly> {
        if self.clamped_size() < self.size() {
            vec![GroupAnomaly::ClampedSessionGroup(self.size())]
        } else {
            Vec::new()
        }
    }

    /// Elapsed time the group spans.
    ///
    /// # Panics
    ///
    /// If the group's end precedes its start, or if it is unbounded — carrying the `Timestamp::MAX`
    /// that means the sweep never closed it.
    ///
    /// Neither is reachable from well-formed input. Sessions whose start, end and duration
    /// contradict each other are flagged [`AnomalyKind::InconsistentDuration`] and excluded before
    /// they reach this module, and [`end_points_for_interval`] clamps every end-point into the
    /// interval of interest, so `start <= end` holds for any group the sweep builds. A panic here
    /// therefore means a bug in the sweep, not bad data — and bad data of that shape should have
    /// been stopped further upstream. Failing loudly is the point: the alternative is a group whose
    /// reported span is arbitrary.
    pub fn duration(&self) -> Duration {
        assert!(
            self.end != Timestamp::MAX,
            "unbounded session group starting at {}: the end-point sweep never closed it",
            self.start
        );
        Duration::try_from(self.end.duration_since(self.start)).unwrap_or_else(|_| {
            panic!(
                "session group ends at {} before it starts at {}",
                self.end, self.start
            )
        })
    }

    fn pure_agg_avg_power(&self) -> f64 {
        self.sessions
            .iter()
            .map(|s| s.as_ref().borrow().avg_power)
            .sum()
    }

    fn pure_size(&self) -> usize {
        self.sessions.len()
    }

    /// Returns the number of sessions in the group, not to exceed
    /// [`EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`], since a panel's PLC will not run more than that
    /// at once.
    ///
    /// This is `min(size, MAX)`, and [`SessionGroup::eligible_sessions`] always yields exactly that
    /// many — the two are the count and the sum over the *same* set. Pinned by
    /// [`test::clamped_size_matches_the_eligible_set`].
    pub(crate) fn pure_clamped_size(&self) -> usize {
        self.pure_size().min(EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS)
    }

    /// The sessions the clamped figures are computed over: all of them while the group holds no
    /// more than [`EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`], and otherwise exactly that many.
    ///
    /// Which ones go is decided in two tiers, ranked on how far a session's `conn_end` reaches
    /// past the group's start:
    ///
    /// 1. **Short-overlap** — `conn_end - start <= SESSION_BOUNDARY_RESOLUTION`. Dropped first,
    ///    lowest average power first.
    /// 2. **Long-overlap** — everything else. Dropped only when emptying tier 1 still leaves the
    ///    group oversized, again lowest power first.
    ///
    /// Ties are broken on session id, so the result never depends on iteration order.
    ///
    /// The tier boundary is derived, not chosen. `conn_end` is `Adj_conn_end`: the reported end
    /// padded to the exclusive end of the minute it fell in, so the true end lies somewhere in
    /// `[conn_end - SESSION_BOUNDARY_RESOLUTION, conn_end)`. A short-overlap session has
    /// `conn_end - SESSION_BOUNDARY_RESOLUTION <= start`, so it may have truly ended at or before
    /// the group began, contributing nothing to it. A long-overlap session's true end is strictly
    /// after `start`, so it was still connected once the group was under way. The comparison is
    /// strict because the intervals are half-open: at exactly one resolution the true end can *be*
    /// `start`, and that is no overlap at all.
    ///
    /// Tier 2 establishes presence in the group's span — which is what the sweep means by
    /// concurrency — and not verified pairwise overlap with any particular member: the group's
    /// start may itself be another session's reported start, whose true start lies up to one
    /// resolution later.
    ///
    /// Tier 1 can only be non-empty for a group no longer than one [`SESSION_BOUNDARY_RESOLUTION`].
    /// Every session in a group outlasts the group — [`end_points_to_groups`] emits the run
    /// `[run_start, time)` *before* applying any end-point at `time` — so
    /// `conn_end - start >= self.duration()` for every member. A short-overlap session therefore
    /// implies a group of at most one tick, so the rule fires only where the uncertainty it answers
    /// to actually lives, and nowhere else.
    fn eligible_sessions(&self) -> Vec<RSession> {
        let mut ranked: Vec<(bool, f64, String, RSession)> = self
            .sessions
            .iter()
            .map(|s| {
                let session = s.as_ref().borrow();
                let overlap = Duration::try_from(session.conn_end.duration_since(self.start))
                    .unwrap_or_else(|_| {
                        panic!(
                            "session ends at {} before session group starts at {}",
                            session.conn_end, self.start
                        )
                    });
                let long_overlap = overlap > SESSION_BOUNDARY_RESOLUTION;
                (
                    long_overlap,
                    session.avg_power,
                    session.id.clone(),
                    s.clone(),
                )
            })
            .collect();

        if ranked.len() > EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS {
            // Drop order: short-overlap first, then ascending average power, then id. Sorting by
            // that puts the first session to drop at the front, so the survivors are the tail.
            ranked.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.1.total_cmp(&b.1))
                    .then_with(|| a.2.cmp(&b.2))
            });
            ranked = ranked.split_off(ranked.len() - EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS);
        }

        ranked.into_iter().map(|(_, _, _, s)| s).collect()
    }

    /// Calculates the aggregate average power over [`SessionGroup::eligible_sessions`], i.e. over
    /// one panel's worth of the group at most.
    pub(crate) fn pure_clamped_agg_avg_power(&self) -> f64 {
        self.eligible_sessions()
            .iter()
            .map(|s| s.as_ref().borrow().avg_power)
            .sum()
    }

    pub(crate) fn initialize_cache(&mut self) {
        let size = self.pure_size();
        let clamped_size = self.pure_clamped_size();
        let agg_avg_power = self.pure_agg_avg_power();
        let clamped_agg_avg_power = self.pure_clamped_agg_avg_power();
        self.cache = Some(SessionGroupCache {
            size,
            agg_avg_power,
            clamped_size,
            clamped_agg_avg_power,
        });
    }
}

/// Note this mutates `sessions`: those whose overlap with `interval` falls entirely within a
/// boundary margin have [`AnomalyKind::IntersectsBoundaryMarginOnly`] appended. Calling it twice
/// with the same sessions appends it twice.
pub fn groups_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &mut Vec<RSession>,
) -> Vec<Rc<SessionGroup>> {
    let end_points = end_points_for_interval(interval, sessions, SESSION_BOUNDARY_RESOLUTION);
    end_points_to_groups(end_points)
}

/// Returns an unsorted list of end-points corresponding to the sessions that non-trivially intersect
/// `interval`.
///
/// A session takes part only if it is active somewhere in `interval` reduced by `boundary_margin`
/// at each end. That reduction is what the reporting uncertainty costs, and it is derived from the
/// two windows the uncertainty leaves — assuming, as every call site does, that `boundary_margin`
/// is one [`SESSION_BOUNDARY_RESOLUTION`]. `conn_start` is truncated to the minute, so the true
/// start lies in `[conn_start, conn_start + margin)`; `conn_end` is `Adj_conn_end`, the reported
/// end padded to the exclusive end of its minute, so the true end lies in
/// `[conn_end - margin, conn_end)`.
///
/// `conn_end > lo + margin` is then exactly the condition that the true end is after `lo` whatever
/// it turns out to be: at `conn_end == lo + margin` the true end may be `lo` itself, and under
/// half-open intervals that is no overlap. The mirror test, `conn_start < hi - margin`, is a hair
/// stronger than it need be — `conn_start == hi - margin` already puts the true start before `hi` —
/// but the overlap that case buys is under one margin and may be arbitrarily small, so it goes with
/// the rest.
///
/// The margin decides *membership* only — end-points are clamped to the real interval, so the
/// groups tile it and a reported peak window is a wall-clock window that can be matched against the
/// metering data. A session whose overlap falls entirely within a margin is flagged
/// [`AnomalyKind::IntersectsBoundaryMarginOnly`].
fn end_points_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &mut Vec<RSession>,
    boundary_margin: Duration,
) -> Vec<EndPoint> {
    let lo = interval.0;
    let hi = interval.1;
    let lo_t = lo + boundary_margin;
    let hi_t = hi - boundary_margin;
    let mut end_points = Vec::<EndPoint>::new();
    for s in sessions {
        if s.as_ref().borrow().conn_start < hi_t && s.as_ref().borrow().conn_end > lo_t {
            let s_mut = &mut s.borrow_mut();

            let left = EndPoint::Left(EndPointData {
                time: s_mut.conn_start.max(lo),
                session: s.clone(),
            });
            let right = EndPoint::Right(EndPointData {
                time: s_mut.conn_end.min(hi),
                session: s.clone(),
            });
            end_points.push(left);
            end_points.push(right);
        } else if s.as_ref().borrow().intersects(interval) {
            // Every other kind is settled at conversion time and arrives on the session already.
            // This one cannot be: it depends on which interval of interest was chosen.
            let s_mut = &mut s.borrow_mut();
            s_mut
                .anomalies
                .push(AnomalyKind::IntersectsBoundaryMarginOnly);
        }
    }
    end_points
}

/// Sweeps the sorted end-points, emitting one [`SessionGroup`] per maximal run of time over which
/// the set of active sessions does not change. The groups come out in chronological order and,
/// taken together, tile the part of the interval of interest that has any session in it.
///
/// Every end-point sharing an instant is applied *after* the run ending at that instant has been
/// emitted, so a group's session set is exactly the set active throughout it — not the set on
/// either side of a change. Zero-length runs are never emitted, which is what stops a session that
/// ends exactly where another begins from producing an empty sliver between them.
///
/// Groups come out behind an [`Rc`] because a [`crate::PowerEstimate`] names the group it was drawn
/// from and the report holds the whole tiling; sharing beats either copying or threading a lifetime
/// through both.
fn end_points_to_groups(mut end_points: Vec<EndPoint>) -> Vec<Rc<SessionGroup>> {
    quicksort(&mut end_points);

    let mut groups = Vec::new();
    let mut active = BTreeSet::<RSession>::new();
    let mut run_start = Timestamp::MIN;

    let mut i = 0;
    while i < end_points.len() {
        let time = end_points[i].time();

        // Close the run that was running up to this instant. `active` is empty before the first
        // left edge and between sessions, and those gaps are not groups.
        if !active.is_empty() && run_start < time {
            let mut group = SessionGroup {
                start: run_start,
                end: time,
                sessions: active.clone(),
                cache: None,
            };
            group.initialize_cache();
            groups.push(Rc::new(group));
        }

        while i < end_points.len() && end_points[i].time() == time {
            match &end_points[i] {
                EndPoint::Left(data) => active.insert(data.session.clone()),
                EndPoint::Right(data) => active.remove(&data.session),
            };
            i += 1;
        }

        run_start = time;
    }

    debug_assert!(
        active.is_empty(),
        "every session that opened should have closed by the end of the sweep"
    );
    groups
}

#[cfg(test)]
// cargo test --package ev-peak-contrib --lib --all-features -- peak_contrib::test --nocapture
mod test {
    use crate::AnomalyKind;

    use super::*;
    use std::rc::Rc;

    #[test]
    fn test_end_point_kind_order() {
        let session = Rc::new(RefCell::new(Session {
            id: Default::default(),
            row: Default::default(),
            conn_start: Default::default(),
            raw_conn_end: Default::default(),
            conn_end: Default::default(),
            conn_duration: Default::default(),
            charge_time: Default::default(),
            energy_use: Default::default(),
            avg_power: Default::default(),
            anomalies: Default::default(),
        }));
        let data1 = EndPointData {
            time: Default::default(),
            session: session.clone(),
        };
        let data2 = EndPointData {
            time: Default::default(),
            session: session.clone(),
        };
        assert!(EndPoint::Left(data1) < EndPoint::Right(data2));
    }

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// A session over `[start, end]` UTC, with enough of a `Session` filled in for the grouping
    /// logic to work on.
    fn rsession(id: &str, start: &str, end: &str) -> RSession {
        let conn_start = ts(start);
        let conn_end = ts(end);
        Rc::new(RefCell::new(Session {
            id: id.to_owned(),
            row: 2,
            conn_start,
            raw_conn_end: conn_end,
            conn_end,
            conn_duration: conn_end.duration_since(conn_start).unsigned_abs(),
            charge_time: Duration::from_secs(60),
            energy_use: 1.0,
            avg_power: 1.0,
            anomalies: Vec::new(),
        }))
    }

    /// A 15-minute interval of interest.
    const LO: &str = "2026-06-01T20:00:00Z";
    const HI: &str = "2026-06-01T20:15:00Z";

    /// The margin is a boundary rule, not a minimum-overlap rule: a short session inside the
    /// interval counts, while one whose only overlap sits in a margin does not.
    #[test]
    fn boundary_margin_excludes_only_overlap_at_the_edges() {
        let cases = [
            // (id, start, end, included?)
            (
                "inside",
                "2026-06-01T20:05:00Z",
                "2026-06-01T20:05:10Z",
                true,
            ),
            // Ten seconds, but wholly within the leading margin.
            (
                "head",
                "2026-06-01T20:00:20Z",
                "2026-06-01T20:00:30Z",
                false,
            ),
            // Overlaps only the trailing margin.
            (
                "tail",
                "2026-06-01T20:14:30Z",
                "2026-06-01T20:20:00Z",
                false,
            ),
            // Reaches past the leading margin, so it counts.
            (
                "spans",
                "2026-06-01T19:50:00Z",
                "2026-06-01T20:02:00Z",
                true,
            ),
        ];
        for (id, start, end, included) in cases {
            let s = rsession(id, start, end);
            let mut sessions = vec![s.clone()];
            let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
            assert_eq!(!groups.is_empty(), included, "{id}");
            assert_eq!(
                s.borrow()
                    .anomalies
                    .contains(&AnomalyKind::IntersectsBoundaryMarginOnly),
                !included,
                "{id}"
            );
        }
    }

    /// Renders the groups as `(start_offset_secs, end_offset_secs, ids)` relative to [`LO`].
    fn tiling(groups: &[Rc<SessionGroup>]) -> Vec<(i64, i64, Vec<String>)> {
        groups
            .iter()
            .map(|g| {
                (
                    g.start().duration_since(ts(LO)).as_secs(),
                    g.end().duration_since(ts(LO)).as_secs(),
                    g.session_iter().map(|s| s.id.clone()).collect(),
                )
            })
            .collect()
    }

    /// The worked example from the module diagram: nested and staggered sessions must tile the
    /// occupied time exactly once, with each group carrying the set active throughout it.
    #[test]
    fn overlapping_sessions_tile_the_interval() {
        let mut sessions = vec![
            rsession("A", "2026-06-01T20:01:00Z", "2026-06-01T20:12:00Z"),
            rsession("B", "2026-06-01T20:04:00Z", "2026-06-01T20:08:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);

        assert_eq!(
            tiling(&groups),
            [
                (60, 240, vec!["A".to_owned()]),
                (240, 480, vec!["A".to_owned(), "B".to_owned()]),
                (480, 720, vec!["A".to_owned()]),
            ]
        );
        // Contiguous: each group starts where the previous ended.
        for pair in groups.windows(2) {
            assert_eq!(pair[0].end(), pair[1].start());
        }
        assert_eq!(groups[1].size(), 2);
        assert_eq!(groups[1].duration(), Duration::from_secs(240));
    }

    /// Two sessions that do not overlap. The gap between them is not a group, and the second
    /// session's edges must sort after the first's — the case a variant-major ordering gets wrong,
    /// since it would place B's left edge before A's right edge.
    #[test]
    fn disjoint_sessions_leave_a_gap_and_stay_in_time_order() {
        let mut sessions = vec![
            rsession("A", "2026-06-01T20:01:00Z", "2026-06-01T20:03:00Z"),
            rsession("B", "2026-06-01T20:05:00Z", "2026-06-01T20:07:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);

        assert_eq!(
            tiling(&groups),
            [
                (60, 180, vec!["A".to_owned()]),
                (300, 420, vec!["B".to_owned()]),
            ]
        );
    }

    /// A session ending exactly where the next begins produces no empty sliver between them, and
    /// the two are never reported as concurrent.
    #[test]
    fn abutting_sessions_produce_no_empty_group() {
        let mut sessions = vec![
            rsession("A", "2026-06-01T20:01:00Z", "2026-06-01T20:05:00Z"),
            rsession("B", "2026-06-01T20:05:00Z", "2026-06-01T20:09:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);

        assert_eq!(
            tiling(&groups),
            [
                (60, 300, vec!["A".to_owned()]),
                (300, 540, vec!["B".to_owned()]),
            ]
        );
        assert!(groups.iter().all(|g| g.size() == 1));
    }

    /// Three sessions starting at the same instant: one group, not three.
    #[test]
    fn simultaneous_starts_make_one_group() {
        let mut sessions = vec![
            rsession("A", "2026-06-01T20:02:00Z", "2026-06-01T20:10:00Z"),
            rsession("B", "2026-06-01T20:02:00Z", "2026-06-01T20:10:00Z"),
            rsession("C", "2026-06-01T20:02:00Z", "2026-06-01T20:10:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].size(), 3);
        assert_eq!(groups[0].agg_avg_power(), 3.0);
    }

    /// As [`rsession`], with an explicit average power. The clamping rule ranks on it.
    fn rsession_kw(id: &str, start: &str, end: &str, avg_power: f64) -> RSession {
        let s = rsession(id, start, end);
        s.borrow_mut().avg_power = avg_power;
        s
    }

    /// `n` sessions spanning `20:02:00`–`20:10:00`, so every one of them outlasts the single group
    /// they induce and all land in the long-overlap tier. Powers are `1.0, 2.0, …`.
    fn long_overlap_group(n: usize) -> Vec<RSession> {
        (1..=n)
            .map(|i| {
                rsession_kw(
                    &format!("L{i:02}"),
                    "2026-06-01T20:02:00Z",
                    "2026-06-01T20:10:00Z",
                    i as f64,
                )
            })
            .collect()
    }

    /// Below the panel limit nothing is dropped, and the clamped figures equal the plain ones.
    #[test]
    fn clamping_is_inert_at_or_below_the_panel_limit() {
        for n in [1, 5, EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS] {
            let mut sessions = long_overlap_group(n);
            let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
            assert_eq!(groups.len(), 1, "n={n}");
            let g = &groups[0];
            assert_eq!(g.clamped_size(), g.size(), "n={n}");
            assert_eq!(g.clamped_agg_avg_power(), g.agg_avg_power(), "n={n}");
            assert!(g.anomalies().is_empty(), "n={n}");
        }
    }

    /// Over the limit, the lowest-power sessions are the ones that go, and exactly enough of them.
    #[test]
    fn oversized_group_drops_the_weakest_sessions() {
        for n in [11, 12, 14] {
            let mut sessions = long_overlap_group(n);
            let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
            let g = &groups[0];

            assert_eq!(g.size(), n, "n={n}");
            assert_eq!(
                g.clamped_size(),
                EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS,
                "n={n}"
            );
            // Powers are 1.0..=n, so the survivors are the top ten and their sum is known.
            let kept: f64 = ((n - EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS + 1)..=n)
                .map(|i| i as f64)
                .sum();
            assert_eq!(g.clamped_agg_avg_power(), kept, "n={n}");
            assert_eq!(
                g.anomalies(),
                vec![GroupAnomaly::ClampedSessionGroup(n)],
                "n={n}"
            );
        }
    }

    /// The tiers, which are the point of the rule. Eleven sessions have an adjusted end exactly one
    /// tick past the group's start, so each of them may in truth have ended before the group began;
    /// one session spans the group outright and was certainly still connected. Two must go, and
    /// both come from the suspect tier — even though the session that spans the group is the
    /// weakest in it.
    #[test]
    fn short_overlap_sessions_are_dropped_before_long_overlap_ones() {
        let mut sessions: Vec<RSession> = (1..=11)
            .map(|i| {
                let kw = match i {
                    1 => 1.0,
                    2 => 2.0,
                    _ => 9.0,
                };
                rsession_kw(
                    &format!("S{i:02}"),
                    "2026-06-01T20:02:00Z",
                    "2026-06-01T20:03:00Z",
                    kw,
                )
            })
            .collect();
        sessions.push(rsession_kw(
            "Lspan",
            "2026-06-01T20:02:00Z",
            "2026-06-01T20:10:00Z",
            0.5,
        ));

        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
        let g = &groups[0];
        assert_eq!(g.start(), ts("2026-06-01T20:02:00Z"));
        assert_eq!(g.duration(), SESSION_BOUNDARY_RESOLUTION);
        assert_eq!(g.size(), 12);

        let kept: Vec<String> = g
            .eligible_sessions()
            .iter()
            .map(|s| s.as_ref().borrow().id.clone())
            .collect();
        assert!(kept.contains(&"Lspan".to_owned()), "{kept:?}");
        assert!(!kept.contains(&"S01".to_owned()), "{kept:?}");
        assert!(!kept.contains(&"S02".to_owned()), "{kept:?}");
        // Nine short-overlap survivors at 9.0, plus the spanning session at 0.5.
        assert_eq!(g.clamped_agg_avg_power(), 9.0 * 9.0 + 0.5);
    }

    /// The invariant `pure_clamped_size` relies on: it is `min(size, MAX)` computed independently,
    /// and must always equal the size of the set the clamped power is summed over.
    #[test]
    fn clamped_size_matches_the_eligible_set() {
        for n in [1, 9, 10, 11, 14] {
            let mut sessions = long_overlap_group(n);
            let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
            let g = &groups[0];
            assert_eq!(g.eligible_sessions().len(), g.clamped_size(), "n={n}");
        }
    }

    /// The margin decides membership only. End-points are clamped to the real interval, so a
    /// reported peak window is a wall-clock window that can be matched against the metering data.
    #[test]
    fn group_endpoints_are_clamped_to_the_real_interval() {
        let mut sessions = vec![rsession(
            "covers",
            "2026-06-01T19:00:00Z",
            "2026-06-01T21:00:00Z",
        )];
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].start(), ts(LO));
        assert_eq!(groups[0].end(), ts(HI));
        assert_eq!(groups[0].duration(), Duration::from_secs(15 * 60));
    }
}
