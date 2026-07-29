use crate::{
    Anomaly, EV_POWER_FACTOR, EVOLUTE_BREAKER_KVA_RATING, EVOLUTE_BREAKER_KW_RATING, GroupAnomaly,
    RSession, Session, SessionGroup, SessionReport, View, groups_for_interval, session_list,
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

/// The estimates for an interval of interest, over the 2×2 of [`crate::Boundary`] and
/// [`crate::Clamping`].
///
/// `direct` is the tiling exactly as reported: every session that might have been running, and no
/// constraint on how many can run at once. It is always present, and it is deliberately the
/// over-inclusive corner — understating a maximum is the unsafe error, so the headline figure is
/// drawn from everything that could have contributed and the doubt is reported beside it.
///
/// `clamped` assumes instead the single panel this software is written for, whose PLC will not run
/// more than [`crate::EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`] sessions at once, so a larger group
/// is cut down to that. It is `Some` exactly when some group was actually cut down — equivalently,
/// when some group carries [`GroupAnomaly::ClampedSessionGroup`], which is the invariant the
/// report's asterisk rests on. The two formulations coincide: the group `direct` draws
/// `breaker_specs_based_*` from is by definition the largest, so if it is oversized its clamped
/// size falls to the limit and the figures differ, and if it is not, no group anywhere is.
///
/// `direct_narrow` and `clamped_narrow` drop the sessions flagged
/// [`crate::AnomalyKind::IntersectsBoundaryMarginOnly`], whose overlap with the interval is not
/// provable. Each is `Some` only when its four figures differ from every set printed before it, so
/// no report ever shows the same numbers twice.
///
/// Clamping can *move* a peak as well as lower it — cutting the peaking group down may drop it
/// below one that was never clamped — so two sets may cite different `session_group_idx`.
///
/// **The four do not nest.** `clamped <= direct` and `direct_narrow <= direct` both hold, since
/// each sums over a subset and counts no more. But `clamped_narrow` can *exceed* `clamped`: see
/// [`SessionGroup::eligible_sessions`] for why narrowing a group can raise its clamped total. Any
/// bracket must therefore be an actual min and max over whichever sets are present, never a corner
/// picked in advance. No report seen so far produces a `clamped` at all; the June sample peaks at
/// three concurrent sessions.
pub struct PowerEstimates {
    pub direct: EstimateSet,
    pub direct_narrow: Option<EstimateSet>,
    pub clamped: Option<EstimateSet>,
    pub clamped_narrow: Option<EstimateSet>,
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
    /// this the caller has a number and no way to judge it — a session flagged
    /// [`crate::AnomalyKind::IntersectsBoundaryMarginOnly`] is counted in `direct` though it may
    /// never have been running, and a spike's substituted average power is invisible in the total
    /// it feeds.
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
    let mut rsessions: Vec<_> = sessions
        .into_iter()
        .chain(spikes)
        .map(|s| Rc::new(RefCell::new(s)))
        .collect();
    let groups = groups_for_interval(interval, &mut rsessions);

    // After grouping, not before: `groups_for_interval` appends `IntersectsBoundaryMarginOnly` to
    // the sessions whose overlap it could not establish, and that flag is half of what makes this
    // list worth reading.
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
/// `clamped` is gated structurally: it is produced only when some group was actually cut down to
/// one panel's worth, which `size_in(CLAMPED) < size()` tests exactly. Testing every group is
/// equivalent to testing only the ones `direct` peaks at, because the size-peak group is the
/// largest there is — but the `any` form is preferred, because it keeps the invariant that a
/// `clamped` set is present exactly when some group carries
/// [`GroupAnomaly::ClampedSessionGroup`], which is what the report's asterisk promises.
///
/// The narrow sets are gated on their figures instead. There is no structural test for them: a
/// flagged session may sit in a group that never peaks, in which case narrowing changes no reported
/// number and a second table would only repeat the first. Each is kept only if it differs from
/// every set already destined for the report, in print order, so no two tables can coincide.
pub(crate) fn estimates_for_groups(groups: &[Rc<SessionGroup>]) -> Option<PowerEstimates> {
    let direct = estimate_set(groups, View::DIRECT)?;

    let clamped = groups
        .iter()
        .any(|g| g.size_in(View::CLAMPED) < g.size())
        .then(|| estimate_set(groups, View::CLAMPED))
        .flatten();

    let mut shown = vec![direct.values()];
    shown.extend(clamped.as_ref().map(EstimateSet::values));

    let mut keep = |set: Option<EstimateSet>| -> Option<EstimateSet> {
        let set = set?;
        if shown.contains(&set.values()) {
            return None;
        }
        shown.push(set.values());
        Some(set)
    };

    Some(PowerEstimates {
        direct_narrow: keep(estimate_set(groups, View::DIRECT_NARROW)),
        clamped_narrow: keep(estimate_set(groups, View::CLAMPED_NARROW)),
        direct,
        clamped,
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

    /// The interval the boundary-axis tests work in. `M` below ends exactly one
    /// `SESSION_BOUNDARY_RESOLUTION` into it, so its true end may be `20:00:00` itself and its
    /// overlap is unprovable — the flag `Boundary::Narrow` reads.
    const LO: &str = "2026-06-01T20:00:00Z";
    const HI: &str = "2026-06-01T21:00:00Z";

    /// A flagged session in the peaking group: the narrow set says something different, so it is
    /// given. `clamped_narrow` would repeat it exactly, nothing being oversized, so it is not.
    #[test]
    fn narrow_set_is_given_when_a_doubtful_session_peaks() {
        let mut sessions = vec![
            rsession("M", "2026-06-01T19:50:00Z", "2026-06-01T20:01:00Z", 5.0),
            rsession("A", "2026-06-01T19:55:00Z", "2026-06-01T20:30:00Z", 1.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
        let est = estimates_for_groups(&groups).expect("groups exist");

        // Both sessions run over the first group; only `A` survives narrowing.
        assert_eq!(groups[0].size(), 2);
        assert_eq!(groups[0].size_in(View::DIRECT_NARROW), 1);

        assert_eq!(est.direct.consumption_based_kw.value, 6.0);
        let narrow = est
            .direct_narrow
            .expect("the figures differ, so it is given");
        assert_eq!(narrow.consumption_based_kw.value, 1.0);
        assert_eq!(
            narrow.breaker_specs_based_kw.value,
            EVOLUTE_BREAKER_KW_RATING
        );

        assert!(est.clamped.is_none(), "nothing is oversized");
        assert!(
            est.clamped_narrow.is_none(),
            "it would only repeat `direct_narrow`"
        );
    }

    /// A flagged session away from every peak changes no figure, so no second table is produced —
    /// the case a structural gate would get wrong. Its group narrows to nothing at all, which is
    /// reported honestly as zero and simply never wins a maximum.
    #[test]
    fn narrow_set_is_withheld_when_the_doubtful_session_never_peaks() {
        let mut sessions = vec![
            rsession("M", "2026-06-01T19:50:00Z", "2026-06-01T20:01:00Z", 1.0),
            rsession("A", "2026-06-01T20:10:00Z", "2026-06-01T20:50:00Z", 5.0),
        ];
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);

        assert_eq!(groups[0].size(), 1);
        assert_eq!(groups[0].size_in(View::DIRECT_NARROW), 0);
        assert_eq!(groups[0].agg_avg_power_in(View::DIRECT_NARROW), 0.0);

        let est = estimates_for_groups(&groups).expect("groups exist");
        assert_eq!(est.direct.consumption_based_kw.value, 5.0);
        assert!(est.direct_narrow.is_none());
        assert!(est.clamped_narrow.is_none());
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
            assert_eq!(
                estimates.direct.consumption_based_kw.value, n as f64,
                "n={n}"
            );
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
                clamped.consumption_based_kw.value, EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS as f64,
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
