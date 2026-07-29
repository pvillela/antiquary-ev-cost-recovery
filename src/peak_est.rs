use crate::{
    Anomaly, EV_POWER_FACTOR, EVOLUTE_BREAKER_KVA_RATING, EVOLUTE_BREAKER_KW_RATING, GroupAnomaly,
    RSession, SessionGroup, SessionReport, groups_for_interval, session_list,
};
use jiff::Timestamp;
use std::{
    cell::RefCell,
    error::Error,
    path::{Path, PathBuf},
    rc::Rc,
};

/// One estimated figure, and the [`SessionGroup`] it was drawn from.
pub struct PowerEstimate {
    pub value: f64,
    /// Index of the group in [`PowerEstimatesReport::session_groups`]. Retained alongside the
    /// group itself because human-facing output wants to say "group 5".
    pub session_group_idx: usize,
    group: Rc<SessionGroup>,
}

impl PowerEstimate {
    /// The group this figure was drawn from.
    pub fn group(&self) -> &SessionGroup {
        &self.group
    }

    /// Anomalies carried by the group this figure was drawn from — most usefully
    /// [`GroupAnomaly::ClampedSessionGroup`], which says the estimate rests on a group the panel
    /// could not physically have run.
    pub fn group_anomalies(&self) -> Vec<GroupAnomaly> {
        self.group.anomalies()
    }
}

/// The four estimates, under one assumption about how many panels are installed.
pub struct EstimateSet {
    pub consumption_based_kw: PowerEstimate,
    pub consumption_based_kva: PowerEstimate,
    pub breaker_specs_based_kw: PowerEstimate,
    pub breaker_specs_based_kva: PowerEstimate,
}

/// The estimates for an interval of interest.
///
/// `direct` is computed from the session groups exactly as the report gives them, assuming no
/// constraint on how many sessions can run at once — which is what more than one panel would mean.
/// It is always present.
///
/// `clamped` assumes instead the single panel this software is written for, whose PLC will not run
/// more than [`crate::EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`] sessions at once, so a larger group
/// is cut down to that. It is `Some` only when some group actually exceeded the limit; otherwise
/// clamping is a no-op and the figures would merely repeat `direct`.
///
/// Testing *any* group is the same as testing the peak ones, so nothing turns on the choice: the
/// group `direct` draws `breaker_specs_based_*` from is by definition the largest, so it exceeds
/// the limit exactly when some group does. A clamped group that never peaks cannot arise.
///
/// What clamping can still do, when the peaking group is itself oversized, is *move* the peak —
/// lowering that group's figures may drop it below one that was never clamped. So `clamped` and
/// `direct` may cite different `session_group_idx`.
///
/// When both are present `clamped <= direct` throughout — the clamped figures sum over a subset
/// and count no more — so the pair nests, and `[clamped.consumption_based_kw,
/// direct.breaker_specs_based_kw]` is the widest honest bracket on the true peak. No report seen
/// so far produces a `clamped` at all; the June sample peaks at three concurrent sessions.
pub struct PowerEstimates {
    pub direct: EstimateSet,
    pub clamped: Option<EstimateSet>,
}

pub struct PowerEstimatesReport {
    /// Workbook the sessions were read from. Held so the report is self-describing: it can be
    /// stored or rendered later without a caller having to remember what produced it.
    pub source: PathBuf,
    /// Interval of interest the estimates cover, as given to
    /// [`max_power_estimates_for_interval`]. Held for the same reason, and so a renderer cannot be
    /// handed an interval the report was not computed for.
    pub interval: (Timestamp, Timestamp),
    pub estimates: Option<PowerEstimates>,
    pub session_groups: Vec<Rc<SessionGroup>>,
    /// Every anomaly carried by every session that intersects this interval, excluded ones
    /// included. Sessions elsewhere in the workbook are not reported: they say nothing about this
    /// estimate. Without this the caller has a number and no way to judge it: sessions dropped for
    /// [`crate::AnomalyKind::InconsistentDuration`] or
    /// [`crate::AnomalyKind::IntersectsBoundaryMarginOnly`] appear in no group, and a spike's
    /// substituted average power is invisible in the total it feeds.
    pub session_anomalies: Vec<Anomaly>,
}

/// Produces EV maximum power estimates for the interval of interest `interval` and the session
/// report at `path`.
pub fn max_power_estimates_for_interval(
    interval: (Timestamp, Timestamp),
    path: &Path,
) -> Result<PowerEstimatesReport, Box<dyn Error>> {
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
        s.avg_power = if s.energy_use == 0.0 {
            0.0
        } else {
            EVOLUTE_BREAKER_KW_RATING
        };
        s
    });

    // Combine sessions and spikes. Grouping algorithm will sort it out.
    let mut rsessions: Vec<_> = sessions
        .into_iter()
        .chain(spikes)
        .map(|s| Rc::new(RefCell::new(s)))
        .collect();
    let groups = groups_for_interval(interval, &mut rsessions);

    // After grouping, not before: `groups_for_interval` appends
    // `IntersectsBoundaryMarginOnly` to sessions it rejects, and those sessions belong to no group,
    // so `rsessions` is the only place they survive.
    let session_anomalies = collect_session_anomalies(interval, &rsessions, &excluded);

    let estimates = estimates_for_groups(&groups);
    Ok(PowerEstimatesReport {
        source: path.to_path_buf(),
        interval,
        estimates,
        session_groups: groups,
        session_anomalies,
    })
}

/// Assembles the estimates for an already-computed tiling, or `None` when no session reached the
/// interval and there are no groups.
///
/// `clamped` is produced only when some group was actually cut down to one panel's worth —
/// `clamped_size() < size()` is exactly that test. If it holds nowhere, clamping is a no-op and the
/// set would merely repeat `direct`.
///
/// Testing every group is equivalent to testing only the ones `direct` peaks at, because the
/// size-peak group is the largest there is: if *it* is within the limit then so is everything else.
/// The `any` form is preferred for keeping the invariant that a `clamped` set is present exactly
/// when some group carries [`GroupAnomaly::ClampedSessionGroup`].
pub(crate) fn estimates_for_groups(groups: &[Rc<SessionGroup>]) -> Option<PowerEstimates> {
    let any_clamped = groups.iter().any(|g| g.clamped_size() < g.size());
    estimate_set(groups, Clamping::Direct).map(|direct| PowerEstimates {
        direct,
        clamped: any_clamped
            .then(|| estimate_set(groups, Clamping::Clamped))
            .flatten(),
    })
}

/// Every anomaly on every session that touches `interval`, in report-row order.
///
/// Restricted to sessions intersecting the interval, because the workbook covers a whole billing
/// period while a report covers one window in it: a spike three weeks away says nothing about this
/// estimate and would only bury the findings that do. Intersection is the plain overlap test, with
/// no boundary margin — a session excluded by the margin is exactly one worth reporting.
///
/// Deliberately blind to [`crate::AnomalyKind`]: it matches on nothing, so a kind added later
/// surfaces here without anyone having to remember to wire it up.
fn collect_session_anomalies(
    interval: (Timestamp, Timestamp),
    rsessions: &[RSession],
    excluded: &[crate::Session],
) -> Vec<Anomaly> {
    let from_rsessions = rsessions.iter().flat_map(|rs| {
        let s = rs.as_ref().borrow();
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
    });
    let from_excluded = excluded
        .iter()
        .filter(|s| s.intersects(interval))
        .flat_map(|s| {
            s.anomalies.iter().map(|kind| Anomaly {
                row: s.row,
                session_id: s.id.clone(),
                kind: *kind,
            })
        });

    let mut anomalies: Vec<Anomaly> = from_rsessions.chain(from_excluded).collect();
    anomalies.sort_by(|a, b| a.row.cmp(&b.row).then_with(|| a.session_id.cmp(&b.session_id)));
    anomalies
}

/// Which view of an oversized group an [`EstimateSet`] is computed under.
#[derive(Clone, Copy)]
enum Clamping {
    /// One panel: a group above the PLC limit is cut down to it.
    Clamped,
    /// The groups as the report gives them, with no panel constraint applied.
    Direct,
}

impl Clamping {
    fn agg_avg_power(self, group: &SessionGroup) -> f64 {
        match self {
            Self::Clamped => group.clamped_agg_avg_power(),
            Self::Direct => group.agg_avg_power(),
        }
    }

    fn size(self, group: &SessionGroup) -> usize {
        match self {
            Self::Clamped => group.clamped_size(),
            Self::Direct => group.size(),
        }
    }
}

/// The four estimates for `groups` under one [`Clamping`], or `None` when there are no groups.
///
/// Each figure selects its own peak group *through the same `clamping`* it reports. Selecting by
/// one view and reporting the other would name a group that did not peak.
fn estimate_set(groups: &[Rc<SessionGroup>], clamping: Clamping) -> Option<EstimateSet> {
    let power_idx = max_group_by(groups, |g| clamping.agg_avg_power(g))?;
    let size_idx = max_group_by(groups, |g| clamping.size(g))?;

    let max_kw = clamping.agg_avg_power(&groups[power_idx]);
    let max_size = clamping.size(&groups[size_idx]) as f64;

    let at = |value: f64, idx: usize| PowerEstimate {
        value,
        session_group_idx: idx,
        group: groups[idx].clone(),
    };

    Some(EstimateSet {
        consumption_based_kw: at(max_kw, power_idx),
        consumption_based_kva: at(max_kw / EV_POWER_FACTOR, power_idx),
        breaker_specs_based_kw: at(max_size * EVOLUTE_BREAKER_KW_RATING, size_idx),
        breaker_specs_based_kva: at(max_size * EVOLUTE_BREAKER_KVA_RATING, size_idx),
    })
}

/// Returns the index of the [`SessionGroup`] in `groups` scoring highest on `metric`, or `None`
/// when there are no groups. Ties go to the earliest group, so the reported peak window is the
/// first moment the peak was reached.
fn max_group_by<T: PartialOrd>(
    groups: &[Rc<SessionGroup>],
    metric: impl Fn(&SessionGroup) -> T,
) -> Option<usize> {
    let mut best: Option<(usize, T)> = None;
    for (i, group) in groups.iter().enumerate() {
        let score = metric(group);
        let improves = match &best {
            Some((_, best_score)) => score > *best_score,
            None => true,
        };
        if improves {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS, RSession, Session};
    use std::time::Duration;

    /// Index of the group with the greatest aggregate average power.
    fn max_avg_power_group(groups: &[Rc<SessionGroup>]) -> Option<usize> {
        max_group_by(groups, |g| g.agg_avg_power())
    }

    /// Index of the group with the greatest session count.
    fn max_size_group(groups: &[Rc<SessionGroup>]) -> Option<usize> {
        max_group_by(groups, |g| g.size())
    }

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn rsession(id: &str, start: &str, end: &str, avg_power: f64) -> RSession {
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
            avg_power,
            anomalies: Vec::new(),
        }))
    }

    /// Five groups whose peak sits at index 3, with an earlier improvement at index 1.
    ///
    /// This is the shape that broke the previous implementation: it advanced its iterator past the
    /// first group and then *accumulated* `i + 1` on every improvement instead of assigning it, so
    /// two improvements compounded to `1 + 3 = 4`. That silently named the wrong group here, and
    /// ran off the end of the slice outright when the improvements fell later.
    #[test]
    fn peak_group_index_is_the_group_that_peaked() {
        let mut sessions = vec![
            rsession("A", "2026-06-01T20:01:00Z", "2026-06-01T20:59:00Z", 1.0),
            rsession("B", "2026-06-01T20:10:00Z", "2026-06-01T20:20:00Z", 2.0),
            rsession("C", "2026-06-01T20:40:00Z", "2026-06-01T20:50:00Z", 5.0),
        ];
        let groups = groups_for_interval(
            (ts("2026-06-01T20:00:00Z"), ts("2026-06-01T21:00:00Z")),
            &mut sessions,
        );

        let powers: Vec<f64> = groups.iter().map(|g| g.agg_avg_power()).collect();
        assert_eq!(powers, [1.0, 3.0, 1.0, 6.0, 1.0]);

        assert_eq!(max_avg_power_group(&groups), Some(3));
        // Counts are 1, 2, 1, 2, 1 — the maximum is reached twice and the earliest wins.
        assert_eq!(max_size_group(&groups), Some(1));
    }

    #[test]
    fn no_groups_yields_no_index() {
        assert_eq!(max_avg_power_group(&[]), None);
        assert_eq!(max_size_group(&[]), None);
    }

    #[test]
    fn no_groups_yields_no_estimates() {
        assert!(estimates_for_groups(&[]).is_none());
    }

    /// `n` sessions running concurrently over the whole interval, each drawing 1 kW, so one group
    /// of `n`.
    fn concurrent(n: usize) -> Vec<Rc<SessionGroup>> {
        let mut sessions: Vec<RSession> = (0..n)
            .map(|i| {
                rsession(
                    &format!("S{i:02}"),
                    "2026-06-01T20:05:00Z",
                    "2026-06-01T20:55:00Z",
                    1.0,
                )
            })
            .collect();
        groups_for_interval(
            (ts("2026-06-01T20:00:00Z"), ts("2026-06-01T21:00:00Z")),
            &mut sessions,
        )
    }

    /// Below the panel limit clamping would change nothing, so no clamped set is produced — the
    /// caller is not handed a second copy of `direct` to compare against.
    #[test]
    fn clamped_set_is_absent_when_no_group_exceeds_the_panel_limit() {
        for n in [1, 5, 10] {
            let groups = concurrent(n);
            let estimates = estimates_for_groups(&groups).expect("a group exists");
            assert_eq!(estimates.direct.consumption_based_kw.value, n as f64, "n={n}");
            assert!(estimates.clamped.is_none(), "n={n}");
        }
    }

    /// Above it, both sets are produced and the clamped one is strictly the lower — the bracket
    /// that says the report claims more concurrent sessions than one panel can run.
    #[test]
    fn clamped_set_appears_and_is_lower_when_a_group_exceeds_the_panel_limit() {
        for n in [11, 14] {
            let groups = concurrent(n);
            let estimates = estimates_for_groups(&groups).expect("a group exists");
            let direct = &estimates.direct;
            let clamped = estimates.clamped.as_ref().expect("a group was clamped");

            assert_eq!(direct.consumption_based_kw.value, n as f64, "n={n}");
            assert_eq!(
                clamped.consumption_based_kw.value,
                EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS as f64,
                "n={n}"
            );
            assert!(
                clamped.breaker_specs_based_kw.value < direct.breaker_specs_based_kw.value,
                "n={n}"
            );
            // The group keeps its real size, so the anomaly can carry it.
            assert_eq!(
                clamped.consumption_based_kw.group_anomalies(),
                vec![GroupAnomaly::ClampedSessionGroup(n)],
                "n={n}"
            );
        }
    }

    /// Clamping can move the peak, but only ever by cutting down the group that was peaking —
    /// lowering a group cannot promote it, and cannot promote anything else either.
    ///
    /// Group 0 holds eleven sessions at 1.0 kW, so it leads on the direct figures (11.0 kW, and
    /// the largest count) and is cut to 10.0 kW. Group 1 holds ten at 1.05 kW, is never clamped,
    /// and so takes the consumption peak once group 0 has been cut below it.
    #[test]
    fn clamping_can_move_the_peak_when_the_peaking_group_is_the_one_cut_down() {
        let mut sessions: Vec<RSession> = (0..11)
            .map(|i| {
                rsession(
                    &format!("X{i:02}"),
                    "2026-06-01T20:05:00Z",
                    "2026-06-01T20:20:00Z",
                    1.0,
                )
            })
            .chain((0..10).map(|i| {
                rsession(
                    &format!("Y{i:02}"),
                    "2026-06-01T20:30:00Z",
                    "2026-06-01T20:45:00Z",
                    1.05,
                )
            }))
            .collect();
        let groups = groups_for_interval(
            (ts("2026-06-01T20:00:00Z"), ts("2026-06-01T21:00:00Z")),
            &mut sessions,
        );
        assert_eq!(groups.len(), 2);

        let estimates = estimates_for_groups(&groups).expect("groups exist");
        let direct = &estimates.direct;
        let clamped = estimates.clamped.as_ref().expect("group 0 was clamped");

        // Direct: group 0 leads on both counts.
        assert_eq!(direct.consumption_based_kw.session_group_idx, 0);
        assert!((direct.consumption_based_kw.value - 11.0).abs() < 1e-9);
        assert_eq!(direct.breaker_specs_based_kw.session_group_idx, 0);

        // Clamped: group 0 falls to 10.0, so group 1 at 10.5 takes the consumption peak.
        assert_eq!(clamped.consumption_based_kw.session_group_idx, 1);
        assert!((clamped.consumption_based_kw.value - 10.5).abs() < 1e-9);
        // Both groups now count 10, and the tie goes to the earliest.
        assert_eq!(clamped.breaker_specs_based_kw.session_group_idx, 0);

        // Only the group that was cut down carries the anomaly.
        assert_eq!(
            groups[0].anomalies(),
            vec![GroupAnomaly::ClampedSessionGroup(11)]
        );
        assert!(groups[1].anomalies().is_empty());
    }
}
