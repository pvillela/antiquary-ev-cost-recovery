//! Golden-file tests for the rendered [`PowerEstimatesReport`].
//!
//! Each case pairs an input CSV in `tests/fixtures/` with the report it must produce, checked
//! byte for byte. Layout is the thing under test, and layout is only judged by looking at it — so
//! the expectation is a file you can read rather than a list of assertions about substrings. A
//! change in wrapping, padding or column order shows up as a diff in the golden file, which is
//! exactly where it should be visible during review.
//!
//! The cases between them cover every shape the renderer has: a report holding a dubious group, so
//! that both estimate sets are printed and the group table carries the marker and its two extra
//! columns; one carrying session anomalies and an excluded-sessions section; and one where a skew
//! margin outruns the interval of interest and so earns a section of its own. All are reachable
//! through the real path — a dubious group needs nothing more than two sessions reported to meet on
//! the same minute.
//!
//! To regenerate after an intended change, having read the diff:
//!
//! ```sh
//! UPDATE_REPORT_GOLDEN=1 cargo test --test report_rendering
//! ```
//!
//! cargo test --test report_rendering

use ev_peak_contrib::{max_power_estimates_for_interval, session_csv_to_xlsx};
use jiff::Timestamp;
use std::{fs, path::PathBuf};

/// `(fixture stem, interval start UTC, interval end UTC)`.
///
/// Both sit on 2026-06-15, a date with no DST transition, and run 16:00–17:00 local — a legal
/// interval of interest per README.
const CASES: [(&str, &str, &str); 3] = [
    (
        "Session_Report_Diagram",
        "2026-06-15T20:00:00Z",
        "2026-06-15T21:00:00Z",
    ),
    (
        "Session_Report_Anomalies",
        "2026-06-15T20:00:00Z",
        "2026-06-15T21:00:00Z",
    ),
    (
        "Session_Report_SkewMargin",
        "2026-06-15T20:00:00Z",
        "2026-06-15T21:00:00Z",
    ),
];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Converts the fixture in a scratch directory and renders its report, so no generated workbook
/// lands in `tests/fixtures/`.
fn render(stem: &str, lo: &str, hi: &str) -> String {
    let dir = std::env::temp_dir().join(format!("ev_peak_report_{stem}_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let csv = dir.join(format!("{stem}.csv"));
    fs::copy(fixtures().join(format!("{stem}.csv")), &csv).unwrap();

    let xlsx = session_csv_to_xlsx(&csv)
        .unwrap_or_else(|e| panic!("{stem} converts: {e}"))
        .output_path;
    let interval: (Timestamp, Timestamp) = (lo.parse().unwrap(), hi.parse().unwrap());
    let report = max_power_estimates_for_interval(interval, &xlsx)
        .unwrap_or_else(|e| panic!("{stem} estimates: {e}"));

    let rendered = report.to_markdown();
    // Display must agree, or there would be two renderings to keep in step.
    assert_eq!(format!("{report}"), rendered, "{stem}: Display disagrees");

    fs::remove_dir_all(&dir).ok();
    // The scratch path leaks into the Source line; normalise it to the bare file name the report
    // already prints, so the golden file does not depend on where the test ran.
    rendered
}

#[test]
fn rendered_reports_match_their_golden_files() {
    let mut stale: Vec<String> = Vec::new();
    for (stem, lo, hi) in CASES {
        let rendered = render(stem, lo, hi);
        let golden = fixtures().join(format!("{stem}.report.md"));

        if std::env::var_os("UPDATE_REPORT_GOLDEN").is_some() {
            fs::write(&golden, &rendered).unwrap();
            continue;
        }

        let expected = fs::read_to_string(&golden).unwrap_or_else(|e| {
            panic!(
                "{}: {e}\nRun with UPDATE_REPORT_GOLDEN=1 to create it.",
                golden.display()
            )
        });
        if expected != rendered {
            // Named rather than left to be found in a diff where every line differs and none of
            // them visibly. The comparison stays byte for byte — a golden that has picked up CRLF
            // is a real fault, just not one the printed diff can show.
            if expected.replace("\r\n", "\n") == rendered {
                stale.push(format!(
                    "--- {stem} ---\nThe only difference is line endings: the golden file holds \
                     CRLF and the renderer emits LF. The working copy was checked out with git \
                     translating line endings - see .gitattributes, which pins these files to LF, \
                     and re-check-out with `git rm --cached -r . && git reset --hard`."
                ));
                continue;
            }
            stale.push(format!(
                "--- {stem} ---\nexpected:\n{expected}\nactual:\n{rendered}"
            ));
        }
    }
    assert!(
        stale.is_empty(),
        "rendered reports differ from their golden files. Read the diff, and if the change is \
         intended regenerate with UPDATE_REPORT_GOLDEN=1.\n\n{}",
        stale.join("\n")
    );
}

/// The constraint the renderer exists to satisfy: the output has to be legible with no markdown
/// renderer at all. Checked against the golden files themselves, so it holds for what ships.
#[test]
fn golden_reports_are_readable_as_plain_text() {
    for (stem, _, _) in CASES {
        let path = fixtures().join(format!("{stem}.report.md"));
        let Ok(md) = fs::read_to_string(&path) else {
            continue; // covered by the test above
        };
        for (i, line) in md.lines().enumerate() {
            let at = format!("{stem}.report.md:{}", i + 1);
            assert!(
                !line.starts_with("    "),
                "{at}: four-space indent renders as a code block: {line:?}"
            );
            assert!(
                !line.starts_with('#'),
                "{at}: hash heading; setext underlines read better raw: {line:?}"
            );
            assert!(
                !line.contains("<br"),
                "{at}: HTML break shows literally: {line:?}"
            );
            assert!(
                line.chars().count() <= 90,
                "{at}: {} columns, too wide to read raw: {line:?}",
                line.chars().count()
            );
        }
        assert!(
            !md.contains("**"),
            "{stem}: bold markers are noise in plain text"
        );
        assert!(
            !md.contains('`'),
            "{stem}: backticks are noise in plain text"
        );
    }
}

/// Anomalies are scoped to the interval, not to the workbook: a workbook covers a billing period
/// while a report covers one window in it.
///
/// The fixture carries two sessions for this, and the golden file is where the outcome shows. A
/// spike the following day must not appear, however anomalous. A record whose reported end precedes
/// its start — the extreme of `InconsistentDuration` — sits inside the window and must appear,
/// which it only does because the overlap test normalises the span rather than taking
/// `start < hi && end > lo` at face value.
#[test]
fn anomalies_are_scoped_to_the_interval() {
    let md = fs::read_to_string(fixtures().join("Session_Report_Anomalies.report.md")).unwrap();
    assert!(
        !md.contains("FARSPIKE"),
        "a session a day outside the interval was reported:\n{md}"
    );
    assert!(
        md.contains("REVERSED"),
        "a record whose end precedes its start, inside the interval, went unreported:\n{md}"
    );
}

/// `ExcessiveAvgPower` carries its figure in the table cell, while the kind itself stays a bare
/// token.
///
/// The value is written into the cell rather than onto the enum, so the workbook's `Anomalies`
/// column remains a list of variant names `AnomalyKind::from_token` can read back, and the glossary
/// under the table still explains each kind once rather than once per session. `EXCESS` draws 6.9 kW
/// against a 6.7 kW breaker.
#[test]
fn an_excessive_average_power_is_reported_with_its_figure() {
    let md = fs::read_to_string(fixtures().join("Session_Report_Anomalies.report.md")).unwrap();
    assert!(
        md.contains("| ExcessiveAvgPower(6.900) |"),
        "the figure is missing from the cell:\n{md}"
    );
    // The glossary explains the kind, so it names the kind and not one session's figure.
    assert!(
        md.contains("- ExcessiveAvgPower - average power above"),
        "the glossary entry is missing or carries a figure:\n{md}"
    );
}

/// A skew margin is reported when it beats the interval of interest, and dropped when it does not —
/// each margin judged on its own.
///
/// The fixture is built for both directions at once. `BEFORE1` and `BEFORE2` run only in the minute
/// before the interval, drawing 6 kW apiece against the 2 kW of the one session inside it, so the
/// left margin outruns `I` on every figure. `AFTER` runs only in the minute after, drawing less than
/// `INSIDE` and alone where `INSIDE` is alone, so the right margin beats `I` on nothing and is left
/// out. Both margins are always computed; only one survives the trigger.
///
/// Every session draws under the breaker rating, as Evolute's hardware constrains them to. A
/// fixture that ignored that would put the consumption-based figure above the breaker-spec-based
/// one and invert the bracket the report states.
#[test]
fn a_skew_margin_is_reported_only_when_it_beats_the_interval() {
    let md = fs::read_to_string(fixtures().join("Session_Report_SkewMargin.report.md")).unwrap();
    assert!(md.contains("Skew margins"), "the section is missing:\n{md}");
    assert!(
        md.contains("BEFORE1") && md.contains("BEFORE2"),
        "the left margin, which outruns the interval, went unreported:\n{md}"
    );
    assert!(
        !md.contains("AFTER"),
        "the right margin beats the interval on nothing and should have been dropped:\n{md}"
    );
    // The margin's own span, not the interval's, and one `SESSION_BOUNDARY_RESOLUTION` wide.
    assert!(
        md.contains("\"Before\" - 2026-06-15 15:59 - 16:00 EDT (1 minute)"),
        "the margin's interval line is wrong:\n{md}"
    );
}

/// Neither margin of the anomalies fixture beats its interval, so no section appears there. The
/// negative case of the test above, on a report that was never built for it.
#[test]
fn a_report_without_a_qualifying_margin_says_nothing_about_margins() {
    let md = fs::read_to_string(fixtures().join("Session_Report_Anomalies.report.md")).unwrap();
    assert!(
        !md.contains("Skew margins"),
        "a margin that beats nothing was reported:\n{md}"
    );
    assert!(
        !md.contains("Covered"),
        "the covered-span line belongs only to a report that shows a margin:\n{md}"
    );
}

/// Every table in every golden file has rows of equal width. That padding is what makes the output
/// line up in a monospace font, and nothing else checks it.
#[test]
fn golden_report_tables_are_padded_evenly() {
    for (stem, _, _) in CASES {
        let path = fixtures().join(format!("{stem}.report.md"));
        let Ok(md) = fs::read_to_string(&path) else {
            continue;
        };
        let mut block: Vec<(usize, usize)> = Vec::new();
        for (i, line) in md.lines().chain(std::iter::once("")).enumerate() {
            if line.starts_with('|') {
                block.push((i + 1, line.chars().count()));
            } else if !block.is_empty() {
                let w = block[0].1;
                for (ln, got) in &block {
                    assert_eq!(
                        *got, w,
                        "{stem}.report.md:{ln}: ragged table row, {got} columns against {w}"
                    );
                }
                block.clear();
            }
        }
    }
}
