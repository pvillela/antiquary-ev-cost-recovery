//! The `InconsistentDuration` band, exercised end to end through the public API.
//!
//! Unit tests in `sessions::excel` already pin the predicate. This pins the *consequence*: which
//! sessions reach an estimate and which are put aside. Nothing did that before, which is how commit
//! `1d99e29` moved the band by a whole `TIME_GRID_STEP` — excluding 116 of the 238 sessions in the
//! real June report — with the whole suite green. See `docs/merger-review-findings.md`, finding 1.
//!
//! # The arithmetic every record here is one second away from
//!
//! With a reported start of `16:10` and end of `16:40`, `Δ = 30:00`, the three checks of
//! `duration_is_consistent` reduce to a closed range of sound durations:
//!
//! ```text
//! check 2:  16:10 + d  <  16:40 + 60s + 1s   =>  d <= 0:31:00
//! check 3:  16:40 - 60s  <  16:10 + d        =>  d >= 0:29:01
//! ```
//!
//! So `[0:29:01, 0:31:00]` is sound and everything outside it is not. Each record below sits on one
//! of those two edges or one second past it. Getting a bound wrong by a single second moves a real
//! record between the two groups, so a second is the right resolution to test at.
//!
//! `INVERT1` tests check 1 on its own, and is the case with no unit-test equivalent reachable from
//! a CSV: a one-minute inversion is the smallest the reporting can express, since both reported
//! times are whole minutes, and that forces the duration to zero. It therefore also carries
//! `ZeroActiveChargeTime` — the two travel together by arithmetic, not coincidence.

use ev_cost_recovery::{
    sessions::{
        AnomalyKind,
        excel::{session_csv_to_xlsx, session_list},
        interval_estimates,
    },
    time::Interval,
};
use jiff::Timestamp;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::fixture;

/// 16:00–17:00 local on a date with no DST transition, which contains every record.
const LO: &str = "2026-06-15T20:00:00Z";
const HI: &str = "2026-06-15T21:00:00Z";

/// Ids expected to fail one of the three checks.
const UNSOUND: [&str; 3] = ["EARLYOUT", "INVERT1", "LATEOUT"];
/// Ids expected to pass all three, each one second inside a bound.
const SOUND: [&str; 2] = ["EARLYIN", "LATEIN"];

/// Converts the fixture in a scratch directory, so no generated workbook lands in the fixtures.
///
/// A directory per call, not per process: these tests run in parallel and each tears its own down
/// afterwards, so a shared one would be removed from under a test still reading it.
fn workbook() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ev_peak_band_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("Session_Report_Band.csv");
    fs::copy(fixture("Session_Report_Band.csv"), &csv).unwrap();
    session_csv_to_xlsx(&csv)
        .expect("the band fixture converts")
        .output_path
}

/// The three unsound records are excluded and the two sound ones are not.
///
/// Asserted on the read side, since that is where the split into `excluded` actually happens and
/// what every estimate downstream depends on.
#[test]
fn the_band_decides_which_sessions_reach_an_estimate() {
    let xlsx = workbook();
    let report = session_list(&xlsx).expect("the workbook reads back");

    let mut excluded: Vec<&str> = report.excluded.iter().map(|s| s.id.as_str()).collect();
    excluded.sort_unstable();
    assert_eq!(excluded, UNSOUND, "excluded set");

    // The sound ones are present and usable, not merely absent from `excluded`. `EARLYIN` and
    // `LATEIN` both have a non-zero charge time, so neither is a spike.
    let mut kept: Vec<&str> = report.sessions.iter().map(|s| s.id.as_str()).collect();
    kept.sort_unstable();
    assert_eq!(kept, SOUND, "sessions reaching the estimates");

    fs::remove_dir_all(xlsx.parent().unwrap()).ok();
}

/// Each unsound record carries `InconsistentDuration`, and each sound one carries no timing
/// anomaly at all.
///
/// Separate from the test above because a session could in principle be excluded for the wrong
/// reason, and then the split would look right while the diagnosis was wrong.
#[test]
fn the_flag_and_the_exclusion_agree() {
    let xlsx = workbook();
    let report = session_list(&xlsx).expect("the workbook reads back");

    for session in &report.excluded {
        assert!(
            session
                .anomalies
                .contains(&AnomalyKind::InconsistentDuration),
            "{} is excluded but not flagged: {:?}",
            session.id,
            session.anomalies
        );
    }
    for session in &report.sessions {
        assert!(
            !session
                .anomalies
                .contains(&AnomalyKind::InconsistentDuration),
            "{} is flagged but not excluded: {:?}",
            session.id,
            session.anomalies
        );
    }

    // The inverted record is the one no other fixture carries. Its zero duration means it picks up
    // `ZeroActiveChargeTime` too, and both are expected.
    let invert = report
        .excluded
        .iter()
        .find(|s| s.id == "INVERT1")
        .expect("the inverted record is excluded");
    assert!(
        invert.conn_end < invert.conn_start,
        "the fixture no longer inverts: {} to {}",
        invert.conn_start,
        invert.conn_end
    );

    fs::remove_dir_all(xlsx.parent().unwrap()).ok();
}

/// An inverted record reaches the report without panicking.
///
/// The reason check 1 exists. `Session::intersects` panics on an inverted span and documents
/// exclusion by this test as the reason it cannot happen; before check 1 that was untrue, and a
/// record like `INVERT1` reached it. This is the end-to-end proof that it no longer does — the
/// excluded listing walks every excluded session, inverted ones included.
#[test]
fn an_inverted_record_is_listed_rather_than_crashing() {
    let xlsx = workbook();
    let interval = Interval::from_start_end(
        LO.parse::<Timestamp>().unwrap(),
        HI.parse::<Timestamp>().unwrap(),
    );
    let estimates = interval_estimates(interval, &xlsx).expect("estimating must not panic");

    let listed: Vec<&str> = estimates
        .excluded_sessions
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert!(listed.contains(&"INVERT1"), "not listed: {listed:?}");

    // And the sound pair did reach the figures, so the report is not empty by accident.
    let members: Vec<String> = estimates
        .seg_estimates
        .iter()
        .flat_map(|(seg, _)| seg.sessions.iter().map(|s| s.id.clone()))
        .collect();
    for id in SOUND {
        assert!(
            members.iter().any(|m| m == id),
            "{id} is sound but took part in no segment: {members:?}"
        );
    }

    fs::remove_dir_all(xlsx.parent().unwrap()).ok();
}
