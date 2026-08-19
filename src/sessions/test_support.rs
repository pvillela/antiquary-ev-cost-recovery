//! Helpers shared by the unit tests of more than one module in this directory.

use super::AnomalyKind;

/// A row's anomalies with [`AnomalyKind::ExcessiveAvgKw`] removed.
///
/// Nearly every test in [`super::csv`] and [`super::excel`] is about *timestamps* — DST
/// resolution, the `adj_conn_end` padding, the consistency band — and each fixture states an
/// `Energy_Use` and an `Active_Charge_Time` as fixed text. Whether the average power those imply
/// clears `BREAKER_RATING_KW` therefore depends on the value of `BREAKER_RATING_A`, and no test
/// may depend on that: lower the breaker rating and a dozen tests about the DST fold would start
/// failing over a flag that has nothing to do with what they check.
///
/// Filtering the one power-dependent kind out is what keeps them testing what they are named
/// for. `ExcessiveAvgKw` is checked where it belongs — against the rating rather than
/// against a number — in `tests/segment_tiling.rs`.
pub(crate) fn timing_anomalies(anomalies: &[AnomalyKind]) -> Vec<AnomalyKind> {
    anomalies
        .iter()
        .copied()
        .filter(|k| *k != AnomalyKind::ExcessiveAvgKw)
        .collect()
}

/// The same filter applied to a workbook's `anomalies` cell, read back through the wire format.
///
/// Going through [`AnomalyKind::from_token`] rather than comparing the cell text also checks
/// that what was written is what can be read back, which is the property the column exists for.
pub(crate) fn timing_anomalies_in_cell(cell: &str) -> Vec<AnomalyKind> {
    let kinds: Vec<AnomalyKind> = cell
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| AnomalyKind::from_token(t).unwrap_or_else(|| panic!("unreadable token {t:?}")))
        .collect();
    timing_anomalies(&kinds)
}
