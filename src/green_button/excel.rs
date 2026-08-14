//! Writing the workbook.
//!
//! Layout lives in the two `COLUMNS` tables and nowhere else. Each drives its sheet's header rows,
//! its data rows, its column widths and its number formats together, so adding or moving a column
//! is one edit rather than four that have to agree.
//!
//! Formatting descends from the hand-formatted workbook the Python filled in place, kept as
//! `docs/reference/Green_Button_Peak_Values-python-2026-07-16.xlsx`, but does not copy it
//! slavishly: that workbook stamps a row height on all 13,924 of its rows because that is what
//! LibreOffice writes, and only three of them were ever a decision. See
//! `docs/maintenance-manual.md` §7. The current standard is `tests/fixtures/billed_period.xlsx`,
//! regenerated with the goldens.
//!
//! `umya-spreadsheet` is used rather than `rust_xlsxwriter` because it stores row heights and
//! column widths as `f64` written straight through, whereas `rust_xlsxwriter` models them as whole
//! pixels — `(height * 4.0 / 3.0).round() as u32` — so the reference's 1.39-wide spacers are not
//! representable there at all. It is also the crate `ev-peak-contrib` uses.
//!
//! Alignment follows the column: the template left-aligns everything in column A — title, header,
//! machine name and data alike — and centres every other column. That is why [`Kind`] carries the
//! alignment rather than the row deciding it.
//!
//! Three deliberate departures from the template: the `kW at interval` header over `max_kva_kw` is
//! corrected (the template said `kVA at interval`, which is the wrong unit), the `kw` and `kva`
//! columns on `Interval_values` get the same explicit width as `kwh` instead of inheriting the
//! default, and machine names are `lower_snake_case` throughout so that reading a sheet back by
//! column name cannot be defeated by a capitalisation difference.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

use umya_spreadsheet::{
    HorizontalAlignmentValues, Pane, PaneStateValues, PaneValues, VerticalAlignmentValues,
    Worksheet, writer,
};

use crate::green_button::{
    Anomaly, Feed, Peak, PeriodValues, Reading, excel_serial, excel_serial_date,
    excel_serial_local, period_values,
};

const GENERAL_FORMAT: &str = "General";
const DATE_FORMAT: &str = "yyyy/mm/dd";
const COUNT_FORMAT: &str = "#,##0";
const NUM_FORMAT: &str = "#,##0.000";
/// Local-time columns carry the weekday, which is what makes a peak at 02:00 on a Sunday obvious.
const LOCAL_DT_FORMAT: &str = r"yyyy/mm/dd\ hh:mm\ ddd";
const UTC_DT_FORMAT: &str = r"yyyy/mm/dd\ hh:mm";

/// Excel's stock "Light Red Fill" background. Applied to an interval count that is not what a
/// complete period should hold, and to any non-empty anomalies cell.
const LIGHT_RED: &str = "FFFFC7CE";

const FONT: &str = "Arial";

/// Row heights, in points.
///
/// Only three rows in the workbook carry a stored height: the two titles and the wrapped header
/// row. Everything else — the blank row, the machine-name row, and every data row on both sheets —
/// takes [`DEFAULT_ROW_HEIGHT`].
///
/// The reference workbook instead stamps a height on all 13,924 of its rows, which is what
/// LibreOffice writes rather than a decision anyone made. Reproducing that was a mistake: it buries
/// the three heights that are actually chosen among thousands that are not, and it made the two
/// files disagree in ways that only showed up on screen.
const DEFAULT_ROW_HEIGHT: f64 = 13.8;

/// Both sheet titles, Arial 12 bold. The reference set `Interval_values` a point larger than
/// `Peak_values` for no reason that survives inspection.
const TITLE_HEIGHT: f64 = 15.0;
const TITLE_FONT_SIZE: f64 = 12.0;

/// `Peak_values` row 3 only: two wrapped lines of Arial 10 bold.
///
/// This is the one genuinely content-dependent height in the workbook. It fits two lines at the
/// current column widths; widen a column enough that a header collapses to one line, or narrow one
/// so it needs three, and this wants revisiting.
const PEAK_HEADER_HEIGHT: f64 = 24.0;

const DEFAULT_COL_WIDTH: f64 = 8.6796875;

/// How a column is formatted. Both its number format and its alignment follow from this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Date,
    Count,
    Num,
    LocalDt,
    UtcDt,
    Text,
    /// A narrow empty column separating the value groups, as in the template.
    Spacer,
}

impl Kind {
    fn number_format(self) -> &'static str {
        match self {
            Self::Date => DATE_FORMAT,
            Self::Count => COUNT_FORMAT,
            Self::Num => NUM_FORMAT,
            Self::LocalDt => LOCAL_DT_FORMAT,
            Self::UtcDt => UTC_DT_FORMAT,
            Self::Text | Self::Spacer => GENERAL_FORMAT,
        }
    }

    /// The template left-aligns the billing-period column and centres everything else, in every
    /// row of the sheet rather than only in the data rows.
    fn horizontal(self) -> HorizontalAlignmentValues {
        match self {
            Self::Date => HorizontalAlignmentValues::Left,
            _ => HorizontalAlignmentValues::Center,
        }
    }
}

struct Col {
    /// Row-4 machine name. Empty for a spacer.
    machine: &'static str,
    /// Row-3 human header, worded as the Toronto Hydro invoice words it.
    header: &'static str,
    kind: Kind,
    width: f64,
}

const fn col(machine: &'static str, header: &'static str, kind: Kind, width: f64) -> Col {
    Col {
        machine,
        header,
        kind,
        width,
    }
}

const SPACER: Col = col("", "", Kind::Spacer, 1.39);

/// `Peak_values`, in order. Four value groups, each ending in the TOU period its interval fell in.
const PEAK_COLUMNS: &[Col] = &[
    col(
        "billing_period_ending",
        "Billing period ending",
        Kind::Date,
        14.14,
    ),
    col(
        "nbr_of_intervals",
        "Number of intervals",
        Kind::Count,
        10.35,
    ),
    SPACER,
    col("kwh", "kWh used", Kind::Num, 14.01),
    SPACER,
    col("max_kw", "Demand kW", Kind::Num, 9.72),
    col(
        "max_kw_interval",
        "Demand kW interval (local time)",
        Kind::LocalDt,
        20.45,
    ),
    col(
        "max_kw_interval_utc",
        "Demand kW interval (UTC)",
        Kind::UtcDt,
        18.44,
    ),
    col("max_kw_kva", "kVA at interval", Kind::Num, 9.72),
    col("max_kw_tou", "TOU", Kind::Text, 9.72),
    SPACER,
    col("max_kw_nop", "Peak kW 7-7", Kind::Num, 9.72),
    col(
        "max_kw_nop_interval",
        "Peak kW 7-7 interval (local time)",
        Kind::LocalDt,
        20.45,
    ),
    col(
        "max_kw_nop_interval_utc",
        "Peak kW 7-7 interval (UTC)",
        Kind::UtcDt,
        18.44,
    ),
    col("max_kw_nop_kva", "kVA at interval", Kind::Num, 9.72),
    col("max_kw_nop_tou", "TOU", Kind::Text, 9.72),
    SPACER,
    col("max_kva", "Demand kVA", Kind::Num, 9.72),
    col(
        "max_kva_interval",
        "Demand kVA interval (local time)",
        Kind::LocalDt,
        20.45,
    ),
    col(
        "max_kva_interval_utc",
        "Demand kVA interval (UTC)",
        Kind::UtcDt,
        18.44,
    ),
    // The template labelled this "kVA at interval"; the value is a kW.
    col("max_kva_kw", "kW at interval", Kind::Num, 9.72),
    col("max_kva_tou", "TOU", Kind::Text, 9.72),
    SPACER,
    col("max_kva_nop", "Peak kVA 7-7", Kind::Num, 9.72),
    col(
        "max_kva_nop_interval",
        "Peak kVA 7-7 interval (local time)",
        Kind::LocalDt,
        20.45,
    ),
    col(
        "max_kva_nop_interval_utc",
        "Peak kVA 7-7 interval (UTC)",
        Kind::UtcDt,
        18.44,
    ),
    col("max_kva_nop_kw", "kW at interval", Kind::Num, 9.72),
    col("max_kva_nop_tou", "TOU", Kind::Text, 9.72),
    SPACER,
    col("anomalies", "Anomalies", Kind::Text, 28.0),
];

/// `Interval_values`, in order. One header row, since every name here is already the machine name.
const INTERVAL_COLUMNS: &[Col] = &[
    col("interval", "interval", Kind::LocalDt, 20.45),
    col("interval_utc", "interval_utc", Kind::UtcDt, 18.44),
    col("kwh", "kwh", Kind::Num, 11.38),
    col("kw", "kw", Kind::Num, 11.38),
    col("kva", "kva", Kind::Num, 11.38),
    col("anomalies", "anomalies", Kind::Text, 28.0),
];

/// One cell's content. Dates and date-times are serials, told apart only by their number format —
/// exactly as the template stores them, and why no cell here carries a time zone.
enum Cell {
    Blank,
    Num(f64),
    Text(String),
}

/// A cell plus whether it should be highlighted.
struct Out {
    cell: Cell,
    fill: bool,
}

impl Out {
    fn plain(cell: Cell) -> Self {
        Self { cell, fill: false }
    }
    fn blank() -> Self {
        Self::plain(Cell::Blank)
    }
    fn num(v: f64) -> Self {
        Self::plain(Cell::Num(v))
    }
    fn text(s: impl Into<String>) -> Self {
        Self::plain(Cell::Text(s.into()))
    }
}

/// What was written, for the CLI to report.
#[derive(Debug, Clone, Default)]
pub struct WriteReport {
    pub interval_rows: usize,
    pub period_rows: usize,
    /// Periods whose interval count is not what a complete period should hold.
    pub incomplete_periods: usize,
    pub anomaly_counts: BTreeMap<Anomaly, usize>,
}

/// Builds the workbook and writes it to `path`.
///
/// # Errors
///
/// Returns an error if the workbook cannot be built or the file cannot be written. It is the
/// caller's job to have established that `path` does not already exist.
pub fn write_workbook(path: &Path, feed: &Feed) -> Result<WriteReport, Box<dyn Error>> {
    let readings = feed.readings();
    let periods = period_values(&readings);

    let mut report = WriteReport {
        interval_rows: readings.rows.len(),
        period_rows: periods.len(),
        incomplete_periods: periods.iter().filter(|p| !p.is_complete()).count(),
        anomaly_counts: BTreeMap::new(),
    };
    for kinds in readings.anomalies.values() {
        for kind in kinds {
            *report.anomaly_counts.entry(*kind).or_default() += 1;
        }
    }

    let mut book = umya_spreadsheet::new_file_empty_worksheet();

    let peak_rows: Vec<Vec<Out>> = periods.iter().rev().map(|p| peak_row(p, feed)).collect();
    let sheet = book.new_sheet("Peak_values")?;
    write_sheet(sheet, "PEAK VALUES", PEAK_COLUMNS, true, &peak_rows);

    let interval_rows: Vec<Vec<Out>> = readings
        .rows
        .iter()
        .rev()
        .map(|r| interval_row(r, readings.anomalies.get(&r.start), feed))
        .collect();
    let sheet = book.new_sheet("Interval_values")?;
    write_sheet(
        sheet,
        "INTERVAL VALUES",
        INTERVAL_COLUMNS,
        false,
        &interval_rows,
    );

    writer::xlsx::write(&book, path)?;
    Ok(report)
}

/// Writes a title row, header row(s) and the data, driven by the column table.
///
/// `machine_row` distinguishes the two sheets: `Peak_values` carries human headers on row 3 and
/// machine names on row 4, `Interval_values` only the one row of names.
fn write_sheet(
    sheet: &mut Worksheet,
    title: &str,
    columns: &[Col],
    machine_row: bool,
    rows: &[Vec<Out>],
) {
    let properties = sheet.sheet_format_properties_mut();
    properties.set_default_row_height(DEFAULT_ROW_HEIGHT);
    properties.set_default_column_width(DEFAULT_COL_WIDTH);

    // The title is left-aligned on both sheets regardless of what its column does: Peak_values
    // left-aligns its whole first column, Interval_values centres it, and both titles are left.
    set_title(sheet, title);
    style_font(sheet, 1, 1, TITLE_FONT_SIZE, true);
    set_row_height(sheet, 1, TITLE_HEIGHT);

    let header_row: u32 = 3;
    for (i, column) in columns.iter().enumerate() {
        let c = i as u32 + 1;
        sheet
            .column_dimension_mut(&column_letters(c))
            .set_width(column.width);
        if column.kind == Kind::Spacer {
            continue;
        }
        set_label(sheet, c, header_row, column.header, column.kind);
        style_font(sheet, c, header_row, 10.0, true);
        // The human-header row wraps and sits at the top of its cell; nothing else does.
        let alignment = sheet.style_mut((c, header_row)).alignment_mut();
        alignment.set_vertical(VerticalAlignmentValues::Top);
        alignment.set_wrap_text(true);

        if machine_row {
            set_label(sheet, c, header_row + 1, column.machine, column.kind);
            style_font(sheet, c, header_row + 1, 7.0, true);
        }
    }
    // The wrapped human-header row is the only row below the title that needs its own height. The
    // blank row, the machine-name row and every data row take the sheet default.
    if machine_row {
        set_row_height(sheet, header_row, PEAK_HEADER_HEIGHT);
    }

    let first_data_row = if machine_row {
        header_row + 2
    } else {
        header_row + 1
    };
    for (r, row) in rows.iter().enumerate() {
        debug_assert_eq!(
            row.len(),
            columns.len(),
            "a row must match the column table"
        );
        let excel_row = first_data_row + r as u32;
        for (i, out) in row.iter().enumerate() {
            let c = i as u32 + 1;
            let kind = columns[i].kind;
            if kind == Kind::Spacer {
                continue;
            }
            match &out.cell {
                Cell::Blank => {
                    if out.fill {
                        style_cell(sheet, c, excel_row, kind, true);
                    }
                }
                Cell::Num(v) => {
                    sheet.cell_mut((c, excel_row)).set_value_number(*v);
                    style_cell(sheet, c, excel_row, kind, out.fill);
                    style_font(sheet, c, excel_row, 10.0, false);
                }
                Cell::Text(s) => {
                    set_text(sheet, c, excel_row, s, kind, out.fill);
                    style_font(sheet, c, excel_row, 10.0, false);
                }
            }
        }
    }

    freeze_panes(sheet);
}

fn set_text(sheet: &mut Worksheet, col: u32, row: u32, text: &str, kind: Kind, fill: bool) {
    sheet.cell_mut((col, row)).set_value_string(text);
    style_cell(sheet, col, row, kind, fill);
}

/// Fixes a row's height.
///
/// `Row::set_height` also sets `customHeight`, which tells the application the height was chosen
/// deliberately and must not be auto-fitted. That is wanted here: the only rows given a height are
/// the three that are genuinely chosen, and a pinned height is what stops them drifting.
///
/// The rule the workbook follows, and the one to keep: a row either has a pinned height because
/// somebody decided it, or it has no stored height at all and takes the sheet default. The
/// half-state — a stored height that the application is free to re-fit — is what made two files
/// with identical stored numbers render differently.
fn set_row_height(sheet: &mut Worksheet, row: u32, height: f64) {
    sheet.row_dimension_mut(row).set_height(height);
}

/// The sheet title in A1: always left, never a number format.
fn set_title(sheet: &mut Worksheet, title: &str) {
    sheet.cell_mut((1u32, 1u32)).set_value_string(title);
    let style = sheet.style_mut((1u32, 1u32));
    style.number_format_mut().set_format_code(GENERAL_FORMAT);
    style
        .alignment_mut()
        .set_horizontal(HorizontalAlignmentValues::Left);
}

/// A header or machine-name cell: the column's alignment, but no number format.
///
/// The template does carry the column's number format on these cells, an artefact of how
/// LibreOffice applies column formatting, and its own column A carries `General` regardless. A
/// number format has no effect on a text cell, so reproducing that would mean a special case for
/// no visible difference.
fn set_label(sheet: &mut Worksheet, col: u32, row: u32, text: &str, kind: Kind) {
    sheet.cell_mut((col, row)).set_value_string(text);
    let style = sheet.style_mut((col, row));
    style.number_format_mut().set_format_code(GENERAL_FORMAT);
    style.alignment_mut().set_horizontal(kind.horizontal());
}

fn style_cell(sheet: &mut Worksheet, col: u32, row: u32, kind: Kind, fill: bool) {
    let style = sheet.style_mut((col, row));
    style
        .number_format_mut()
        .set_format_code(kind.number_format());
    style.alignment_mut().set_horizontal(kind.horizontal());
    if fill {
        style.set_background_color(LIGHT_RED);
    }
}

fn style_font(sheet: &mut Worksheet, col: u32, row: u32, size: f64, bold: bool) {
    let font = sheet.style_mut((col, row)).font_mut();
    font.set_name(FONT);
    font.set_size(size);
    font.set_bold(bold);
}

/// One column and three rows, as in the template. `Peak_values` freezes at row 3 even though its
/// data starts at row 5, so the machine-name row scrolls away and the human headers stay.
fn freeze_panes(sheet: &mut Worksheet) {
    let mut top_left = umya_spreadsheet::Coordinate::default();
    top_left.set_col_num(2);
    top_left.set_row_num(4);

    let mut pane = Pane::default();
    pane.set_horizontal_split(1.0);
    pane.set_vertical_split(3.0);
    pane.set_top_left_cell(top_left);
    pane.set_active_pane(PaneValues::BottomRight);
    pane.set_state(PaneStateValues::Frozen);

    let views = sheet.sheet_views_mut().sheet_view_list_mut();
    if views.is_empty() {
        views.push(umya_spreadsheet::SheetView::default());
    }
    views[0].set_pane(pane);
}

fn peak_row(v: &PeriodValues, feed: &Feed) -> Vec<Out> {
    let mut row = Vec::with_capacity(PEAK_COLUMNS.len());
    row.push(Out::num(excel_serial_date(v.period.ending)));
    row.push(Out {
        cell: Cell::Num(v.interval_count as f64),
        fill: !v.is_complete(),
    });
    row.push(Out::blank()); // spacer
    row.push(Out::num(v.kwh_total as f64 / feed.kwh.divisor()));
    row.push(Out::blank()); // spacer

    for (peak, value_divisor, companion_divisor) in [
        (&v.max_kw, feed.kw.divisor(), feed.kva.divisor()),
        (&v.max_kw_nop, feed.kw.divisor(), feed.kva.divisor()),
        (&v.max_kva, feed.kva.divisor(), feed.kw.divisor()),
        (&v.max_kva_nop, feed.kva.divisor(), feed.kw.divisor()),
    ] {
        push_peak(&mut row, peak.as_ref(), value_divisor, companion_divisor);
        row.push(Out::blank()); // spacer
    }

    let anomalies = format_counts(&v.anomaly_counts);
    row.push(Out {
        fill: !anomalies.is_empty(),
        cell: Cell::Text(anomalies),
    });
    row
}

/// The five cells of one value group: the maximum, when it occurred in local and UTC time, the
/// companion figure at that interval, and its price period.
fn push_peak(row: &mut Vec<Out>, peak: Option<&Peak>, value_divisor: f64, companion_divisor: f64) {
    match peak {
        Some(p) => {
            row.push(Out::num(p.value as f64 / value_divisor));
            row.push(Out::num(excel_serial_local(p.at)));
            row.push(Out::num(excel_serial(p.at)));
            row.push(match p.companion {
                Some(c) => Out::num(c as f64 / companion_divisor),
                None => Out::blank(),
            });
            row.push(Out::text(p.tou.as_str()));
        }
        // A period with no interval in the demand window at all leaves the whole group blank,
        // rather than borrowing the unrestricted figure.
        None => row.extend((0..5).map(|_| Out::blank())),
    }
}

fn interval_row(r: &Reading, anomalies: Option<&BTreeSet<Anomaly>>, feed: &Feed) -> Vec<Out> {
    let value = |raw: Option<i64>, divisor: f64| match raw {
        Some(v) => Out::num(v as f64 / divisor),
        None => Out::blank(),
    };
    let tokens = anomalies
        .map(|a| a.iter().map(Anomaly::as_str).collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    vec![
        Out::num(excel_serial_local(r.start)),
        Out::num(excel_serial(r.start)),
        value(r.kwh, feed.kwh.divisor()),
        value(r.kw, feed.kw.divisor()),
        value(r.kva, feed.kva.divisor()),
        Out {
            fill: !tokens.is_empty(),
            cell: Cell::Text(tokens),
        },
    ]
}

/// `MissingKw(2),MissingInterval(3)` — the per-period roll-up of what went wrong in its hours.
fn format_counts(counts: &BTreeMap<Anomaly, usize>) -> String {
    counts
        .iter()
        .map(|(kind, n)| format!("{}({n})", kind.as_str()))
        .collect::<Vec<_>>()
        .join(",")
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

// cargo test --package green-button --lib -- excel::test --nocapture
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_column_tables_have_unique_machine_names() {
        for columns in [PEAK_COLUMNS, INTERVAL_COLUMNS] {
            let names: Vec<&str> = columns
                .iter()
                .filter(|c| c.kind != Kind::Spacer)
                .map(|c| c.machine)
                .collect();
            let unique: BTreeSet<&str> = names.iter().copied().collect();
            assert_eq!(
                names.len(),
                unique.len(),
                "duplicate machine name in {names:?}"
            );
        }
    }

    /// Reading these sheets back by name is the whole reason for the naming rules, and a
    /// capitalisation difference between two columns is exactly what would defeat it.
    #[test]
    fn every_machine_name_is_lower_snake_case() {
        for columns in [PEAK_COLUMNS, INTERVAL_COLUMNS] {
            for c in columns.iter().filter(|c| c.kind != Kind::Spacer) {
                assert!(
                    c.machine
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
                    "{} is not lower_snake_case",
                    c.machine
                );
            }
        }
    }

    #[test]
    fn the_peak_sheet_has_the_expected_shape() {
        assert_eq!(PEAK_COLUMNS.len(), 30);
        assert_eq!(
            PEAK_COLUMNS
                .iter()
                .filter(|c| c.kind == Kind::Spacer)
                .count(),
            6
        );
        assert_eq!(
            PEAK_COLUMNS
                .iter()
                .filter(|c| c.machine.ends_with("_tou"))
                .count(),
            4
        );
    }

    /// The template left-aligns the billing-period column throughout and centres everything else.
    #[test]
    fn only_the_billing_period_column_is_left_aligned() {
        assert_eq!(Kind::Date.horizontal(), HorizontalAlignmentValues::Left);
        for kind in [
            Kind::Count,
            Kind::Num,
            Kind::LocalDt,
            Kind::UtcDt,
            Kind::Text,
        ] {
            assert_eq!(kind.horizontal(), HorizontalAlignmentValues::Center);
        }
        assert_eq!(PEAK_COLUMNS[0].kind, Kind::Date);
        assert_eq!(
            PEAK_COLUMNS.iter().filter(|c| c.kind == Kind::Date).count(),
            1
        );
    }

    #[test]
    fn anomaly_counts_render_with_their_totals() {
        let counts = BTreeMap::from([(Anomaly::MissingKw, 2), (Anomaly::MissingInterval, 3)]);
        assert_eq!(format_counts(&counts), "MissingKw(2),MissingInterval(3)");
        assert_eq!(format_counts(&BTreeMap::new()), "");
    }

    #[test]
    fn column_letters_pass_z() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(30), "AD");
    }
}
