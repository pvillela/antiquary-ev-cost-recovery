use crate::{
    Anomaly, Bracket, Interval, RSession, SEGMENT_DURATION, Segment, Session, SessionReport,
    session_list,
};
use std::{
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

/// The four estimates for one [`Segment`].
///
/// Two derivations times two units. The energy-based pair reads the sessions' own consumption; the
/// count-based pair reads how many of them were charging against the per-EV rating of the
/// infrastructure. Each is a [`Bracket`], because the reported session times are stated only to the
/// minute and the overlap they imply is therefore a range rather than a number.
pub struct EstimateSet {
    pub energy_based_kw: Bracket<f64>,
    pub energy_based_kva: Bracket<f64>,
    pub count_based_kw: Bracket<f64>,
    pub count_based_kva: Bracket<f64>,
}

impl EstimateSet {
    /// The four figures, in the order the report tabulates them.
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

    // Spikes take part in the estimates on the same footing as any other session. A spike's raw
    // energy over charge time is infinite or NaN, either of which would swamp or poison any
    // segment it entered, and [`Session::avg_kw`] substitutes a finite figure for exactly that
    // reason — so nothing has to be done to a spike here. See README.md, "Other".
    let rsessions: Vec<RSession> = sessions
        .into_iter()
        .chain(spikes)
        .map(Rc::new)
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

/// The [`SEGMENT_DURATION`]-wide segments tiling `ioi`, each holding the sessions that intersect
/// it.
///
/// # Panics
///
/// If `ioi`'s duration is not a whole number of [`SEGMENT_DURATION`]s, or is zero. That is the
/// precondition [`SEGMENT_DURATION`] states, and it is checked rather than accommodated.
///
/// Rounding the segment count up would make the segments overrun the interval — a 20-minute
/// interval would tile to 20:00–20:15 and 20:15–20:30 — so a session charging only in the overrun
/// would be counted into the estimates despite falling outside the interval of interest entirely,
/// and could be reported as its peak. Rounding down would silently leave part of the interval
/// unestimated. Neither is a defensible answer to a question that should not have been asked, and
/// both are wrong in a way no figure in the report would reveal.
///
/// The legal interval lengths are 15 minutes and an hour, so nothing coming through
/// [`crate::checked_interval`] can reach this. The core stays permissive about *when* an interval
/// starts, which is what exploratory callers and tests rely on; it was never permissive about how
/// long one may be.
fn segments_for_ioi(ioi: Interval, sessions: &[RSession]) -> Vec<Segment> {
    let (ioi_secs, seg_secs) = (ioi.duration.as_secs(), SEGMENT_DURATION.as_secs());
    assert!(
        ioi_secs > 0 && ioi_secs % seg_secs == 0,
        "interval of interest is {ioi_secs}s, which is not a positive whole number of \
         {seg_secs}s segments; see SEGMENT_DURATION"
    );

    let nsegs = (ioi_secs / seg_secs) as usize;
    let mut segments = (0..nsegs)
        .map(|i| Segment::new(ioi.start + SEGMENT_DURATION * i as u32, SEGMENT_DURATION))
        .collect::<Vec<_>>();

    for s in sessions {
        for segment in segments.iter_mut() {
            if !s.intersects(&segment.interval) {
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

/// The segment maximizing `criterion`, with its estimates.
///
/// Seeded from the first segment's own criterion rather than from zero, so a segment is never
/// beaten by one that merely scores above zero — an empty interval's segments all score zero, and
/// the first of them is as much the maximum as any other.
///
/// Ties go to the earliest segment: the comparison is strict and the segments are visited in time
/// order, so a later segment has to *beat* the incumbent to displace it. That makes the choice
/// deterministic, which matters because a tie is not rare — every segment of an interval no session
/// reached is tied at the standing block.
pub(crate) fn maximal_segment_estimate(
    segments: &[Segment],
    criterion: impl Fn(&Segment) -> f64,
) -> (Segment, EstimateSet) {
    let mut seg_iter = segments.iter();
    let first = seg_iter
        .next()
        .expect("`segments` slice expected to be non-empty");
    let mut hi_crit = criterion(first);
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
        .flat_map(|s| {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
