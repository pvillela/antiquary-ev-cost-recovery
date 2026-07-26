use crate::SessionGroup;

pub const EV_POWER_FACTOR: f64 = 0.95;
pub const EVOLUTE_BREAKER_KW_RATING: f64 = 6.7;
pub const EVOLUTE_BREAKER_KVA_RATING: f64 = 7.5;

/// Estimate of EV charging peak (kW, kVA) in the interval that was used to compute `clusters`,
/// based on the clustering of sessions and the Evolute breaker specs.
pub fn max_kw_kva_over_clusters_based_on_breaker_specs<'a>(
    clusters: &'a [SessionGroup<'a>],
) -> (f64, f64) {
    let max_depth = max_depth_over_clusters(clusters) as f64;
    (
        max_depth * EVOLUTE_BREAKER_KW_RATING,
        max_depth * EVOLUTE_BREAKER_KVA_RATING,
    )
}

/// Estimate of EV charging peak (kW, kVA) in the interval that was used to compute `clusters`,
/// based on the clustering of sessions and their average power draw.
pub fn max_kw_kva_over_clusters_based_on_consumption<'a>(
    clusters: &'a [SessionGroup<'a>],
) -> (f64, f64) {
    let max_avg_kw = clusters
        .iter()
        .map(|c| c.sessions.iter().map(|s| s.avg_power).sum::<f64>())
        .max_by(|a, b| a.total_cmp(b))
        .unwrap_or(0.0);
    let kva = max_avg_kw / EV_POWER_FACTOR;
    (max_avg_kw, kva)
}

fn max_depth_over_clusters<'a>(clusters: &'a [SessionGroup<'a>]) -> usize {
    let max_depth = clusters.iter().map(|c| c.sessions.len()).max().unwrap_or(0);
    max_depth
}
