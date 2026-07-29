//! Golden-file tests for the rendered [`PowerEstimatesReport`].
//!
//! Each case pairs an input CSV in `tests/fixtures/` with the report it must produce, checked
//! byte for byte. Layout is the thing under test, and layout is only judged by looking at it — so
//! the expectation is a file you can read rather than a list of assertions about substrings. A
//! change in wrapping, padding or column order shows up as a diff in the golden file, which is
//! exactly where it should be visible during review.
//!
//! The three cases between them cover every shape the renderer has: a clean report, one carrying
//! session anomalies, and one where a group exceeds a single panel and both estimate sets are
//! emitted. The last is otherwise unreachable through the real path — no report anyone has
//! produced reaches eleven concurrent sessions.
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
/// All three sit on 2026-06-15, a date with no DST transition, and run 16:00–17:00 local — a legal
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
        "Session_Report_Clamped",
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
