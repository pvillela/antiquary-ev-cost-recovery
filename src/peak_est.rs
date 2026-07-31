use crate::{
    Anomaly, EV_POWER_FACTOR, EVOLUTE_BREAKER_KVA_RATING, EVOLUTE_BREAKER_KW_RATING, RSession,
    Session, SessionGroup, SessionReport, View, groups_for_interval, session_list,
};
use jiff::Timestamp;
use std::{
    cell::RefCell,
    cmp::Ordering,
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
}

/// The four estimates, under one [`View`] of the session groups.
pub struct EstimateSet {
    pub consumption_based_kw: PowerEstimate,
    pub consumption_based_kva: PowerEstimate,
    pub breaker_specs_based_kw: PowerEstimate,
    pub breaker_specs_based_kva: PowerEstimate,
}

impl EstimateSet {
    /// The four figures, in the order the report tabulates them.
    ///
    /// Deduplication compares these exactly, floats and all, which is sound here rather than
    /// sloppy: a set that differs from another does so because a different subset of the *same*
    /// `avg_power` values was summed, and dropping a member that contributed 0.0 leaves the sum
    /// bit-identical. Nothing reaching a group is NaN — a spike's infinite average power is
    /// substituted before grouping.
    pub fn values(&self) -> [f64; 4] {
        [
            self.consumption_based_kw.value,
            self.consumption_based_kva.value,
            self.breaker_specs_based_kw.value,
            self.breaker_specs_based_kva.value,
        ]
    }
}

/// The estimates for an interval of interest, under both [`View`]s of the session groups.
///
/// `nominal` takes every group's membership at face value. It is always present, and it is
/// deliberately the over-inclusive reading — understating a maximum is the unsafe error, so the
/// headline figure is drawn from everything that could have contributed and the doubt is reported
/// beside it.
///
/// `min_overlap` assumes instead that the members of each [dubious](SessionGroup::is_dubious) group
/// overlapped as little as their reported times allow. It is `Some` only when its four figures
/// differ from `nominal`'s: a dubious group that carries no peak changes no reported number, and a
/// report never shows the same four figures twice.
///
/// **`min_overlap <= nominal` on all four figures, unconditionally.** Each is a sum over a subset
/// of the same members and counts no more of them, per group, and a maximum across groups preserves
/// that. The bracket the report states therefore runs from one to the other, with no case analysis.
///
/// The two may still name *different* groups: lowering a dubious group can hand the peak to one
/// that was never in doubt, so each figure carries its own `session_group_idx`. Where two groups
/// tie, [`max_group_by`] names the one whose figure is certain.
pub struct PowerEstimates {
    pub nominal: EstimateSet,
    pub min_overlap: Option<EstimateSet>,
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
    /// Every anomaly carried by every session that took part in this interval's groups. Sessions
    /// elsewhere in the workbook are not reported: they say nothing about this estimate. Without
    /// this the caller has a number and no way to judge it — a spike's substituted average power,
    /// for one, is invisible in the total it feeds.
    ///
    /// Sessions excluded outright are *not* here; they are in
    /// [`PowerEstimatesReport::excluded_sessions`], which is reported in full and separately.
    pub session_anomalies: Vec<Anomaly>,
    /// Every session excluded from the estimates for
    /// [`crate::AnomalyKind::InconsistentDuration`] — the whole workbook's worth, not only those
    /// near this interval.
    ///
    /// Unfiltered on purpose. Such a record's own fields contradict each other, so asking whether
    /// it intersects the interval is asking a question of the very timestamps that are in doubt: a
    /// session that belongs in this window may well test as falling outside it. The report states
    /// which ones appear to touch the interval and lists the rest anyway, leaving the judgement to
    /// a reader who can go back to the source rows.
    pub excluded_sessions: Vec<Session>,
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
    let rsessions: Vec<_> = sessions
        .into_iter()
        .chain(spikes)
        .map(|s| Rc::new(RefCell::new(s)))
        .collect();
    let groups = groups_for_interval(interval, &rsessions);
    let session_anomalies = collect_session_anomalies(interval, &rsessions);

    let estimates = estimates_for_groups(&groups);
    Ok(PowerEstimatesReport {
        source: path.to_path_buf(),
        interval,
        estimates,
        session_groups: groups,
        session_anomalies,
        excluded_sessions: excluded,
    })
}

/// Assembles the estimates for an already-computed tiling, or `None` when no session reached the
/// interval and there are no groups.
///
/// `min_overlap` is gated on its figures rather than structurally. A dubious group may sit in a
/// tiling without carrying either peak, in which case the second set repeats the first exactly and
/// a second table would only invite the reader to hunt for a difference that is not there. The
/// group table still marks the dubious group, so nothing about it goes unreported.
pub(crate) fn estimates_for_groups(groups: &[Rc<SessionGroup>]) -> Option<PowerEstimates> {
    let nominal = estimate_set(groups, View::Nominal)?;
    let min_overlap =
        estimate_set(groups, View::MinOverlap).filter(|set| set.values() != nominal.values());

    Some(PowerEstimates {
        nominal,
        min_overlap,
    })
}

/// Every anomaly on every session that touches `interval`, in report-row order.
///
/// Restricted to sessions intersecting the interval, because the workbook covers a whole billing
/// period while a report covers one window in it: a spike three weeks away says nothing about this
/// estimate and would only bury the findings that do. That restriction is safe here and not for
/// [`PowerEstimatesReport::excluded_sessions`], because these sessions' timestamps are the ones the
/// grouping already trusted.
///
/// Deliberately blind to [`crate::AnomalyKind`]: it matches on nothing, so a kind added later
/// surfaces here without anyone having to remember to wire it up.
fn collect_session_anomalies(
    interval: (Timestamp, Timestamp),
    rsessions: &[RSession],
) -> Vec<Anomaly> {
    let mut anomalies: Vec<Anomaly> = rsessions
        .iter()
        .flat_map(|rs| {
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
        })
        .collect();
    anomalies.sort_by(|a, b| {
        a.row
            .cmp(&b.row)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    anomalies
}

/// The four estimates for `groups` under one [`View`], or `None` when there are no groups.
///
/// Each figure selects its own peak group *through the same `view`* it reports. Selecting by one
/// view and reporting another would name a group that did not peak.
fn estimate_set(groups: &[Rc<SessionGroup>], view: View) -> Option<EstimateSet> {
    let power_idx = max_group_by(groups, |g| g.agg_avg_power_in(view))?;
    let size_idx = max_group_by(groups, |g| g.size_in(view))?;

    let max_kw = groups[power_idx].agg_avg_power_in(view);
    let max_size = groups[size_idx].size_in(view) as f64;

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
/// when there are no groups.
///
/// Ties are broken twice over: first towards a group that is not
/// [dubious](SessionGroup::is_dubious), so that a figure two groups reach alike is attributed to
/// the one that reached it beyond doubt; then towards the earlier group, so the reported peak
/// window is the first moment the peak was reached.
///
/// The first tie-break only ever moves *which* group is named, never the figure, since it applies
/// at equal scores. One consequence is worth knowing: a figure whose peak group is dubious is one
/// no other group tied, so every other group scores strictly lower and `min_overlap` comes out
/// strictly lower too. A dubious group carrying a peak therefore all but guarantees a second
/// estimate set — the exception being a group whose movable members all draw zero power, where the
/// sizes differ but the aggregates do not.
fn max_group_by<T: PartialOrd>(
    groups: &[Rc<SessionGroup>],
    metric: impl Fn(&SessionGroup) -> T,
) -> Option<usize> {
    let mut best: Option<(usize, T)> = None;
    for (i, group) in groups.iter().enumerate() {
        let score = metric(group);
        let improves = match &best {
            Some((best_idx, best_score)) => match score.partial_cmp(best_score) {
                Some(Ordering::Greater) => true,
                Some(Ordering::Equal) => groups[*best_idx].is_dubious() && !group.is_dubious(),
                _ => false,
            },
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
    use crate::{RSession, Session};
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

    /// The interval the estimate-set tests work in.
    const LO: &str = "2026-06-01T20:00:00Z";
    const HI: &str = "2026-06-01T21:00:00Z";

    /// A dubious group carrying the peak: the second set says something the first does not, so it
    /// is given. `E` is reported as ending in the same minute `S` is reported as starting, so the
    /// group they share at `[20:04, 20:05)` may hold either both of them or one at a time.
    #[test]
    fn min_overlap_is_given_when_a_dubious_group_peaks() {
        let sessions = vec![
            rsession("E", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 5.0),
            rsession("S", "2026-06-01T20:04:00Z", "2026-06-01T20:30:00Z", 4.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        let peak = groups
            .iter()
            .find(|g| g.start() == ts("2026-06-01T20:04:00Z"))
            .expect("the shared group");
        assert!(peak.is_dubious());

        let est = estimates_for_groups(&groups).expect("groups exist");
        assert_eq!(est.nominal.consumption_based_kw.value, 9.0);
        let min = est.min_overlap.expect("the figures differ, so it is given");
        assert_eq!(min.consumption_based_kw.value, 5.0);
        assert_eq!(min.breaker_specs_based_kw.value, EVOLUTE_BREAKER_KW_RATING);
    }

    /// A dubious group away from every peak changes no figure, so no second table is produced —
    /// the case a structural gate would get wrong.
    ///
    /// Both peaks have to be moved elsewhere for this, the size peak included: a dubious pair is
    /// two concurrent sessions, so it carries the size peak unless some group is larger. Hence the
    /// three-session block, which also outdraws it.
    #[test]
    fn min_overlap_is_withheld_when_the_dubious_group_never_peaks() {
        let sessions = vec![
            rsession("E", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 1.0),
            rsession("S", "2026-06-01T20:04:00Z", "2026-06-01T20:08:00Z", 1.0),
            rsession("A1", "2026-06-01T20:10:00Z", "2026-06-01T20:50:00Z", 5.0),
            rsession("A2", "2026-06-01T20:10:00Z", "2026-06-01T20:50:00Z", 2.0),
            rsession("A3", "2026-06-01T20:10:00Z", "2026-06-01T20:50:00Z", 1.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        assert!(
            groups.iter().any(|g| g.is_dubious()),
            "one group is dubious"
        );

        let est = estimates_for_groups(&groups).expect("groups exist");
        assert_eq!(est.nominal.consumption_based_kw.value, 8.0);
        assert!(est.min_overlap.is_none());
    }

    /// Lowering a dubious group can hand the peak to one that was never in doubt, so the two sets
    /// name different groups. `E`+`S` reach 9.0 together but only 5.0 apart, while `A` runs at 6.0
    /// throughout with nothing uncertain about it.
    #[test]
    fn the_peak_can_move_between_the_two_sets() {
        let sessions = vec![
            rsession("E", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 5.0),
            rsession("S", "2026-06-01T20:04:00Z", "2026-06-01T20:06:00Z", 4.0),
            rsession("A", "2026-06-01T20:20:00Z", "2026-06-01T20:50:00Z", 6.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        let est = estimates_for_groups(&groups).expect("groups exist");
        let min = est.min_overlap.expect("the figures differ");

        let dubious_idx = est.nominal.consumption_based_kw.session_group_idx;
        assert!(groups[dubious_idx].is_dubious());
        assert_eq!(est.nominal.consumption_based_kw.value, 9.0);

        let certain_idx = min.consumption_based_kw.session_group_idx;
        assert!(!groups[certain_idx].is_dubious());
        assert_eq!(min.consumption_based_kw.value, 6.0);
        assert_ne!(dubious_idx, certain_idx);
    }

    /// Where two groups reach the same figure, the one that reached it beyond doubt is named. `A`
    /// alone draws exactly what `E` and `S` draw together, so the consumption peak is a tie between
    /// a dubious group and a certain one.
    #[test]
    fn a_tie_on_a_figure_goes_to_the_group_that_is_not_dubious() {
        let sessions = vec![
            rsession("E", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 5.0),
            rsession("S", "2026-06-01T20:04:00Z", "2026-06-01T20:06:00Z", 4.0),
            rsession("A", "2026-06-01T20:20:00Z", "2026-06-01T20:50:00Z", 9.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        let est = estimates_for_groups(&groups).expect("groups exist");

        let idx = est.nominal.consumption_based_kw.session_group_idx;
        assert_eq!(est.nominal.consumption_based_kw.value, 9.0);
        assert!(
            !groups[idx].is_dubious(),
            "the tie should be attributed to the group that is certain"
        );
        // The dubious group is earlier, so earliest-wins alone would have named it.
        assert!(
            groups.iter().take(idx).any(|g| g.is_dubious()),
            "an earlier dubious group ties this figure"
        );
    }

    /// The tie-break moves which group is named, never the figure: a dubious group that scores
    /// strictly higher is still the peak.
    #[test]
    fn a_dubious_group_still_wins_when_it_scores_higher() {
        let sessions = vec![
            rsession("E", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 5.0),
            rsession("S", "2026-06-01T20:04:00Z", "2026-06-01T20:06:00Z", 4.0),
            rsession("A", "2026-06-01T20:20:00Z", "2026-06-01T20:50:00Z", 8.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        let est = estimates_for_groups(&groups).expect("groups exist");

        let idx = est.nominal.consumption_based_kw.session_group_idx;
        assert_eq!(est.nominal.consumption_based_kw.value, 9.0);
        assert!(groups[idx].is_dubious());
    }

    /// `min_overlap <= nominal` on all four figures, whatever the arrangement.
    #[test]
    fn min_overlap_never_exceeds_nominal() {
        let sessions = vec![
            rsession("E", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 5.0),
            rsession("S", "2026-06-01T20:04:00Z", "2026-06-01T20:06:00Z", 4.0),
            rsession("A", "2026-06-01T20:03:00Z", "2026-06-01T20:50:00Z", 6.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        let est = estimates_for_groups(&groups).expect("groups exist");
        if let Some(min) = &est.min_overlap {
            for (m, n) in min.values().iter().zip(est.nominal.values()) {
                assert!(*m <= n, "{m} <= {n}");
            }
        }
    }
}
