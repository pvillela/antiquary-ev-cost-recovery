use crate::{
    EV_POWER_FACTOR, EVOLUTE_BREAKER_KVA_RATING, EVOLUTE_BREAKER_KW_RATING, SessionGroup,
    SessionReport, groups_for_interval, session_list,
};
use jiff::Timestamp;
use std::{cell::RefCell, error::Error, path::Path, rc::Rc};

pub struct PowerEstimate {
    pub value: f64,
    pub session_group_idx: usize,
}

pub struct PowerEstimates {
    pub consumption_based_kw: PowerEstimate,
    pub consumption_based_kva: PowerEstimate,
    pub breaker_specs_based_kw: PowerEstimate,
    pub breaker_specs_based_kva: PowerEstimate,
}

pub struct PowerEstimatesReport {
    pub estimates: Option<PowerEstimates>,
    pub session_groups: Vec<SessionGroup>,
}

/// Produces EV maximum power estimates for the interval of interest `interval` and the session
/// report at `path`.
pub fn max_power_estimates_for_interval(
    interval: (Timestamp, Timestamp),
    path: &Path,
) -> Result<PowerEstimatesReport, Box<dyn Error>> {
    let session_report = session_list(path)?;
    // `excluded` sessions contradict themselves and take no part in any estimate. See README.md,
    // "Other".
    let SessionReport {
        sessions,
        spikes,
        excluded: _,
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

    let opt_consumption = max_kw_kva_over_groups_based_on_consumption(&groups);
    let opt_specs = max_kw_kva_over_groups_based_on_breaker_specs(&groups);
    let estimates = match (opt_consumption, opt_specs) {
        (None, _) => None,
        (
            Some((consumption_based_kw, consumption_based_kva)),
            Some((breaker_specs_based_kw, breaker_specs_based_kva)),
        ) => {
            let estimates = PowerEstimates {
                consumption_based_kw,
                consumption_based_kva,
                breaker_specs_based_kw,
                breaker_specs_based_kva,
            };
            Some(estimates)
        }
        _ => unreachable!(
            "because either there is some session that intersects the interval or there are none"
        ),
    };
    Ok(PowerEstimatesReport {
        estimates,
        session_groups: groups,
    })
}

/// Estimate of EV charging peak (kW, kVA) in the interval that was used to compute `groups`,
/// based on the grouping of sessions and their average power draw.
fn max_kw_kva_over_groups_based_on_consumption(
    groups: &[SessionGroup],
) -> Option<(PowerEstimate, PowerEstimate)> {
    let idx_max = max_avg_power_group(groups);
    if let Some(idx_max) = idx_max {
        let max_group = &groups[idx_max];
        let max_kw = max_group.agg_avg_power();
        let kw_est = PowerEstimate {
            value: max_kw,
            session_group_idx: idx_max,
        };
        let kva_est = PowerEstimate {
            value: max_kw / EV_POWER_FACTOR,
            session_group_idx: idx_max,
        };
        Some((kw_est, kva_est))
    } else {
        None
    }
}

/// Estimate of EV charging peak (kW, kVA) in the interval that was used to compute `groups`,
/// based on the grouping of sessions and the Evolute breaker specs.
fn max_kw_kva_over_groups_based_on_breaker_specs(
    groups: &[SessionGroup],
) -> Option<(PowerEstimate, PowerEstimate)> {
    let idx_max = max_depth_group(groups);
    if let Some(idx_max) = idx_max {
        let max_group = &groups[idx_max];
        let max_depth = max_group.session_count();
        let kw_est = PowerEstimate {
            value: max_depth as f64 * EVOLUTE_BREAKER_KW_RATING,
            session_group_idx: idx_max,
        };
        let kva_est = PowerEstimate {
            value: max_depth as f64 * EVOLUTE_BREAKER_KVA_RATING,
            session_group_idx: idx_max,
        };
        Some((kw_est, kva_est))
    } else {
        None
    }
}

/// Returns the index of the [`SessionGroup`] in `groups` scoring highest on `metric`, or `None`
/// when there are no groups. Ties go to the earliest group, so the reported peak window is the
/// first moment the peak was reached.
fn max_group_by<T: PartialOrd>(
    groups: &[SessionGroup],
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

/// Returns the index of the [`SessionGroup`] in `groups` the with greatest aggregate average power.
fn max_avg_power_group(groups: &[SessionGroup]) -> Option<usize> {
    max_group_by(groups, SessionGroup::agg_avg_power)
}

/// Returns the index of the [`SessionGroup`] in `groups` the with greatest session count.
fn max_depth_group(groups: &[SessionGroup]) -> Option<usize> {
    max_group_by(groups, SessionGroup::session_count)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{RSession, Session};
    use std::time::Duration;

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

        let powers: Vec<f64> = groups.iter().map(SessionGroup::agg_avg_power).collect();
        assert_eq!(powers, [1.0, 3.0, 1.0, 6.0, 1.0]);

        assert_eq!(max_avg_power_group(&groups), Some(3));
        // Counts are 1, 2, 1, 2, 1 — the maximum is reached twice and the earliest wins.
        assert_eq!(max_depth_group(&groups), Some(1));
    }

    #[test]
    fn no_groups_yields_no_index() {
        assert_eq!(max_avg_power_group(&[]), None);
        assert_eq!(max_depth_group(&[]), None);
    }
}
