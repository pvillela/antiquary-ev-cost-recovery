use crate::{Anomaly, AnomalyKind, OVERLAP_THRESHOLD, RSession, Session, quicksort};
use jiff::Timestamp;
use std::{
    cell::{Ref, RefCell},
    collections::{BTreeSet, btree_set::Iter},
    time::Duration,
};

#[derive(Debug, PartialEq, PartialOrd)]
enum EndPoint {
    Left(EndPointData),
    Right(EndPointData),
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

impl PartialOrd for EndPointData {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.time.partial_cmp(&other.time) {
            Some(core::cmp::Ordering::Equal) => {}
            ord => return ord,
        }
        self.session
            .as_ref()
            .borrow()
            .id
            .partial_cmp(&other.session.as_ref().borrow().id)
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

impl Default for SessionGroup {
    fn default() -> Self {
        Self {
            start: Timestamp::MIN,
            end: Timestamp::MAX,
            sessions: BTreeSet::new(),
        }
    }
}

impl SessionGroup {
    fn new(data: EndPointData) -> Self {
        let mut sessions = BTreeSet::<RSession>::new();
        sessions.insert(data.session);
        Self {
            start: data.time,
            end: Timestamp::MAX,
            sessions,
        }
    }

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

    pub fn duration(&self) -> Duration {
        Duration::try_from(self.end - self.start).expect("span should be at most a few hours")
    }

    fn remove_sessions(&mut self, remove_list: &mut Vec<RSession>) -> Self {
        let old_group = self.clone();
        remove_list.iter().for_each(|s| {
            self.sessions.remove(s);
        });
        self.end = Timestamp::MAX;
        old_group
    }
}

#[derive(Default)]
struct GroupState {
    groups: Vec<SessionGroup>,
    curr_group: SessionGroup,
    remove_list: Vec<RSession>,
}

impl GroupState {
    fn process_left_edge(&mut self, data: EndPointData) {
        let group = &mut self.curr_group;
        if group.sessions.is_empty() {
            self.curr_group = SessionGroup::new(data);
        } else {
            let old_group = group.remove_sessions(&mut self.remove_list);
            self.groups.push(old_group);
            if group.start == data.time {
                group.sessions.insert(data.session);
            } else {
                let mut old_group = group.clone();
                old_group.end = data.time;
                self.groups.push(old_group);
                group.start = data.time;
                group.end = Timestamp::MAX;
                group.sessions.insert(data.session);
            }
        }
    }

    fn process_right_edge(&mut self, data: EndPointData) {
        let group = &mut self.curr_group;
        if group.sessions.is_empty() {
            panic!("illegal state");
        } else {
            if group.end == Timestamp::MAX {
                group.end = data.time
            }

            if group.end == data.time {
                self.remove_list.push(data.session);
            } else {
                let old_group = group.remove_sessions(&mut self.remove_list);
                group.start = old_group.end;
                group.end = data.time;
                self.groups.push(old_group);
            }
        }
    }
}

//  |------------------------------------|
//      |----------|
//          |---------------------|
//                     |------|
//       I============================I
//
//       |2-|3-----|2--|3-----|2--|1--|

pub fn groups_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &mut Vec<RSession>,
) -> Vec<SessionGroup> {
    let end_points = end_points_for_interval(interval, sessions, OVERLAP_THRESHOLD);
    end_points_to_groups(end_points)
}

/// Returns an unsorted list of end-points corresponding to the sessions that non-trivially intersect
/// `interval`.
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
                time: s_mut.conn_start.max(lo_t),
                session: s.clone(),
            });
            let right = EndPoint::Right(EndPointData {
                time: s_mut.conn_end.min(hi_t),
                session: s.clone(),
            });
            end_points.push(left);
            end_points.push(right);

            // Anomaly checking for included sessions.
            if s_mut.charge_time.is_zero() {
                let row = s_mut.row;
                let session_id = s_mut.id.clone();
                s_mut.anomalies.push(Anomaly {
                    row,
                    session_id,
                    kind: AnomalyKind::ZeroActiveChargeTime,
                });
            }
        } else if s.as_ref().borrow().conn_start < hi_t && s.as_ref().borrow().conn_end >= lo_t {
            let s_mut = &mut s.borrow_mut();
            let row = s_mut.row;
            let session_id = s_mut.id.clone();
            s_mut.anomalies.push(Anomaly {
                row,
                session_id,
                kind: AnomalyKind::IntersectsBelowThreshold,
            });
        }
    }
    end_points
}

fn end_points_to_groups(mut end_points: Vec<EndPoint>) -> Vec<SessionGroup> {
    quicksort(&mut end_points);

    let mut state = GroupState::default();
    for end_point in end_points {
        match end_point {
            EndPoint::Left(data) => state.process_left_edge(data),
            EndPoint::Right(data) => state.process_right_edge(data),
        }
    }
    state.groups
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
}
