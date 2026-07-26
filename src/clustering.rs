use crate::quicksort;
use jiff::{Timestamp, Zoned, tz::TimeZone};
use std::{collections::BTreeSet, time::Duration};

pub const BREAKER_MAX_KW: f64 = 6.7;
pub const BREAKER_MAX_KVA: f64 = 7.5;

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
pub enum EndPoint<'a> {
    Left(EndPointData<'a>),
    Right(EndPointData<'a>),
}

#[derive(Debug, Clone)]
pub struct EndPointData<'a> {
    time: Timestamp,
    session: &'a Session,
}

impl<'a> PartialEq for EndPointData<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.session.id == other.session.id
    }
}

impl<'a> PartialOrd for EndPointData<'a> {
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
pub struct Cluster<'a> {
    start: Timestamp,
    end: Timestamp,
    sessions: BTreeSet<&'a Session>,
}

impl<'a> Default for Cluster<'a> {
    fn default() -> Self {
        Self {
            start: Timestamp::MIN,
            end: Timestamp::MAX,
            sessions: Default::default(),
        }
    }
}

impl<'a> Cluster<'a> {
    fn new(data: EndPointData<'a>) -> Self {
        let mut sessions = BTreeSet::new();
        sessions.insert(data.session);
        Self {
            start: data.time,
            end: Timestamp::MAX,
            sessions,
        }
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

    fn remove_sessions(&mut self, remove_list: &mut Vec<&Session>) -> Self {
        let old_cluster = self.clone();
        remove_list.iter().for_each(|s| {
            self.sessions.remove(*s);
        });
        self.end = Timestamp::MAX;
        old_cluster
    }
}

#[derive(Default)]
struct ClusterState<'a> {
    clusters: Vec<Cluster<'a>>,
    curr_cluster: Cluster<'a>,
    remove_list: Vec<&'a Session>,
}

impl<'a> ClusterState<'a> {
    fn process_left_edge(&mut self, data: EndPointData<'a>) {
        let cluster = &mut self.curr_cluster;
        if cluster.sessions.is_empty() {
            self.curr_cluster = Cluster::new(data);
        } else {
            let old_cluster = cluster.remove_sessions(&mut self.remove_list);
            self.clusters.push(old_cluster);
            if cluster.start == data.time {
                cluster.sessions.insert(data.session);
            } else {
                let mut old_cluster = cluster.clone();
                old_cluster.end = data.time;
                self.clusters.push(old_cluster);
                cluster.start = data.time;
                cluster.end = Timestamp::MAX;
                cluster.sessions.insert(data.session);
            }
        }
    }

    fn process_right_edge(&mut self, data: EndPointData<'a>) {
        let cluster = &mut self.curr_cluster;
        if cluster.sessions.is_empty() {
            panic!("illegal state");
        } else {
            if cluster.end == Timestamp::MAX {
                cluster.end = data.time
            }

            if cluster.end == data.time {
                self.remove_list.push(data.session);
            } else {
                let old_cluster = cluster.remove_sessions(&mut self.remove_list);
                cluster.start = old_cluster.end;
                cluster.end = data.time;
                self.clusters.push(old_cluster);
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

pub fn clusters_for_interval<'a>(
    interval: (Timestamp, Timestamp),
    sessions: &'a [Session],
) -> Vec<Cluster<'a>> {
    let end_points = end_points_for_interval(interval, sessions, OVERLAP_THRESHOLD);
    end_points_to_clusters(end_points)
}

/// Returns an unsorted list of end-points corresponding to the sessions that non-trivially intersect
/// `interval`.
fn end_points_for_interval<'a>(
    interval: (Timestamp, Timestamp),
    sessions: &'a [Session],
    overlap_threshold: Duration,
) -> Vec<EndPoint<'a>> {
    let lo = interval.0 + overlap_threshold;
    let hi = interval.1 - overlap_threshold;
    let mut end_points = Vec::<EndPoint<'a>>::new();
    for session in sessions {
        if session.conn_start < hi && session.conn_end >= lo {
            let left = EndPoint::Left(EndPointData {
                time: session.conn_start.max(lo),
                session,
            });
            let right = EndPoint::Right(EndPointData {
                time: session.conn_end.min(hi),
                session,
            });
            end_points.push(left);
            end_points.push(right);
        }
    }
    end_points
}

fn end_points_to_clusters<'a>(mut end_points: Vec<EndPoint<'a>>) -> Vec<Cluster<'a>> {
    quicksort(&mut end_points);

    let mut state = ClusterState::default();
    for end_point in end_points {
        match end_point {
            EndPoint::Left(data) => state.process_left_edge(data),
            EndPoint::Right(data) => state.process_right_edge(data),
        }
    }
    state.clusters
}

#[cfg(test)]
// cargo test --package ev-peak-contrib --lib --all-features -- peak_contrib::test --nocapture
mod test {
    use crate::{EndPoint, Session, clustering::EndPointData};

    #[test]
    fn test_end_point_kind_order() {
        let session = Session {
            id: Default::default(),
            conn_start: Default::default(),
            raw_conn_end: Default::default(),
            conn_end: Default::default(),
            charge_time: Default::default(),
            energy_use: Default::default(),
            avg_power: Default::default(),
        };
        let data1 = EndPointData {
            time: Default::default(),
            session: &session,
        };
        let data2 = EndPointData {
            time: Default::default(),
            session: &session,
        };
        assert!(EndPoint::Left(data1) < EndPoint::Right(data2));
    }
}
