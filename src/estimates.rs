use crate::{
    Anomaly, RSession, Session, SessionGroup, SessionReport, View, ev_real_power_kw,
    groups_for_interval, session_list,
};
use jiff::{SignedDuration, Timestamp};
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
    pub evems_specs_based_kw: PowerEstimate,
    pub evems_specs_based_kva: PowerEstimate,
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
            self.evems_specs_based_kw.value,
            self.evems_specs_based_kva.value,
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

impl PowerEstimates {
    /// The set to read as this window's *minimum*: `min_overlap` where it was given, `nominal`
    /// otherwise — which is exactly what its absence means, the two readings having come out equal.
    ///
    /// Lets a comparison between two windows put like against like without a case analysis at every
    /// call site.
    pub fn min_reading(&self) -> &EstimateSet {
        self.min_overlap.as_ref().unwrap_or(&self.nominal)
    }
}

/// The three windows the estimates cover: the interval of interest, and a *skew margin* interval
/// immediately before and after it.
///
/// The margins answer for the two clocks nothing reconciles. If the meter's clock leads or lags
/// Evolute's by `δ`, the interval really at issue is `I` shifted by `δ`; every such window lies
/// inside these three taken together, and the peak over a union is the highest of the peaks over its
/// parts. See README.md, "Clock skew and drift".
#[derive(Debug, Clone, Copy)]
pub struct WindowSpans {
    pub left_margin: (Timestamp, Timestamp),
    pub interest: (Timestamp, Timestamp),
    pub right_margin: (Timestamp, Timestamp),
}

impl WindowSpans {
    pub fn around(interest: (Timestamp, Timestamp)) -> Self {
        // Signed, because the timestamp arithmetic is; `CLOCK_SKEW_MARGIN` is a whole number of
        // seconds by construction, being a multiple of `SESSION_BOUNDARY_RESOLUTION`.
        let m = SignedDuration::from_secs(CLOCK_SKEW_MARGIN.as_secs() as i64);
        // Saturating rather than checked: at the ends of representable time a margin is clipped to
        // an empty window rather than taking the whole report down. The `Err` arm cannot arise —
        // jiff returns one only for a `Span` carrying units above hours, which needs a reference
        // date to interpret; a `SignedDuration` never does.
        let shift = |ts: Timestamp, d: SignedDuration| {
            ts.saturating_add(d)
                .expect("a fixed duration needs no reference date")
        };
        Self {
            left_margin: (shift(interest.0, -m), interest.0),
            interest,
            right_margin: (interest.1, shift(interest.1, m)),
        }
    }

    /// The whole span the estimates reach over, margins included.
    pub fn full(&self) -> (Timestamp, Timestamp) {
        (self.left_margin.0, self.right_margin.1)
    }
}

/// Which of the [`WindowSpans`] something falls in. More than one at a time, for anything longer
/// than a margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Windows {
    pub left_margin: bool,
    pub interest: bool,
    pub right_margin: bool,
}

impl Windows {
    pub fn of_session(session: &Session, spans: &WindowSpans) -> Self {
        Self {
            left_margin: session.intersects(spans.left_margin),
            interest: session.intersects(spans.interest),
            right_margin: session.intersects(spans.right_margin),
        }
    }

    pub fn any(self) -> bool {
        self.left_margin || self.interest || self.right_margin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginSide {
    Left,
    Right,
}

/// A skew margin interval that earned a place in the report, with its own estimates and its own
/// tiling.
///
/// Only margins that could raise the estimate are kept — see [`raises_estimate`] — so `estimates` is
/// not optional here: a margin no session reached has nothing to exceed anything with, and never
/// gets this far.
///
/// The tiling is the margin's own, and so are the `session_group_idx` values in its estimates. A
/// group number means "group *n* of this window", as it always has.
pub struct SkewMargin {
    pub side: MarginSide,
    pub interval: (Timestamp, Timestamp),
    pub estimates: PowerEstimates,
    pub session_groups: Vec<Rc<SessionGroup>>,
}

/// An anomaly, with the windows the session carrying it reaches.
///
/// The window is reported because the anomalies now span more than one: a figure drawn from a skew
/// margin can rest on a session that never touches the interval of interest, and a reader has to be
/// able to tell which is which.
#[derive(Debug)]
pub struct WindowedAnomaly {
    pub anomaly: Anomaly,
    pub windows: Windows,
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
    /// The skew margins that could raise the estimate, in time order. Empty in the ordinary case,
    /// where neither margin beats the interval of interest on any figure.
    ///
    /// Margins that could not are dropped here rather than hidden by the renderer, so there is one
    /// place that decides what the report covers.
    pub skew_margins: Vec<SkewMargin>,
    /// Every anomaly carried by every session that took part in any of the [`WindowSpans`], each
    /// marked with the windows its session reaches. Sessions elsewhere in the workbook are not
    /// reported: they say nothing about this estimate. Without this the caller has a number and no
    /// way to judge it — a spike's substituted average power, for one, is invisible in the total it
    /// feeds.
    ///
    /// Sessions excluded outright are *not* here; they are in
    /// [`PowerEstimatesReport::excluded_sessions`], which is reported in full and separately.
    pub session_anomalies: Vec<WindowedAnomaly>,
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
        s.avg_kw = if s.energy_use == 0.0 {
            0.0
        } else {
            ev_real_power_kw()
        };
        s
    });

    // Combine sessions and spikes. Grouping algorithm will sort it out.
    let rsessions: Vec<_> = sessions
        .into_iter()
        .chain(spikes)
        .map(|s| Rc::new(RefCell::new(s)))
        .collect();
    let spans = WindowSpans::around(interval);
    let groups = groups_for_interval(interval, &rsessions);
    let session_anomalies = collect_session_anomalies(&spans, &rsessions);

    let estimates = estimates_for_groups(&groups);
    // Each margin goes through the same path as the interval of interest, deliberately. At the
    // current `CLOCK_SKEW_MARGIN` a margin spans one grid cell and so holds exactly one group,
    // which invites a shortcut; taking it would make raising `CLOCK_SKEW_BOUND` past
    // `SESSION_BOUNDARY_RESOLUTION` a code change rather than a change to one constant.
    let skew_margins = [
        (MarginSide::Left, spans.left_margin),
        (MarginSide::Right, spans.right_margin),
    ]
    .into_iter()
    .filter_map(|(side, margin_interval)| {
        let session_groups = groups_for_interval(margin_interval, &rsessions);
        let margin = estimates_for_groups(&session_groups)?;
        raises_estimate(&margin, estimates.as_ref()).then_some(SkewMargin {
            side,
            interval: margin_interval,
            estimates: margin,
            session_groups,
        })
    })
    .collect();

    Ok(PowerEstimatesReport {
        source: path.to_path_buf(),
        interval,
        estimates,
        session_groups: groups,
        skew_margins,
        session_anomalies,
        excluded_sessions: excluded,
    })
}

/// Whether a skew margin could raise the estimate, and so earns a place in the report.
///
/// True when any of the margin's four figures, in either reading, exceeds the corresponding figure
/// for the interval of interest in that same reading. Readings are compared like with like: a
/// margin's `nominal` against `I`'s `nominal`, its minimum against `I`'s minimum.
///
/// The minimum clause is not redundant. `I`'s own `nominal` can be inflated by a dubious group while
/// its minimum sits well below a margin's, and that margin is worth seeing — it raises the floor of
/// the bracket even though it does not touch the ceiling.
///
/// A margin is compared against nothing when no session reached `I` at all, and any margin that
/// reached a session then qualifies: it is the only thing the report has to say.
fn raises_estimate(margin: &PowerEstimates, interest: Option<&PowerEstimates>) -> bool {
    let Some(interest) = interest else {
        return true;
    };
    let beats = |margin: &EstimateSet, interest: &EstimateSet| {
        margin
            .values()
            .into_iter()
            .zip(interest.values())
            .any(|(m, i)| m > i)
    };
    beats(&margin.nominal, &interest.nominal) || beats(margin.min_reading(), interest.min_reading())
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

/// Every anomaly on every session that touches any of `spans`, in report-row order, each marked
/// with the windows its session reaches.
///
/// Restricted to sessions intersecting those windows, because the workbook covers a whole billing
/// period while a report covers a little over one interval of it: a spike three weeks away says
/// nothing about this estimate and would only bury the findings that do. The radius is the whole
/// span rather than the interval of interest alone, because a figure drawn from a skew margin can
/// rest on a session that never touches `I`, and a reported figure whose anomalies go unmentioned is
/// the situation this list exists to prevent.
///
/// That restriction is safe here and not for [`PowerEstimatesReport::excluded_sessions`], because
/// these sessions' timestamps are the ones the grouping already trusted.
///
/// Deliberately blind to [`crate::AnomalyKind`]: it matches on nothing, so a kind added later
/// surfaces here without anyone having to remember to wire it up.
fn collect_session_anomalies(spans: &WindowSpans, rsessions: &[RSession]) -> Vec<WindowedAnomaly> {
    let mut anomalies: Vec<WindowedAnomaly> = rsessions
        .iter()
        .flat_map(|rs| {
            let s = rs.as_ref().borrow();
            let windows = Windows::of_session(&s, spans);
            if !windows.any() {
                return Vec::new();
            }
            s.anomalies
                .iter()
                .map(|kind| WindowedAnomaly {
                    anomaly: Anomaly {
                        row: s.row,
                        session_id: s.id.clone(),
                        kind: *kind,
                    },
                    windows,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    anomalies.sort_by(|a, b| {
        a.anomaly
            .row
            .cmp(&b.anomaly.row)
            .then_with(|| a.anomaly.session_id.cmp(&b.anomaly.session_id))
    });
    anomalies
}

/// The four estimates for `groups` under one [`View`], or `None` when there are no groups.
///
/// When there are no groups, i.e., no sessions intersecting the interval of interest,
/// the EV charging infrastructure still impacts the overall building's peak kW and kVA,
/// but the impact is small (currently ~ 0.35 kW and 1.54 kVA) and not reported.
///
/// Each estimating basis selects its own peak group.
fn estimate_set(groups: &[Rc<SessionGroup>], view: View) -> Option<EstimateSet> {
    let power_basis_idx = max_group_by(groups, |g| g.agg_avg_power_in(view))?;
    let size_basis_idx = max_group_by(groups, |g| g.size_in(view))?;

    let power_basis_metrics = groups[power_basis_idx].metrics(view);
    let size_basis_metrics = groups[size_basis_idx].metrics(view);

    let estimate = |value: f64, idx: usize| PowerEstimate {
        value,
        session_group_idx: idx,
        group: groups[idx].clone(),
    };

    Some(EstimateSet {
        consumption_based_kw: estimate(
            power_basis_metrics.power_basis_site_load().real_kw,
            power_basis_idx,
        ),
        consumption_based_kva: estimate(
            power_basis_metrics.power_basis_site_load().apparent_kva(),
            power_basis_idx,
        ),
        evems_specs_based_kw: estimate(
            size_basis_metrics.size_basis_site_load().real_kw,
            size_basis_idx,
        ),
        evems_specs_based_kva: estimate(
            size_basis_metrics.size_basis_site_load().apparent_kva(),
            size_basis_idx,
        ),
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
            avg_kw: avg_power,
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
        // TODO: Fix or remove assertion below.
        // assert_eq!(min.evems_specs_based_kw.value, EVOLUTE_BREAKER_KW_RATING);
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
