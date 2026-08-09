//! Golden-file tests over the four fixture feeds.
//!
//! Each fixture is one complete billing period cut from the real export, flanked by two partial
//! ones, and each exists to prove a particular rule:
//!
//! | fixture | proves |
//! |---|---|
//! | `civic_holiday` | the August Civic Holiday is off-peak -- the one date where the OEB list and the ESA list disagree |
//! | `dst_fall` | a 745-interval period is complete |
//! | `dst_spring` | a 671-interval period is complete |
//! | `billed_period` | the month that reconciles against a real invoice |
//!
//! The goldens are text, not workbooks. A generated `.xlsx` is a zip and has no readable diff, and
//! the entire value of a golden file is that somebody reads the diff before committing it.
//! Regenerate with:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test --test fixtures_golden
//! ```
//!
//! Then **read what changed**. Regenerating without reading turns these into a rubber stamp.
//!
//! `Peak_values` is dumped in full -- it is three data rows. `Interval_values` is dumped as its
//! header, its first and last few rows, its row count and its column totals: 792 rows of it would
//! be 4,700 lines nobody would read, while the totals still catch a change anywhere in the middle.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use green_button::{parse, write_workbook};
use umya_spreadsheet::{Worksheet, reader};

const FIXTURES: &[&str] = &["billed_period", "civic_holiday", "dst_fall", "dst_spring"];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn each_fixture_matches_its_golden() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();

    for name in FIXTURES {
        let xml = std::fs::read_to_string(fixtures_dir().join(format!("{name}.XML")))
            .unwrap_or_else(|e| panic!("{name}.XML: {e}"));
        let feed = parse(&xml).unwrap_or_else(|e| panic!("{name}.XML: {e}"));

        // A scratch directory per fixture, since tests run in parallel and the writer refuses to
        // overwrite. Nothing generated is ever written into tests/fixtures.
        let dir = std::env::temp_dir().join(format!("gb_golden_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let workbook = dir.join(format!("{name}.xlsx"));
        let _ = std::fs::remove_file(&workbook);
        write_workbook(&workbook, &feed).unwrap();

        let actual = dump(&workbook);
        std::fs::remove_dir_all(&dir).ok();

        let golden = fixtures_dir().join(format!("{name}.golden.txt"));
        if update {
            std::fs::write(&golden, &actual).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&golden).unwrap_or_default();
        if actual != expected {
            failures.push(name);
            eprintln!("--- {name} differs ---");
            for diff in first_differences(&expected, &actual, 12) {
                eprintln!("{diff}");
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{failures:?} differ from their goldens. Rerun with UPDATE_GOLDEN=1, then read the diff."
    );
}

/// The whole `Peak_values` sheet, then `Interval_values` in summary.
fn dump(path: &Path) -> String {
    let book = reader::xlsx::read(path).expect("the workbook just written must read back");
    let mut out = String::new();

    let peak = book
        .sheet_by_name("Peak_values")
        .expect("Peak_values sheet");
    out.push_str("== Peak_values ==\n");
    for row in 1..=peak.highest_row() {
        dump_row(&mut out, peak, row);
    }

    let interval = book
        .sheet_by_name("Interval_values")
        .expect("Interval_values sheet");
    let last = interval.highest_row();
    writeln!(
        out,
        "\n== Interval_values ==\ndata rows: {}",
        last.saturating_sub(3)
    )
    .unwrap();
    for row in 1..=6.min(last) {
        dump_row(&mut out, interval, row);
    }
    out.push_str("...\n");
    for row in last.saturating_sub(2)..=last {
        dump_row(&mut out, interval, row);
    }
    // Totals catch any change in the rows the excerpt skips.
    for (col, name) in [(3u32, "kwh"), (4, "kw"), (5, "kva")] {
        let total: f64 = (4..=last)
            .filter_map(|row| interval.cell((col, row)))
            .filter_map(|c| c.value().parse::<f64>().ok())
            .sum();
        writeln!(out, "total {name}: {total:.3}").unwrap();
    }
    out
}

fn dump_row(out: &mut String, sheet: &Worksheet, row: u32) {
    for col in 1..=sheet.highest_column() {
        let Some(cell) = sheet.cell((col, row)) else {
            continue;
        };
        let value = cell.value();
        let style = cell.style();
        let format = style
            .number_format()
            .map(|f| f.format_code())
            .unwrap_or_default();
        let fill = match style.background_color() {
            Some(c) => format!("  FILL:{:?}", c.argb()),
            None => String::new(),
        };
        if value.is_empty() && fill.is_empty() {
            continue;
        }
        writeln!(
            out,
            "{}{row}  {value}  [{format}]{fill}",
            column_letters(col)
        )
        .unwrap();
    }
}

/// 1 -> A, 27 -> AA.
fn column_letters(mut index: u32) -> String {
    let mut letters = String::new();
    while index > 0 {
        let rem = (index - 1) % 26;
        letters.insert(0, (b'A' + rem as u8) as char);
        index = (index - 1) / 26;
    }
    letters
}

/// The first differing lines, so a failure says what changed rather than dumping two files.
fn first_differences(expected: &str, actual: &str, limit: usize) -> Vec<String> {
    expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .filter(|(_, (e, a))| e != a)
        .take(limit)
        .map(|(i, (e, a))| format!("  line {}:\n    golden: {e}\n    actual: {a}", i + 1))
        .chain(
            match expected.lines().count().cmp(&actual.lines().count()) {
                std::cmp::Ordering::Equal => None,
                _ => Some(format!(
                    "  line count: golden {}, actual {}",
                    expected.lines().count(),
                    actual.lines().count()
                )),
            },
        )
        .collect()
}
