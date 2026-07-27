use crate::quicksort;
use jiff::{Timestamp, Zoned, tz::TimeZone};
use std::{
    collections::{BTreeSet, btree_set::Iter},
    rc::Rc,
    time::Duration,
};

/// Sessions whose overlap with the interval of interest is less than or equal to this
/// are excluded from the calculations.
pub const OVERLAP_THRESHOLD: Duration = Duration::from_secs(60);

fn time_zone() -> TimeZone {
    TimeZone::get("America/Toronto").expect("America/Toronto should be a valid time-zone name")
}

#[derive(Debug)]
/// Charging session
pub struct Session {
    /// From `session report`.
    pub id: String,
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

#[derive(Debug, PartialEq, PartialOrd)]
enum EndPoint {
    Left(EndPointData),
    Right(EndPointData),
}

#[derive(Debug, Clone)]
struct EndPointData {
    time: Timestamp,
    session: Rc<Session>,
}

impl PartialEq for EndPointData {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.session.id == other.session.id
    }
}

impl PartialOrd for EndPointData {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.time.partial_cmp(&other.time) {
            Some(core::cmp::Ordering::Equal) => {}
            ord => return ord,
        }
        self.session.id.partial_cmp(&other.session.id)
    }
}

#[derive(Debug, Clone)]
/// Collection of sessions that have non-trivial intersections with each other and with the interval
/// of interest.
pub struct SessionGroup {
    start: Timestamp,
    end: Timestamp,
    sessions: BTreeSet<Rc<Session>>,
}

pub struct SessionIter<'a>(Iter<'a, Rc<Session>>);

impl<'a> Iterator for SessionIter<'a> {
    type Item = &'a Session;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0.next() {
            None => None,
            Some(rc) => Some(rc.as_ref()),
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
        let mut sessions = BTreeSet::<Rc<Session>>::new();
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
        self.sessions.iter().map(|sess| sess.avg_power).sum()
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn duration(&self) -> Duration {
        Duration::try_from(self.end - self.start).expect("span should be at most a few hours")
    }

    fn remove_sessions(&mut self, remove_list: &mut Vec<Rc<Session>>) -> Self {
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
    remove_list: Vec<Rc<Session>>,
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
    sessions: &[Rc<Session>],
) -> Vec<SessionGroup> {
    let end_points = end_points_for_interval(interval, sessions, OVERLAP_THRESHOLD);
    end_points_to_groups(end_points)
}

/// Returns an unsorted list of end-points corresponding to the sessions that non-trivially intersect
/// `interval`.
fn end_points_for_interval(
    interval: (Timestamp, Timestamp),
    sessions: &[Rc<Session>],
    overlap_threshold: Duration,
) -> Vec<EndPoint> {
    let lo = interval.0 + overlap_threshold;
    let hi = interval.1 - overlap_threshold;
    let mut end_points = Vec::<EndPoint>::new();
    for session in sessions {
        if session.conn_start < hi && session.conn_end >= lo {
            let left = EndPoint::Left(EndPointData {
                time: session.conn_start.max(lo),
                session: session.clone(),
            });
            let right = EndPoint::Right(EndPointData {
                time: session.conn_end.min(hi),
                session: session.clone(),
            });
            end_points.push(left);
            end_points.push(right);
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

    #[test]
    fn test_end_point_kind_order() {
        let session = Rc::new(Session {
            id: Default::default(),
            conn_start: Default::default(),
            raw_conn_end: Default::default(),
            conn_end: Default::default(),
            charge_time: Default::default(),
            energy_use: Default::default(),
            avg_power: Default::default(),
        });
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
