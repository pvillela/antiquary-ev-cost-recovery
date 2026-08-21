//! Helpers shared by the unit tests of more than one module.
//!
//! Reachable from anywhere in the crate, not only from this directory: [`session`] is what the API
//! layer's tests build their inputs from, and a second definition of it there would be a second
//! place for a fixture to drift from what the readers actually produce.

use super::{AnomalyKind, RSession, Session};
use jiff::Timestamp;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// A session for tests: read from `path` at `row`, starting at `conn_start` (RFC 3339), lasting
/// `minutes`, and drawing `energy_use` kWh over that time.
///
/// Sound by construction, so nothing built here is excluded or held apart as a spike. The reported
/// start, end and duration agree exactly, which is well inside the band
/// [`duration_is_consistent`](super::duration_is_consistent) allows, and the charge time is
/// non-zero.
///
/// Note the padding when reasoning about which segment a session lands in: the adjusted end is one
/// [`TIME_GRID_STEP`](super::TIME_GRID_STEP) past the reported one, so a session of `minutes`
/// occupies `minutes + 1` on the timeline. A 14-minute session starting on a quarter hour is
/// therefore exactly one segment wide.
pub(crate) fn session(
    path: &str,
    row: usize,
    id: &str,
    conn_start: &str,
    minutes: i64,
    energy_use: f64,
) -> RSession {
    let conn_start: Timestamp = conn_start.parse().expect("an RFC 3339 timestamp");
    let elapsed = Duration::from_secs(minutes as u64 * 60);
    Rc::new(Session {
        path: Rc::new(PathBuf::from(path)),
        row,
        id: id.to_owned(),
        conn_start,
        conn_end: conn_start + elapsed,
        conn_duration: elapsed,
        charge_time: elapsed,
        energy_use,
        anomalies: Vec::new(),
    })
}

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
