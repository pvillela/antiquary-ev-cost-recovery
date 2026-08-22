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

/// A session whose reported end precedes its reported start, flagged as such.
///
/// [`session`] cannot express this: it derives the end from a positive elapsed time. The
/// inversion has to be built by hand, and it is worth having because such a record is the one shape
/// that no calculation may be handed --
/// [`Session::adj_duration`](super::Session::adj_duration) panics on it rather than returning a
/// negative span, so a caller that forgets to drop it brings the call down instead of reporting a
/// wrong figure.
///
/// Carries [`AnomalyKind::InconsistentDuration`] because that is what the readers attach to it, and
/// what [`Sessions`](super::Sessions) sorts on. See
/// [`duration_is_consistent`](super::duration_is_consistent), check 1.
pub(crate) fn inverted_session(
    path: &str,
    row: usize,
    id: &str,
    conn_start: &str,
    minutes_before_start: i64,
    energy_use: f64,
) -> RSession {
    let conn_start: Timestamp = conn_start.parse().expect("an RFC 3339 timestamp");
    let backwards = Duration::from_secs(minutes_before_start.unsigned_abs() * 60);
    Rc::new(Session {
        path: Rc::new(PathBuf::from(path)),
        row,
        id: id.to_owned(),
        conn_start,
        conn_end: conn_start - backwards,
        conn_duration: backwards,
        charge_time: backwards,
        energy_use,
        anomalies: vec![AnomalyKind::InconsistentDuration],
    })
}

/// A session with real energy and zero `Active_Charge_Time` — what the readers call a spike.
///
/// `energy_use / charge_time` is infinite, which is why the peak estimates hold these apart. The
/// connection itself is sound and `minutes` long, so anything derived from connection time rather
/// than charge time is well defined.
pub(crate) fn spike_session(
    path: &str,
    row: usize,
    id: &str,
    conn_start: &str,
    minutes: i64,
    energy_use: f64,
) -> RSession {
    let sound = session(path, row, id, conn_start, minutes, energy_use);
    Rc::new(Session {
        path: sound.path.clone(),
        row: sound.row,
        id: sound.id.clone(),
        conn_start: sound.conn_start,
        conn_end: sound.conn_end,
        conn_duration: sound.conn_duration,
        charge_time: Duration::ZERO,
        energy_use: sound.energy_use,
        anomalies: sound.anomalies.clone(),
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
