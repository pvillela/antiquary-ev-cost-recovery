//! End-to-end check against the worked example in `docs/session-grouping.md`.
//!
//! Seven sessions are arranged so that the interval of interest splits into ten session groups with
//! session counts 2, 3, 2, 3, 4, 5, 3, 1, 2, 1. The arrangement is not arbitrary: it exercises
//! every shape the sweep has to get right — nested sessions, staggered ones, two sessions ending on
//! the same minute a third begins, two ending together, a stretch with only one session left, and a
//! session overrunning the interval at both ends.
//!
//! The run is driven entirely through the public API, from CSV to power estimates, so it also
//! stands as a check that the public surface is sufficient for a caller.
//!
//! cargo test --test session_grouping_diagram

use ev_peak_contrib::{
    EV_POWER_FACTOR, EVOLUTE_BREAKER_KVA_RATING, EVOLUTE_BREAKER_KW_RATING, SessionGroup,
    max_power_estimates_for_interval, session_csv_to_xlsx, session_list,
};
use jiff::Timestamp;
use std::{fs, path::PathBuf, time::Duration};

/// The interval of interest: 16:00–17:00 local on 2026-06-15, i.e. a legal one-hour interval on a
/// date with no DST transition. Stated in UTC, which is what the estimating logic takes.
const INTERVAL_START: &str = "2026-06-15T20:00:00Z";
const INTERVAL_END: &str = "2026-06-15T21:00:00Z";

fn ts(s: &str) -> Timestamp {
    s.parse().expect("valid timestamp")
}

/// Copies the committed fixture into a scratch directory and converts it there, so the generated
/// workbook never lands in `tests/fixtures/`.
fn convert_fixture() -> PathBuf {
    let src =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Session_Report_Diagram.csv");
    let dir = std::env::temp_dir().join(format!("ev_peak_diagram_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("Session_Report_Diagram.csv");
    fs::copy(&src, &csv).unwrap();

    let report = session_csv_to_xlsx(&csv).expect("fixture converts");
    assert!(
        report.anomalies.is_empty(),
        "the diagram fixture is meant to be clean: {:?}",
        report.anomalies
    );
    report.output_path
}

/// `(seconds from interval start, seconds from interval start, session ids)` for each group.
fn tiling(groups: &[SessionGroup]) -> Vec<(i64, i64, String)> {
    let lo = ts(INTERVAL_START);
    groups
        .iter()
        .map(|g| {
            let ids: Vec<String> = g.session_iter().map(|s| s.id.clone()).collect();
            (
                g.start().duration_since(lo).as_secs(),
                g.end().duration_since(lo).as_secs(),
                ids.join(","),
            )
        })
        .collect()
}

#[test]
fn diagram_scenario_produces_the_expected_tiling_and_estimates() {
    let xlsx = convert_fixture();

    // Every session is well formed: nothing anomalous, nothing excluded, no spikes.
    let listed = session_list(&xlsx).expect("workbook reads back");
    assert_eq!(listed.sessions.len(), 7);
    assert!(listed.spikes.is_empty());
    assert!(listed.excluded.is_empty());
    assert!(listed.sessions.iter().all(|s| s.anomalies.is_empty()));

    let report = max_power_estimates_for_interval((ts(INTERVAL_START), ts(INTERVAL_END)), &xlsx)
        .expect("estimates computed");
    let groups = &report.session_groups;

    // The ten groups, as offsets in seconds from 16:00 local.
    #[rustfmt::skip]
    let expected = [
        (   0,  480, "A,B"),          //  0  16:00:00 – 16:08:00   B running, C not yet
        ( 480,  959, "A,B,C"),        //  1  16:08:00 – 16:15:59   C joins
        ( 959, 1200, "A,C"),          //  2  16:15:59 – 16:20:00   B's padded end
        (1200, 1440, "A,C,E"),        //  3  16:20:00 – 16:24:00   E joins
        (1440, 2040, "A,C,D,E"),      //  4  16:24:00 – 16:34:00   D joins
        (2040, 2099, "A,C,D,E,F"),    //  5  16:34:00 – 16:34:59   F starts as D and E finish
        (2099, 2579, "A,C,F"),        //  6  16:34:59 – 16:42:59   D and E gone
        (2579, 2880, "A"),            //  7  16:42:59 – 16:48:00   C and F gone, A alone
        (2880, 3359, "A,G"),          //  8  16:48:00 – 16:55:59   G joins
        (3359, 3600, "A"),            //  9  16:55:59 – 17:00:00   A alone to the interval end
    ];
    assert_eq!(
        tiling(groups),
        expected.map(|(s, e, ids)| (s, e, ids.to_owned()))
    );

    // The counts along the bottom of the diagram.
    let counts: Vec<usize> = groups.iter().map(SessionGroup::session_count).collect();
    assert_eq!(counts, [2, 3, 2, 3, 4, 5, 3, 1, 2, 1]);

    // The groups tile the interval: contiguous, and spanning it end to end. A overruns both ends,
    // so no part of the hour is left uncovered.
    for pair in groups.windows(2) {
        assert_eq!(pair[0].end(), pair[1].start(), "groups must be contiguous");
    }
    assert_eq!(groups[0].start(), ts(INTERVAL_START));
    assert_eq!(groups[9].end(), ts(INTERVAL_END));

    // The five-session group lasts 59 seconds, not a minute. D and E report ending on the same
    // minute F reports starting, and `Adj_conn_end` pads a reported end to the end of its minute —
    // so the three genuinely overlap, for as long as that padding lasts. This sliver is the reason
    // the arrangement is worth a test: it is the shortest group and the highest load.
    assert_eq!(groups[5].duration(), Duration::from_secs(59));
    assert_eq!(
        groups.iter().map(SessionGroup::duration).min(),
        Some(Duration::from_secs(59)),
        "the sliver should be the shortest group"
    );

    // Aggregate power per group. Every session draws between 5.9 and 6.7 kW, as Evolute's breakers
    // constrain them to, so the aggregate tracks the count closely.
    let agg: Vec<f64> = groups.iter().map(SessionGroup::agg_avg_power).collect();
    let expected_agg = [12.4, 18.6, 12.2, 18.1, 24.7, 31.4, 18.9, 6.0, 12.1, 6.0];
    for (i, (got, want)) in agg.iter().zip(expected_agg).enumerate() {
        assert!((got - want).abs() < 1e-9, "group {i}: {got} != {want}");
    }

    // Both estimates peak at group 5. With every session near the breaker rating the aggregate can
    // only track the count, so the two can diverge just when the maximum count is reached by more
    // than one group — which it is not here.
    let estimates = report
        .estimates
        .expect("some session intersects the interval");
    assert_eq!(estimates.consumption_based_kw.session_group_idx, 5);
    assert_eq!(estimates.breaker_specs_based_kw.session_group_idx, 5);

    assert!((estimates.consumption_based_kw.value - 31.4).abs() < 1e-9);
    assert!((estimates.consumption_based_kva.value - 31.4 / EV_POWER_FACTOR).abs() < 1e-9);
    assert!(
        (estimates.breaker_specs_based_kw.value - 5.0 * EVOLUTE_BREAKER_KW_RATING).abs() < 1e-9
    );
    assert!(
        (estimates.breaker_specs_based_kva.value - 5.0 * EVOLUTE_BREAKER_KVA_RATING).abs() < 1e-9
    );

    // The brackets from README, "Estimation logic": the consumption-based figure is the lower end
    // of each pair, the breaker-spec figure the upper.
    assert!(estimates.consumption_based_kw.value < estimates.breaker_specs_based_kw.value);
    assert!(estimates.consumption_based_kva.value < estimates.breaker_specs_based_kva.value);

    fs::remove_dir_all(xlsx.parent().unwrap()).ok();
}
