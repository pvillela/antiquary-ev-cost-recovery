use crate::{RSession, Session, quicksort};
use jiff::Timestamp;
use std::{
    cell::{Ref, RefCell},
    cmp::Ordering,
    collections::{BTreeSet, btree_set::Iter},
    rc::Rc,
    time::Duration,
};

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

/// Which reading of a group a figure is computed under.
///
/// The two differ only for a [dubious](SessionGroup::is_dubious) group, where the reported times
/// cannot settle whether two members overlapped or merely abutted. Everywhere else they coincide.
///
/// See README.md, "The two estimate sets".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The group's membership taken at face value. What the no-argument [`SessionGroup::size`] and
    /// [`SessionGroup::agg_avg_power`] answer.
    Nominal,
    /// As little overlap between the members as their reported times allow.
    MinOverlap,
}

impl View {
    /// Both, in the order the report prints them — which is also [`View::index`] order, so
    /// `ALL[i].index() == i`. [`SessionGroup::initialize_cache`] relies on that.
    pub const ALL: [Self; 2] = [Self::Nominal, Self::MinOverlap];

    fn index(self) -> usize {
        match self {
            Self::Nominal => 0,
            Self::MinOverlap => 1,
        }
    }
}

/// The figures a group reports under one [`View`].
#[derive(Debug, Clone, Copy)]
struct Figures {
    size: usize,
    agg_avg_power: f64,
}

/// The [`Figures`] for both views, indexed by [`View::index`]. Computed once, when the sweep closes
/// the group: each entry is a fold over the members, and the report asks for both of them.
#[derive(Debug, Clone)]
struct SessionGroupCache([Figures; 2]);

/// Where a member sits relative to its group's two ends, which is what decides how far its true
/// times can be moved inside the group. See [`SessionGroup::min_overlap_figures`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    /// Starts before the group and ends after it: certainly running throughout.
    Ba,
    /// Starts before the group and ends at its end, so its true end falls inside.
    Bi,
    /// Starts at the group's start and ends after it, so its true start falls inside.
    Ia,
    /// Confined to the group, both true end-points falling inside it.
    Ii,
}

impl Bucket {
    fn of(session: &Session, start: Timestamp, end: Timestamp) -> Self {
        match (session.conn_start < start, session.adj_conn_end > end) {
            (true, true) => Self::Ba,
            (true, false) => Self::Bi,
            (false, true) => Self::Ia,
            (false, false) => Self::Ii,
        }
    }
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

    fn figures(&self, view: View) -> Figures {
        match &self.cache {
            Some(cache) => cache.0[view.index()],
            None => panic!("SessionGroup cache not initialized"),
        }
    }

    /// Aggregate average power over the members `view` counts.
    pub fn agg_avg_power_in(&self, view: View) -> f64 {
        self.figures(view).agg_avg_power
    }

    /// Number of members `view` counts.
    pub fn size_in(&self, view: View) -> usize {
        self.figures(view).size
    }

    /// Aggregate average power of the group exactly as reported. Shorthand for [`View::Nominal`],
    /// which is what the report's Session groups table shows.
    pub fn agg_avg_power(&self) -> f64 {
        self.agg_avg_power_in(View::Nominal)
    }

    /// Number of sessions in the group as reported. Shorthand for [`View::Nominal`].
    pub fn size(&self) -> usize {
        self.size_in(View::Nominal)
    }

    /// Whether the reported times leave two members' overlap undecided.
    ///
    /// Tested on the sizes rather than on the buckets directly, because the two are equivalent —
    /// see [`SessionGroup::min_overlap_figures`] — and this way the invariant that a marked group is one
    /// whose figures actually differ cannot drift from the marking. The power figures cannot
    /// stand in: a member drawing zero power can leave the aggregates equal while the sizes differ.
    ///
    /// The group's `sessions` are never filtered. Doubt changes which members a *figure* folds
    /// over, never which sessions belong to the group, so [`SessionGroup::size`] keeps reporting
    /// the real count.
    pub fn is_dubious(&self) -> bool {
        self.size_in(View::MinOverlap) < self.size()
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

    /// The figures with every member taken at face value.
    fn nominal_figures(&self) -> Figures {
        Figures {
            size: self.sessions.len(),
            agg_avg_power: self
                .sessions
                .iter()
                .map(|s| s.as_ref().borrow().avg_power)
                .sum(),
        }
    }

    /// The figures under as little overlap between the members as their reported times allow.
    ///
    /// Every member runs the group's whole span — [`end_points_to_groups`] emits the run
    /// `[run_start, time)` *before* applying any end-point at `time`, so `conn_start <= start` and
    /// `adj_conn_end >= end` for all of them. No member literally starts or ends inside its own
    /// group, then; what varies is whether each of those relations is strict, which sorts the
    /// members into four classes. Writing `ba` for "starts before, ends after" and so on:
    ///
    /// | set | `conn_start` | `adj_conn_end` | what is certain |
    /// |-----|--------------|----------------|-----------------|
    /// | `ba` | `< start`   | `> end`        | runs through all of the group |
    /// | `bi` | `< start`   | `== end`       | its true end falls inside the group |
    /// | `ia` | `== start`  | `> end`        | its true start falls inside the group |
    /// | `ii` | `== start`  | `== end`       | both its true end-points fall inside |
    ///
    /// The tests read each session's own `conn_start` and `adj_conn_end`. Not `conn_end`, which has
    /// not had the truncation padding applied; and not the end-points clamped into the interval of
    /// interest, which would make a session running through the interval's edge look like one
    /// confined to the group and drop it from the unconditional `ba` term.
    ///
    /// `ba` members are certainly present throughout, so they are added unconditionally. The rest
    /// can be arranged so that at most one of the three remaining classes is drawing at any one
    /// instant — every `bi` ending as early inside the group as it can, every `ia` starting as late
    /// as it can, the `ii` members spread out between them — and no two `bi` can ever be separated
    /// from each other, nor any two `ia`, since each pair shares an end of the group.
    ///
    /// **Narrowness is screened for first, and the screen is not an optimisation.** The reading of
    /// the table above holds only at a width of one [`crate::SESSION_BOUNDARY_RESOLUTION`], where
    /// `adj_conn_end == end` puts the true end in `[end - R, end)`, which *is* the group. Wider,
    /// the classes do not empty out and the arithmetic understates: sessions `A = [20:02, 20:05)`
    /// and `B = [20:03, 20:05)` give the group `[20:03, 20:05)`, two resolutions wide, with `A` in
    /// `bi` and `B` in `ii` — yet `A` runs from before the group starts until at least `20:04` and
    /// `B` starts before `20:04`, so the two certainly overlap and the group's size is not in doubt.
    ///
    /// No group wider than one resolution can be dubious at all. Group durations are multiples of
    /// `R`, so such a group is at least `2R` wide; every member's true start is before `start + R`
    /// and every member's true end is at or after `end - R >= start + R`, so at the latest true
    /// start every member is running. See README.md, "Dubious groups".
    /// The aggregate is summed over the surviving members **in the group's own iteration order**,
    /// not assembled from the bucket subtotals. Floating-point addition is not associative, so
    /// `(ba + bi)` and a straight fold over the same four sessions can differ in the last bit —
    /// enough to defeat [`crate::max_group_by`]'s tie-break in the very case it exists for, where a
    /// dubious group's minimum reading reproduces a neighbouring group's membership exactly. Summed
    /// in one order, equal multisets give equal totals and the tie is a real tie.
    fn min_overlap_figures(&self) -> Figures {
        if self.duration() > crate::SESSION_BOUNDARY_RESOLUTION {
            return self.nominal_figures();
        }

        let buckets: Vec<(Bucket, f64)> = self
            .sessions
            .iter()
            .map(|s| {
                let session = s.as_ref().borrow();
                (
                    Bucket::of(&session, self.start, self.end),
                    session.avg_power,
                )
            })
            .collect();

        let total = |b: Bucket| -> (usize, f64) {
            buckets
                .iter()
                .filter(|(bucket, _)| *bucket == b)
                .fold((0, 0.0), |(n, sum), (_, p)| (n + 1, sum + p))
        };
        let (bi_size, bi_power) = total(Bucket::Bi);
        let (ia_size, ia_power) = total(Bucket::Ia);

        // Of the members confined to the group, only one can be relied on to be drawing at any
        // instant, so the strongest of them stands for the whole class. Ties take the first in
        // iteration order, which keeps the choice independent of how the set was built.
        let strongest_ii = buckets
            .iter()
            .enumerate()
            .filter(|(_, (b, _))| *b == Bucket::Ii)
            .max_by(|(_, (_, a)), (_, (_, b))| b.total_cmp(a).reverse())
            .map(|(i, _)| i);
        let ii_power = strongest_ii.map_or(0.0, |i| buckets[i].1);

        // Whichever class contributes most joins the members that certainly span the group.
        let winner = if bi_power >= ia_power && bi_power >= ii_power {
            Bucket::Bi
        } else if ia_power >= ii_power {
            Bucket::Ia
        } else {
            Bucket::Ii
        };
        let survives = |i: usize, b: Bucket| match b {
            Bucket::Ba => true,
            Bucket::Ii => winner == Bucket::Ii && strongest_ii == Some(i),
            _ => b == winner,
        };

        let agg_avg_power = buckets
            .iter()
            .enumerate()
            .filter(|(i, (b, _))| survives(*i, *b))
            .map(|(_, (_, p))| *p)
            .sum();

        // The size is the same choice made on counts rather than on power, so the two may name
        // different classes — a class can hold the most sessions and the least load.
        let ba_size = total(Bucket::Ba).0;
        Figures {
            size: ba_size
                + bi_size
                    .max(ia_size)
                    .max(usize::from(strongest_ii.is_some())),
            agg_avg_power,
        }
    }

    /// Fills the [`Figures`] for both views. `View::ALL` is in [`View::index`] order, so the array
    /// `map` lands each entry where `figures` will look for it.
    pub(crate) fn initialize_cache(&mut self) {
        let cache = SessionGroupCache(View::ALL.map(|view| match view {
            View::Nominal => self.nominal_figures(),
            View::MinOverlap => self.min_overlap_figures(),
        }));
        self.cache = Some(cache);
    }
}

/// The tiling of `interval` induced by the sessions that intersect it.
pub fn groups_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &[RSession],
) -> Vec<Rc<SessionGroup>> {
    end_points_to_groups(end_points_for_interval(interval, sessions))
}

/// Returns an unsorted list of end-points corresponding to the sessions that intersect `interval`.
///
/// Membership is the *possible*-overlap test, `conn_start < hi && adj_conn_end > lo` — a session
/// takes part if it might have been running. Understating a maximum is the unsafe error, so the
/// estimates are drawn from every session that could have contributed, and what the reported times
/// leave undecided is answered per group by [`SessionGroup::min_overlap_figures`] rather than by
/// admitting or refusing a session here.
///
/// End-points are clamped into `interval`, so the groups tile it and a reported peak window is a
/// wall-clock window that can be matched against the metering data. The clamped values are used
/// only to place the group boundaries: the bucket tests read each session's own times, since a
/// session running through an edge of `interval` is certainly present throughout the groups at
/// that edge and must not be mistaken for one confined to them.
fn end_points_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &[RSession],
) -> Vec<EndPoint> {
    let lo = interval.0;
    let hi = interval.1;
    let mut end_points = Vec::<EndPoint>::new();
    for s in sessions {
        let (conn_start, adj_conn_end) = {
            let session = s.as_ref().borrow();
            (session.conn_start, session.adj_conn_end)
        };

        if conn_start >= hi || adj_conn_end <= lo {
            continue;
        }

        end_points.push(EndPoint::Left(EndPointData {
            time: conn_start.max(lo),
            session: s.clone(),
        }));
        end_points.push(EndPoint::Right(EndPointData {
            time: adj_conn_end.min(hi),
            session: s.clone(),
        }));
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
    use super::*;
    use crate::SESSION_BOUNDARY_RESOLUTION;
    use std::rc::Rc;

    #[test]
    fn test_end_point_kind_order() {
        let session = Rc::new(RefCell::new(Session {
            id: Default::default(),
            row: Default::default(),
            conn_start: Default::default(),
            conn_end: Default::default(),
            adj_conn_end: Default::default(),
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
        let adj_conn_end = ts(end);
        Rc::new(RefCell::new(Session {
            id: id.to_owned(),
            row: 2,
            conn_start,
            conn_end: adj_conn_end,
            adj_conn_end,
            conn_duration: adj_conn_end.duration_since(conn_start).unsigned_abs(),
            charge_time: Duration::from_secs(60),
            energy_use: 1.0,
            avg_power: 1.0,
            anomalies: Vec::new(),
        }))
    }

    /// A 15-minute interval of interest.
    const LO: &str = "2026-06-01T20:00:00Z";
    const HI: &str = "2026-06-01T20:15:00Z";

    /// A session that misses the interval entirely takes no part in the tiling.
    #[test]
    fn sessions_outside_the_interval_produce_no_groups() {
        for (id, start, end) in [
            ("before", "2026-06-01T19:00:00Z", "2026-06-01T20:00:00Z"),
            ("after", "2026-06-01T20:15:00Z", "2026-06-01T20:30:00Z"),
        ] {
            let sessions = vec![rsession(id, start, end)];
            let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
            assert!(groups.is_empty(), "{id}");
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
        let sessions = vec![
            rsession("A", "2026-06-01T20:01:00Z", "2026-06-01T20:12:00Z"),
            rsession("B", "2026-06-01T20:04:00Z", "2026-06-01T20:08:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);

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
        let sessions = vec![
            rsession("A", "2026-06-01T20:01:00Z", "2026-06-01T20:03:00Z"),
            rsession("B", "2026-06-01T20:05:00Z", "2026-06-01T20:07:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);

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
        let sessions = vec![
            rsession("A", "2026-06-01T20:01:00Z", "2026-06-01T20:05:00Z"),
            rsession("B", "2026-06-01T20:05:00Z", "2026-06-01T20:09:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);

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
        let sessions = vec![
            rsession("A", "2026-06-01T20:02:00Z", "2026-06-01T20:10:00Z"),
            rsession("B", "2026-06-01T20:02:00Z", "2026-06-01T20:10:00Z"),
            rsession("C", "2026-06-01T20:02:00Z", "2026-06-01T20:10:00Z"),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].size(), 3);
        assert_eq!(groups[0].agg_avg_power(), 3.0);
    }

    /// As [`rsession`], with an explicit average power, for the figure arithmetic.
    fn rsession_kw(id: &str, start: &str, end: &str, avg_power: f64) -> RSession {
        let s = rsession(id, start, end);
        s.borrow_mut().avg_power = avg_power;
        s
    }

    /// The group starting at `20:04` in each of the arrangements below, which is one resolution
    /// wide in every one of them.
    fn narrow_group(sessions: &[RSession]) -> Rc<SessionGroup> {
        groups_for_interval((ts(LO), ts(HI)), sessions)
            .into_iter()
            .find(|g| g.start() == ts("2026-06-01T20:04:00Z"))
            .expect("the arrangement should produce a group starting at 20:04")
    }

    /// One session ending inside the group and another starting inside it: the two may have
    /// overlapped for part of the minute or merely abutted, and the reported times cannot say.
    #[test]
    fn a_session_ending_and_one_starting_make_the_group_dubious() {
        let sessions = vec![
            rsession_kw("ends", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 3.0),
            rsession_kw(
                "starts",
                "2026-06-01T20:04:00Z",
                "2026-06-01T20:08:00Z",
                4.0,
            ),
        ];
        let g = narrow_group(&sessions);
        assert_eq!(g.duration(), SESSION_BOUNDARY_RESOLUTION);
        assert_eq!(g.size(), 2);
        assert!(g.is_dubious());
        assert_eq!(g.size_in(View::MinOverlap), 1);
        assert_eq!(g.agg_avg_power_in(View::MinOverlap), 4.0);
    }

    /// Two sessions ending inside the group are both running at its start, so they are certainly
    /// concurrent however the truncation fell. Narrow is not the same as dubious.
    #[test]
    fn two_sessions_ending_in_the_group_are_not_dubious() {
        let sessions = vec![
            rsession_kw(
                "ends_a",
                "2026-06-01T20:02:00Z",
                "2026-06-01T20:05:00Z",
                3.0,
            ),
            rsession_kw(
                "ends_b",
                "2026-06-01T20:03:00Z",
                "2026-06-01T20:05:00Z",
                4.0,
            ),
            // Opens the group at 20:04 without joining it.
            rsession_kw(
                "leaves",
                "2026-06-01T20:01:00Z",
                "2026-06-01T20:04:00Z",
                1.0,
            ),
        ];
        let g = narrow_group(&sessions);
        assert_eq!(g.duration(), SESSION_BOUNDARY_RESOLUTION);
        assert_eq!(g.size(), 2);
        assert!(!g.is_dubious());
        assert_eq!(g.agg_avg_power_in(View::MinOverlap), 7.0);
    }

    /// A lone session ending inside the group is not in doubt either: it occupies some of the
    /// minute wherever its true end fell.
    #[test]
    fn a_lone_session_ending_in_the_group_is_not_dubious() {
        let sessions = vec![
            rsession_kw("ends", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 3.0),
            rsession_kw(
                "leaves",
                "2026-06-01T20:01:00Z",
                "2026-06-01T20:04:00Z",
                1.0,
            ),
        ];
        let g = narrow_group(&sessions);
        assert_eq!(g.size(), 1);
        assert!(!g.is_dubious());
    }

    /// Both boundaries of a narrow group can be made by sessions that are not its members — one
    /// leaving at its start, another arriving at its end. Its only member spans it throughout, so
    /// the group is narrow and in no doubt at all.
    #[test]
    fn a_group_whose_members_all_span_it_is_not_dubious() {
        let sessions = vec![
            rsession_kw("spans", "2026-06-01T20:01:00Z", "2026-06-01T20:10:00Z", 6.0),
            rsession_kw(
                "leaves",
                "2026-06-01T20:01:00Z",
                "2026-06-01T20:04:00Z",
                1.0,
            ),
            rsession_kw(
                "arrives",
                "2026-06-01T20:05:00Z",
                "2026-06-01T20:09:00Z",
                2.0,
            ),
        ];
        let g = narrow_group(&sessions);
        assert_eq!(g.duration(), SESSION_BOUNDARY_RESOLUTION);
        assert_eq!(g.size(), 1);
        assert!(!g.is_dubious());
        assert_eq!(g.agg_avg_power_in(View::MinOverlap), 6.0);
    }

    /// Sessions certainly present throughout are added to the best of the movable ones rather than
    /// competing with them — the unconditional term in the closed form.
    #[test]
    fn spanning_sessions_are_added_to_the_best_of_the_rest() {
        let sessions = vec![
            rsession_kw("spans", "2026-06-01T20:01:00Z", "2026-06-01T20:10:00Z", 6.0),
            rsession_kw("ends", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 3.0),
            rsession_kw(
                "starts",
                "2026-06-01T20:04:00Z",
                "2026-06-01T20:08:00Z",
                4.0,
            ),
        ];
        let g = narrow_group(&sessions);
        assert_eq!(g.size(), 3);
        assert!(g.is_dubious());
        // 6.0 unconditionally, plus the larger of the two that cannot be shown to coincide.
        assert_eq!(g.agg_avg_power_in(View::MinOverlap), 10.0);
        assert_eq!(g.size_in(View::MinOverlap), 2);
    }

    /// A session running through an edge of the interval of interest has its end-points clamped to
    /// that edge for the tiling, yet it is certainly present throughout. The buckets must therefore
    /// read its own reported times: reading the clamped ones would file it among the movable
    /// sessions and lose it from the unconditional term.
    #[test]
    fn a_session_running_through_the_interval_edge_still_counts_as_spanning() {
        let sessions = vec![
            rsession_kw(
                "through",
                "2026-06-01T19:00:00Z",
                "2026-06-01T21:00:00Z",
                6.0,
            ),
            rsession_kw("ends", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 3.0),
            rsession_kw(
                "starts",
                "2026-06-01T20:04:00Z",
                "2026-06-01T20:08:00Z",
                4.0,
            ),
        ];
        let g = narrow_group(&sessions);
        assert_eq!(g.size(), 3);
        assert_eq!(g.agg_avg_power_in(View::MinOverlap), 10.0);
        assert_eq!(g.size_in(View::MinOverlap), 2);
    }

    /// Two sessions confined to the group can be arranged one after the other, so only one of them
    /// is certainly drawing at any instant.
    #[test]
    fn two_sessions_confined_to_the_group_are_dubious() {
        let sessions = vec![
            rsession_kw(
                "inner_a",
                "2026-06-01T20:04:00Z",
                "2026-06-01T20:05:00Z",
                3.0,
            ),
            rsession_kw(
                "inner_b",
                "2026-06-01T20:04:00Z",
                "2026-06-01T20:05:00Z",
                4.0,
            ),
        ];
        let g = narrow_group(&sessions);
        assert_eq!(g.size(), 2);
        assert!(g.is_dubious());
        assert_eq!(g.size_in(View::MinOverlap), 1);
        assert_eq!(g.agg_avg_power_in(View::MinOverlap), 4.0);
    }

    /// Wider than one resolution, no arrangement of the true times can separate any pair. This
    /// arrangement is the one that fooled an earlier version of the bucket arithmetic, which
    /// applied it at every width: `a` lands in the ends-inside class and `b` in the confined one,
    /// yet `a` runs from before the group starts until at least `20:04` and `b` starts before
    /// `20:04`, so the two certainly overlap.
    #[test]
    fn groups_wider_than_one_resolution_are_never_dubious() {
        let sessions = vec![
            rsession_kw("a", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 3.0),
            rsession_kw("b", "2026-06-01T20:03:00Z", "2026-06-01T20:05:00Z", 4.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        let g = groups
            .iter()
            .find(|g| g.start() == ts("2026-06-01T20:03:00Z"))
            .expect("a group starting at 20:03");
        assert_eq!(g.duration(), 2 * SESSION_BOUNDARY_RESOLUTION);
        assert!(!g.is_dubious());
        assert_eq!(g.size_in(View::MinOverlap), 2);
        assert_eq!(g.agg_avg_power_in(View::MinOverlap), 7.0);
    }

    /// End-points are clamped to the real interval, so a reported peak window is a wall-clock
    /// window that can be matched against the metering data.
    #[test]
    fn group_endpoints_are_clamped_to_the_real_interval() {
        let sessions = vec![rsession(
            "covers",
            "2026-06-01T19:00:00Z",
            "2026-06-01T21:00:00Z",
        )];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].start(), ts(LO));
        assert_eq!(groups[0].end(), ts(HI));
        assert_eq!(groups[0].duration(), Duration::from_secs(15 * 60));
    }
}
