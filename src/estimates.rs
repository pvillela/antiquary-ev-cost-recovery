use crate::{
    Anomaly, BREAKER_RATING_KW, Bracket, Interval, RSession, SEGMENT_DURATION, Segment, Session,
    SessionReport, session_list,
};
use std::{
    cell::RefCell,
    error::Error,
    path::{Path, PathBuf},
    rc::Rc,
};

/// Estimates for an interval of interest.
pub struct IntervalEstimates {
    /// Workbook the sessions were read from. Held so the report is self-describing: it can be
    /// stored or rendered later without a caller having to remember what produced it.
    pub source: PathBuf,
    /// Interval of interest.
    pub interval: Interval,
    /// All segments and their estimates.
    pub seg_estimates: Vec<(Segment, EstimateSet)>,
    /// Segment and estimate set that maximize the energy-based estimates.
    pub energy_based_seg_estimate: (Segment, EstimateSet),
    /// Segment and estimate set that maximize the count-based estimates.
    pub count_based_seg_estimate: (Segment, EstimateSet),
    /// Every anomaly carried by every session that intersects the interval of interest.
    /// Sessions excluded outright are *not* here; they are in
    /// [`Self::excluded_sessions`].
    pub session_anomalies: Vec<Anomaly>,
    /// Every session excluded from the estimates for
    /// [`crate::AnomalyKind::InconsistentDuration`] — the whole workbook's worth, not only those
    /// intersecting this interval.
    ///
    /// Unfiltered on purpose. Such a record's own fields contradict each other, so asking whether
    /// it intersects the interval is asking a question of the very timestamps that are in doubt.
    /// The report states which ones appear to touch the interval and lists the rest anyway,
    /// leaving the judgement to a reader who can go back to the source rows.
    pub excluded_sessions: Vec<Session>,
}

/// The four estimates, under one [`View`] of the session groups.
pub struct EstimateSet {
    pub energy_based_kw: Bracket<f64>,
    pub energy_based_kva: Bracket<f64>,
    pub count_based_kw: Bracket<f64>,
    pub count_based_kva: Bracket<f64>,
}

impl EstimateSet {
    /// The four figures, in the order the report tabulates them.
    ///
    /// Deduplication compares these exactly, floats and all, which is sound here rather than
    /// sloppy: a set that differs from another does so because a different subset of the *same*
    /// `avg_power` values was summed, and dropping a member that contributed 0.0 leaves the sum
    /// bit-identical. Nothing reaching a group is NaN — a spike's infinite average power is
    /// substituted before grouping.
    pub fn values(&self) -> [Bracket<f64>; 4] {
        [
            self.energy_based_kw,
            self.energy_based_kva,
            self.count_based_kw,
            self.count_based_kva,
        ]
    }
}

/// Produces EV maximum power estimates for the interval of interest `ioi` and the session
/// report at `path`.
pub fn interval_estimates(ioi: Interval, path: &Path) -> Result<IntervalEstimates, Box<dyn Error>> {
    let session_report = session_list(path)?;
    // `excluded` sessions contradict themselves and take no part in any estimate, but they are
    // still reported: a caller judging an estimate needs to know what was left out. See README.md,
    // "Other".
    let SessionReport {
        sessions,
        spikes,
        excluded,
    } = session_report;

    // A spike's own avg_power is infinite or NaN, either of which would swamp or poison every group
    // it entered, so the estimating logic substitutes a finite figure. See README.md, "Other".
    let spikes = spikes.into_iter().map(|mut s| {
        s.avg_kw = if s.energy_use == 0.0 {
            0.0
        } else {
            BREAKER_RATING_KW
        };
        s
    });

    // Combine sessions and spikes. Grouping algorithm will sort it out.
    let rsessions: Vec<_> = sessions
        .into_iter()
        .chain(spikes)
        .map(|s| Rc::new(RefCell::new(s)))
        .collect();
    let segments = segments_for_ioi(ioi, &rsessions);
    let seg_estimates: Vec<(Segment, EstimateSet)> = segments
        .iter()
        .map(|seg| (seg.clone(), segment_estimate(seg)))
        .collect();
    let energy_based_seg_estimate = maximal_segment_estimate(&segments, |seg| seg.agg_kw().mid());
    let count_based_seg_estimate = maximal_segment_estimate(&segments, |seg| seg.agg_count().mid());
    let session_anomalies = collect_session_anomalies(&ioi, &rsessions);

    Ok(IntervalEstimates {
        source: path.to_path_buf(),
        interval: ioi,
        seg_estimates,
        energy_based_seg_estimate,
        count_based_seg_estimate,
        session_anomalies,
        excluded_sessions: excluded,
    })
}

fn segments_for_ioi(ioi: Interval, sessions: &Vec<RSession>) -> Vec<Segment> {
    let Interval {
        start: ioi_start,
        duration: ioi_dur,
    } = ioi;

    let nsegs = ioi.duration.as_secs().div_ceil(SEGMENT_DURATION.as_secs()) as usize;
    let mut segments = (0..nsegs)
        .map(|i| Segment::new(ioi_start + ioi_dur * i as u32, SEGMENT_DURATION))
        .collect::<Vec<_>>();

    for s in sessions {
        let session = s.borrow();
        for segment in segments.iter_mut() {
            if !session.intersects(&segment.interval) {
                continue;
            }
            segment.add_session(s.clone());
        }
    }

    segments
}

pub(crate) fn segment_estimate(segment: &Segment) -> EstimateSet {
    let energy_based_load = segment.energy_based_load();
    let count_based_load = segment.count_based_load();
    EstimateSet {
        energy_based_kw: energy_based_load.map(|load| load.real_kw),
        energy_based_kva: energy_based_load.map(|load| load.apparent_kva()),
        count_based_kw: count_based_load.map(|load| load.real_kw),
        count_based_kva: count_based_load.map(|load| load.apparent_kva()),
    }
}

pub(crate) fn maximal_segment_estimate(
    segments: &[Segment],
    criterion: impl Fn(&Segment) -> f64,
) -> (Segment, EstimateSet) {
    let mut seg_iter = segments.iter();
    let first = seg_iter
        .next()
        .expect("`segments` slice expected to be non-empty");
    let mut hi_crit = 0.0;
    let mut hi_seg = first;
    let mut hi_est = segment_estimate(first);
    for segment in seg_iter {
        let crit = criterion(segment);
        if crit > hi_crit {
            hi_crit = crit;
            hi_seg = segment;
            hi_est = segment_estimate(segment);
        }
    }
    (hi_seg.clone(), hi_est)
}

/// Every anomaly on every session that intersects the interval of interest.
///
/// Deliberately blind to [`crate::AnomalyKind`]: it matches on nothing, so a kind added later
/// surfaces here without anyone having to remember to wire it up.
fn collect_session_anomalies(interval: &Interval, rsessions: &[RSession]) -> Vec<Anomaly> {
    let mut anomalies: Vec<Anomaly> = rsessions
        .iter()
        .flat_map(|rs| {
            let s = rs.borrow();
            if !s.intersects(interval) {
                return Vec::new();
            }
            s.anomalies
                .iter()
                .map(|kind| Anomaly {
                    row: s.row,
                    session_id: s.id.clone(),
                    kind: *kind,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    anomalies.sort_by(|a, b| {
        a.row
            .cmp(&b.row)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    anomalies
}
