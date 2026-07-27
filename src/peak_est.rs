use crate::{SessionGroup, SessionReport, groups_for_interval, session_list};
use jiff::Timestamp;
use std::{error::Error, path::Path};

pub const EV_POWER_FACTOR: f64 = 0.95;
pub const EVOLUTE_BREAKER_KW_RATING: f64 = 6.7;
pub const EVOLUTE_BREAKER_KVA_RATING: f64 = 7.5;

pub struct PowerEstimate<'a> {
    value: f64,
    session_group: SessionGroup<'a>,
}

pub struct PowerEstimates<'a> {
    pub consumption_based_kw: PowerEstimate<'a>,
    pub consumption_based_kva: PowerEstimate<'a>,
    pub breaker_specs_based_kw: PowerEstimate<'a>,
    pub breaker_specs_based_kva: PowerEstimate<'a>,
}

pub struct PowerEstimatesReport<'a> {
    pub estimates: PowerEstimates<'a>,
    pub session_report: SessionReport,
}

// pub fn max_power_estimates_for_interval<'a>(
//     interval: (Timestamp, Timestamp),
//     path: &Path,
// ) -> Result<PowerEstimatesReport<'a>, Box<dyn Error>> {
//     let session_report = session_list(path)?;
//     let sessions = session_report.sessions;
//     let groups = groups_for_interval(interval, &sessions);

//     let (consumption_based_kw, consumption_based_kva) =
//         max_kw_kva_over_groups_based_on_consumption(&groups);
//     let (breaker_specs_based_kw, breaker_specs_based_kva) =
//         max_kw_kva_over_groups_based_on_breaker_specs(&groups);

//     let estimates = PowerEstimates {
//         consumption_based_kw,
//         consumption_based_kva,
//         breaker_specs_based_kw,
//         breaker_specs_based_kva,
//     };

//     Ok(PowerEstimatesReport {
//         estimates,
//         session_report,
//     })
// }

/// Estimate of EV charging peak (kW, kVA) in the interval that was used to compute `groups`,
/// based on the grouping of sessions and their average power draw.
fn max_kw_kva_over_groups_based_on_consumption<'a>(
    groups: &'a [SessionGroup<'a>],
) -> (PowerEstimate<'a>, PowerEstimate<'a>) {
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
fn max_kw_kva_over_groups_based_on_breaker_specs<'a>(
    groups: &'a [SessionGroup<'a>],
) -> (PowerEstimate<'a>, PowerEstimate<'a>) {
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
fn max_depth_group<'a>(groups: &'a [SessionGroup<'a>]) -> SessionGroup<'a> {
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
