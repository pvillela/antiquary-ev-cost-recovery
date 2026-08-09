//! How sessions land on the quarter-hour segments that tile an interval of interest.
//!
//! Driven through the public API only — a CSV in, an [`ev_peak_contrib::IntervalEstimates`] out —
//! so what is pinned here is behaviour a caller can actually depend on, not the shape of an
//! internal helper.
//!
//! The fixture is `Session_Report_Diagram.csv`, seven sessions over 16:00–17:00 chosen to exercise
//! every geometry the tiling has to get right at once, and `docs/segment-tiling.md` walks through
//! the same example in prose:
//!
//! ```text
//!            16:00      16:15      16:30      16:45      17:00
//!              |          |          |          |          |
//!   A   15:54 =|==========|==========|==========|==========|===== 17:04
//!   B     15:59|=====|16:16                                        overruns the left edge
//!   C          |  16:08 =====|16:43                                nested, spans two segments
//!   E          |         16:20 ==|16:35                            staggered start with D
//!   D          |          16:24 =|16:35                            ends the same minute as E
//!   F          |            16:34 =====|16:43                      starts the minute D and E end
//!   G          |                       16:48 ==|16:56              alone in the last segment
//! ```
//!
//! Spans are `[conn_start, adj_conn_end)`, so each right edge is the reported end plus one
//! `TIME_GRID_STEP`. That is why `F` overlaps `D` and `E` rather than merely abutting them: all
//! three are reported to the minute, and the minute they share may hold any of them.
//!
//! No assertion here names an electrical constant. The numbers this file does state are clock
//! times and session counts, which are properties of the fixture rather than of the site model.

use ev_peak_contrib::{
    AnomalyKind, BREAKER_RATING_KW, Interval, interval_estimates, session_csv_to_xlsx,
};
use jiff::{Timestamp, Zoned, tz::TimeZone};
use std::{
    fs,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

/// The interval of interest: 16:00–17:00 local on a date with no DST transition.
const LO: &str = "2026-06-15T20:00:00Z";
const HI: &str = "2026-06-15T21:00:00Z";

/// The four quarters, in order, as the report names them.
const QUARTERS: [&str; 4] = ["16:00", "16:15", "16:30", "16:45"];

/// Which sessions each quarter holds, given the spans in the diagram above.
const MEMBERSHIP: [(&str, &[&str]); 4] = [
    // `D`, `E`, `F` and `G` all start after 16:15, so the first quarter holds only the three
    // sessions already running.
    ("16:00", &["A", "B", "C"]),
    // `B` reaches this quarter by a single minute: reported to end at 16:15, its true end lies
    // anywhere in [16:15, 16:16), so it may still have been drawing when the quarter opened.
    ("16:15", &["A", "B", "C", "D", "E"]),
    // `D` and `E` are reported to end at 16:34 and `F` to start at 16:34, so all three are here:
    // the reported times cannot say whether they overlapped or merely abutted.
    ("16:30", &["A", "C", "D", "E", "F"]),
    // Only `A`, which outruns the whole interval, and `G`.
    ("16:45", &["A", "G"]),
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Local clock time, the way the report names a segment.
fn hm(ts: Timestamp) -> String {
    Zoned::new(ts, TimeZone::get("America/Toronto").unwrap())
        .strftime("%H:%M")
        .to_string()
}

/// Converts the fixture in a scratch directory and estimates over the interval, so no generated
/// workbook lands in `tests/fixtures/`.
///
/// A directory per call, not per process: the tests below run in parallel and each tears its own
/// down afterwards, so a shared one would be removed from under a test still reading it.
/// `IntervalEstimates` holds `Rc`s, so it cannot be computed once and shared across threads either.
fn estimates() -> ev_peak_contrib::IntervalEstimates {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ev_peak_tiling_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("Session_Report_Diagram.csv");
    fs::copy(fixtures().join("Session_Report_Diagram.csv"), &csv).unwrap();

    let xlsx = session_csv_to_xlsx(&csv).unwrap().output_path;
    let interval = Interval::from_start_end(LO.parse().unwrap(), HI.parse().unwrap());
    let report = interval_estimates(interval, &xlsx).unwrap();

    fs::remove_dir_all(&dir).ok();
    report
}

/// An hour is four quarters that tile it exactly: consecutive, gapless, and no wider than the
/// interval between them.
#[test]
fn an_hour_is_tiled_by_four_consecutive_quarters() {
    let report = estimates();
    let segments: Vec<_> = report.seg_estimates.iter().map(|(s, _)| s).collect();

    assert_eq!(segments.len(), 4);
    assert_eq!(
        segments.iter().map(|s| hm(s.start())).collect::<Vec<_>>(),
        QUARTERS
    );

    // Half-open: each ends exactly where the next begins, so no instant falls in two of them.
    for pair in segments.windows(2) {
        assert_eq!(pair[0].end(), pair[1].start());
    }
    assert_eq!(segments[0].start(), report.interval.start);
    assert_eq!(segments[3].end(), report.interval.end());
}

/// Each quarter holds exactly the sessions whose span meets it.
///
/// This is the assertion that would have caught the inverted overlap test: with `intersects`
/// negated, every quarter collected precisely the sessions listed for the others.
#[test]
fn each_quarter_holds_the_sessions_that_meet_it() {
    let report = estimates();

    for (segment, (quarter, expected)) in report.seg_estimates.iter().zip(MEMBERSHIP) {
        let (segment, _) = segment;
        assert_eq!(hm(segment.start()), quarter);

        let mut ids: Vec<String> = segment.sessions.iter().map(|s| s.id.clone()).collect();
        ids.sort();
        assert_eq!(ids, expected, "quarter {quarter}");
    }
}

/// A session reported to end in the same minute another is reported to start belongs to both, and
/// the doubt shows up as a bracket rather than being resolved by fiat.
///
/// `D` and `E` end at 16:34, `F` starts at 16:34. All three are in the 16:30 quarter, and `F`'s
/// contribution to it is a range: it covers at least 16:35–16:42 and at most 16:34–16:43.
#[test]
fn a_shared_minute_leaves_the_count_bracketed() {
    let report = estimates();
    let (third, _) = &report.seg_estimates[2];
    assert_eq!(hm(third.start()), "16:30");

    let count = third.agg_count();
    assert!(
        count.min < count.max,
        "a quarter holding a shared minute should not be exact: {count:?}"
    );
    // A session that neither starts nor ends inside the quarter contributes exactly 1, so the
    // bracket's width comes only from the sessions whose edges fall in it.
    assert!(count.min > 1.0 && count.max < 5.0, "{count:?}");
}

/// A session outrunning the interval on both sides counts as a full session in every quarter, with
/// no uncertainty: neither of its reported edges falls inside the interval, so no minute of doubt
/// applies.
#[test]
fn a_session_spanning_the_whole_interval_is_exact_everywhere() {
    let report = estimates();

    for (segment, _) in &report.seg_estimates {
        assert!(
            segment.sessions.iter().any(|s| s.id == "A"),
            "A spans the whole interval and should be in every quarter"
        );
    }

    // The last quarter holds only `A` and `G`, and `G` contributes less than a whole session, so
    // the quarter's count brackets `A`'s exact 1 from above.
    let (last, _) = &report.seg_estimates[3];
    let count = last.agg_count();
    assert!(count.min > 1.0 && count.max < 2.0, "{count:?}");
}

/// The busiest quarters are the middle two, and the maximal segment is drawn from them rather than
/// from the sparse first or last.
///
/// Stated as an ordering rather than as figures: which quarter wins is a fact about the fixture's
/// geometry, while the figures it wins with depend on the site model.
#[test]
fn the_maximal_segment_is_one_of_the_busy_middle_quarters() {
    let report = estimates();
    let counts: Vec<f64> = report
        .seg_estimates
        .iter()
        .map(|(s, _)| s.agg_count().mid())
        .collect();

    assert!(counts[1] > counts[0], "16:15 should beat 16:00");
    assert!(counts[2] > counts[0], "16:30 should beat 16:00");
    assert!(counts[1] > counts[3], "16:15 should beat 16:45");
    assert!(counts[2] > counts[3], "16:30 should beat 16:45");

    let (energy_seg, _) = &report.energy_based_seg_estimate;
    let (count_seg, _) = &report.count_based_seg_estimate;
    for name in [hm(energy_seg.start()), hm(count_seg.start())] {
        assert!(
            name == "16:15" || name == "16:30",
            "maximal segment {name} is not one of the busy quarters"
        );
    }

    // Each maximum is one of the segments in the listing, shared rather than copied — so a caller
    // can join the Estimates figures to the Segments row by identity, not by matching clock times.
    for maximal in [energy_seg, count_seg] {
        assert!(
            report
                .seg_estimates
                .iter()
                .any(|(seg, _)| Rc::ptr_eq(seg, maximal)),
            "a maximal segment is a copy rather than one of the listed segments"
        );
    }
}

/// No session in the fixture is excluded, so all seven take part in the tiling.
///
/// Worth stating: it is what makes the membership assertions above a test of the tiling rather
/// than of the exclusion filter. Whatever anomalies the fixture carries are informational, and
/// `InconsistentDuration` is the only kind that removes a session.
///
/// The anomalies it does carry are checked against the rating rather than counted. Which sessions
/// exceed `BREAKER_RATING_KW` depends on that constant's value, and no test here may; what does not
/// depend on it is that the flag and the comparison agree.
#[test]
fn no_session_is_excluded_and_every_flag_agrees_with_the_rating() {
    let report = estimates();
    assert!(report.excluded_sessions.is_empty());

    let mut seen: Vec<String> = report
        .seg_estimates
        .iter()
        .flat_map(|(seg, _)| seg.sessions.iter().map(|s| s.id.clone()))
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen, ["A", "B", "C", "D", "E", "F", "G"]);

    // Every session, flagged or not, judged by the one rule the flag encodes.
    for (seg, _) in &report.seg_estimates {
        for session in &seg.sessions {
            let flagged = report
                .session_anomalies
                .iter()
                .any(|a| a.session_id == session.id && a.kind == AnomalyKind::ExcessiveAvgKw);
            assert_eq!(
                flagged,
                session.avg_kw() > BREAKER_RATING_KW,
                "{} draws {} against a {BREAKER_RATING_KW} rating",
                session.id,
                session.avg_kw()
            );
        }
    }

    // Nothing else is wrong with the fixture: it was built for geometry, not for faults.
    assert!(
        report
            .session_anomalies
            .iter()
            .all(|a| a.kind == AnomalyKind::ExcessiveAvgKw)
    );
}
