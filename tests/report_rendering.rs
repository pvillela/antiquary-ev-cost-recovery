//! Golden-file tests for the rendered [`ev_peak_contrib::IntervalEstimates`].
//!
//! Each case pairs an input CSV in `tests/fixtures/` with the report it must produce, checked
//! byte for byte. Layout is the thing under test, and layout is only judged by looking at it — so
//! the expectation is a file you can read rather than a list of assertions about substrings. A
//! change in wrapping, padding or column order shows up as a diff in the golden file, which is
//! exactly where it should be visible during review.
//!
//! The cases between them cover every shape the renderer has: a report whose four segments differ
//! from each other, so the Estimates section has a maximal quarter to name and the Segments table
//! has something to show; and one carrying session anomalies and an excluded-sessions section.
//! Both are reached through the real path, from a CSV.
//!
//! The site-load table is pinned here too, by the same variable, so one command regenerates every
//! golden file the crate has.
//!
//! These files are the one deliberate exception to the rule that no test may depend on the value of
//! a freely-declared constant. They pin *rendering* — column widths, decimal places, wrapping — and
//! no relational reformulation preserves any of that. Changing an electrical constant is therefore
//! expected to fail exactly these tests and no others; see `docs/maintenance-manual.md`.
//!
//! To regenerate after an intended change, having read the diff:
//!
//! ```sh
//! UPDATE_REPORT_GOLDEN=1 cargo test --test report_rendering
//! ```

use ev_peak_contrib::{Interval, interval_estimates, session_csv_to_xlsx, site_load_report};
use jiff::Timestamp;
use std::{fs, path::PathBuf};

/// `(fixture stem, interval start UTC, interval end UTC)`.
///
/// Both sit on 2026-06-15, a date with no DST transition, and run 16:00–17:00 local — a legal
/// interval of interest per README.
const CASES: [(&str, &str, &str); 2] = [
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
    let (lo, hi): (Timestamp, Timestamp) = (lo.parse().unwrap(), hi.parse().unwrap());
    let interval = Interval::from_start_end(lo, hi);
    let report =
        interval_estimates(interval, &xlsx).unwrap_or_else(|e| panic!("{stem} estimates: {e}"));

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

/// `ExcessiveAvgKw` carries its figure in the table cell, while the kind itself stays a bare
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
        md.contains("| ExcessiveAvgKw(6.900) |"),
        "the figure is missing from the cell:\n{md}"
    );
    // The glossary explains the kind, so it names the kind and not one session's figure.
    assert!(
        md.contains("- ExcessiveAvgKw - average power above"),
        "the glossary entry is missing or carries a figure:\n{md}"
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

/// The site-load table, pinned the same way and by the same variable.
///
/// `.txt` rather than `.md`: it is fixed-width plain text with no markdown in it at all, and naming
/// it otherwise would invite someone to render it. It is the table
/// `docs/ev-charger-power-factor-and-kva-allocation.md` §4 is read against, so a change to any
/// electrical constant should be seen here before it is believed anywhere else.
#[test]
fn the_site_load_table_matches_its_golden_file() {
    let rendered = site_load_report();
    let golden = fixtures().join("site_load.report.txt");

    if std::env::var_os("UPDATE_REPORT_GOLDEN").is_some() {
        fs::write(&golden, &rendered).unwrap();
        return;
    }

    let expected = fs::read_to_string(&golden).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nRun with UPDATE_REPORT_GOLDEN=1 to create it.",
            golden.display()
        )
    });
    assert_eq!(
        expected, rendered,
        "the site-load table differs from its golden file. Read the diff, and if the change is \
         intended regenerate with UPDATE_REPORT_GOLDEN=1."
    );
}
