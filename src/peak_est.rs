use crate::{Session, SessionGroup, SessionReport, groups_for_interval, session_list};
use jiff::Timestamp;
use std::{error::Error, path::Path, rc::Rc};

pub const EV_POWER_FACTOR: f64 = 0.95;
pub const EVOLUTE_BREAKER_KW_RATING: f64 = 6.7;
pub const EVOLUTE_BREAKER_KVA_RATING: f64 = 7.5;

pub struct PowerEstimate {
    pub value: f64,
    pub session_group: SessionGroup,
}

pub struct PowerEstimates {
    pub consumption_based_kw: PowerEstimate,
    pub consumption_based_kva: PowerEstimate,
    pub breaker_specs_based_kw: PowerEstimate,
    pub breaker_specs_based_kva: PowerEstimate,
}

pub struct PowerEstimatesReport {
    pub estimates: PowerEstimates,
    pub spikes: Vec<Session>,
    pub warnings: Vec<String>,
}

/// Produces EV maximum power estimates for the interval of interest `interval` and the session
/// report at `path`.
pub fn max_power_estimates_for_interval(
    interval: (Timestamp, Timestamp),
    path: &Path,
) -> Result<PowerEstimatesReport, Box<dyn Error>> {
    let session_report = session_list(path)?;
    let SessionReport { sessions, spikes } = session_report;

    // Prepare warnings.
    let mut warnings = Vec::<String>::new();
    if !spikes.is_empty() {
        warnings.push("spikes need to be reviewed".to_owned());
    }
    let intersects = sessions
        .iter()
        .any(|s| s.conn_start < interval.1 && s.raw_conn_end > interval.0);
    if !intersects {
        warnings.push("interval does not intersect any session in the report".to_owned());
    }

    let session_refs: Vec<_> = sessions.into_iter().map(|s| Rc::new(s)).collect();
    let groups = groups_for_interval(interval, &session_refs);

    let (consumption_based_kw, consumption_based_kva) =
        max_kw_kva_over_groups_based_on_consumption(&groups);
    let (breaker_specs_based_kw, breaker_specs_based_kva) =
        max_kw_kva_over_groups_based_on_breaker_specs(&groups);

    let estimates = PowerEstimates {
        consumption_based_kw,
        consumption_based_kva,
        breaker_specs_based_kw,
        breaker_specs_based_kva,
    };

    Ok(PowerEstimatesReport {
        estimates,
        spikes,
        warnings,
    })
}

/// Estimate of EV charging peak (kW, kVA) in the interval that was used to compute `groups`,
/// based on the grouping of sessions and their average power draw.
fn max_kw_kva_over_groups_based_on_consumption(
    groups: &[SessionGroup],
) -> (PowerEstimate, PowerEstimate) {
    let mut group_iter = groups.iter();
    let max_group = if let Some(group0) = group_iter.next() {
        let mut max_group = group0;
        let mut max_kw = max_group.agg_avg_power();
        for group in group_iter {
            if group.agg_avg_power() > max_kw {
                max_group = group;
                max_kw = max_group.agg_avg_power();
            }
        }
        max_group.clone()
    } else {
        SessionGroup::default()
    };

    let max_kw = max_group.agg_avg_power();
    let kw_est = PowerEstimate {
        value: max_kw,
        session_group: max_group.clone(),
    };
    let kva_est = PowerEstimate {
        value: max_kw / EV_POWER_FACTOR,
        session_group: max_group,
    };
    (kw_est, kva_est)
}

/// Estimate of EV charging peak (kW, kVA) in the interval that was used to compute `groups`,
/// based on the grouping of sessions and the Evolute breaker specs.
fn max_kw_kva_over_groups_based_on_breaker_specs(
    groups: &[SessionGroup],
) -> (PowerEstimate, PowerEstimate) {
    let max_group = max_depth_group(groups);
    let max_depth = max_group.session_count();
    let kw_est = PowerEstimate {
        value: max_depth as f64 * EVOLUTE_BREAKER_KW_RATING,
        session_group: max_group.clone(),
    };
    let kva_est = PowerEstimate {
        value: max_depth as f64 * EVOLUTE_BREAKER_KVA_RATING,
        session_group: max_group,
    };
    (kw_est, kva_est)
}

/// Returns the [`SessionGroup`] in `groups` the with greatest session count.
fn max_depth_group(groups: &[SessionGroup]) -> SessionGroup {
    let mut group_iter = groups.iter();
    if let Some(group0) = group_iter.next() {
        let mut max_group = group0;
        let mut max_count = max_group.session_count();
        for group in group_iter {
            if group.session_count() > max_count {
                max_group = group;
                max_count = max_group.session_count();
            }
        }
        max_group.clone()
    } else {
        SessionGroup::default()
    }
}
