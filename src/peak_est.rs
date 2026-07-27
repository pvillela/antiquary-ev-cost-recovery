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
    let SessionReport { sessions, spikes } = session_report;

    // Combine sessions and spikes. Grouping algorithm will sort it out.
    let mut rsessions: Vec<_> = sessions
        .into_iter()
        .chain(spikes.into_iter())
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

/// Returns the index of the [`SessionGroup`] in `groups` the with greatest aggregate average power.
fn max_avg_power_group(groups: &[SessionGroup]) -> Option<usize> {
    let mut group_iter = groups.iter();
    if let Some(group0) = group_iter.next() {
        let mut max_kw = group0.agg_avg_power();
        let mut idx_max = 0;
        for (i, group) in group_iter.enumerate() {
            if group.agg_avg_power() > max_kw {
                idx_max += i + 1;
                max_kw = group.agg_avg_power();
            }
        }
        Some(idx_max)
    } else {
        None
    }
}

/// Returns the index of the [`SessionGroup`] in `groups` the with greatest session count.
fn max_depth_group(groups: &[SessionGroup]) -> Option<usize> {
    let mut group_iter = groups.iter();
    if let Some(group0) = group_iter.next() {
        let mut max_count = group0.session_count();
        let mut idx_max = 0;
        for (i, group) in group_iter.enumerate() {
            if group.session_count() > max_count {
                idx_max += i + 1;
                max_count = group.session_count();
            }
        }
        Some(idx_max)
    } else {
        None
    }
}
