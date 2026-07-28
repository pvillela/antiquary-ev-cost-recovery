use crate::{AnomalyKind, OVERLAP_THRESHOLD, RSession, Session, quicksort};
use jiff::Timestamp;
use std::{
    cell::{Ref, RefCell},
    cmp::Ordering,
    collections::{BTreeSet, btree_set::Iter},
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

#[derive(Debug, Clone)]
/// Collection of sessions that have non-trivial intersections with each other and with the interval
/// of interest.
pub struct SessionGroup {
    start: Timestamp,
    end: Timestamp,
    sessions: BTreeSet<RSession>,
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

    pub fn agg_avg_power(&self) -> f64 {
        self.sessions
            .iter()
            .map(|s| s.as_ref().borrow().avg_power)
            .sum()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
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
}

//  |------------------------------------|
//      |----------|
//          |---------------------|
//                     |------|
//       I============================I
//
//       |2-|3-----|2--|3-----|2--|1--|

/// Note this mutates `sessions`: those whose overlap with `interval` falls entirely within a
/// boundary margin have [`AnomalyKind::IntersectsBoundaryMarginOnly`] appended. Calling it twice
/// with the same sessions appends it twice.
pub fn groups_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &mut Vec<RSession>,
) -> Vec<SessionGroup> {
    let end_points = end_points_for_interval(interval, sessions, OVERLAP_THRESHOLD);
    end_points_to_groups(end_points)
}

/// Returns an unsorted list of end-points corresponding to the sessions that non-trivially intersect
/// `interval`.
///
/// A session takes part only if it is active somewhere in `interval` reduced by `overlap_threshold`
/// at each end: reported times are truncated to whole minutes, so an overlap confined to a boundary
/// margin cannot be trusted. The margin decides *membership* only — end-points are clamped to the
/// real interval, so the groups tile it and a reported peak window is a wall-clock window that can
/// be matched against the metering data. A session whose overlap falls entirely within a margin is
/// flagged [`AnomalyKind::IntersectsBoundaryMarginOnly`].
fn end_points_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &mut Vec<RSession>,
    overlap_threshold: Duration,
) -> Vec<EndPoint> {
    let lo = interval.0;
    let hi = interval.1;
    let lo_t = lo + overlap_threshold;
    let hi_t = hi - overlap_threshold;
    let mut end_points = Vec::<EndPoint>::new();
    for s in sessions {
        if s.as_ref().borrow().conn_start < hi_t && s.as_ref().borrow().conn_end >= lo_t {
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
        } else if s.as_ref().borrow().conn_start < hi && s.as_ref().borrow().conn_end >= lo {
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
fn end_points_to_groups(mut end_points: Vec<EndPoint>) -> Vec<SessionGroup> {
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
            groups.push(SessionGroup {
                start: run_start,
                end: time,
                sessions: active.clone(),
            });
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
    fn tiling(groups: &[SessionGroup]) -> Vec<(i64, i64, Vec<String>)> {
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
        assert_eq!(groups[1].session_count(), 2);
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
        assert!(groups.iter().all(|g| g.session_count() == 1));
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
        assert_eq!(groups[0].session_count(), 3);
        assert_eq!(groups[0].agg_avg_power(), 3.0);
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
