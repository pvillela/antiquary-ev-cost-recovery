//! Writing the workbook.
//!
//! Layout lives in the two `COLUMNS` tables and nowhere else. Each drives its sheet's header rows,
//! its data rows, its column widths and its number formats together, so adding or moving a column
//! is one edit rather than four that have to agree.
//!
//! Formatting reproduces `bak/Green_Button_Peak_Values-template.xlsx`, the hand-formatted workbook
//! the Python filled in place. Three deliberate departures from it: the `kW at interval` header
//! over `max_kva_kw` is corrected (the template said `kVA at interval`, which is the wrong unit),
//! the `kw` and `kva` columns on `Interval_values` get the same explicit width as `kwh` instead of
//! inheriting the default, and machine names are `lower_snake_case` throughout so that reading a
//! sheet back by column name cannot be defeated by a capitalisation difference.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::Path;

use rust_xlsxwriter::{Color, Format, FormatAlign, Workbook, Worksheet};

use crate::{
    Anomaly, Feed, Peak, PeriodValues, Reading, Readings, excel_serial, excel_serial_date,
    excel_serial_local, period_values,
};

const DATE_FORMAT: &str = "yyyy/mm/dd";
const COUNT_FORMAT: &str = "#,##0";
const NUM_FORMAT: &str = "#,##0.000";
/// Local-time columns carry the weekday, which is what makes a peak at 02:00 on a Sunday obvious.
const LOCAL_DT_FORMAT: &str = r"yyyy/mm/dd\ hh:mm\ ddd";
const UTC_DT_FORMAT: &str = r"yyyy/mm/dd\ hh:mm";

/// Excel's stock "Light Red Fill" background. Applied to an interval count that is not what a
/// complete period should hold, and to any non-empty anomalies cell.
const LIGHT_RED: Color = Color::RGB(0xFF_C7CE);

const FONT: &str = "Arial";

/// How a column is formatted. Every column's number format and alignment follows from this.
#[derive(Clone, Copy, PartialEq, Eq)]
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

    let styles = Styles::new();
    let mut workbook = Workbook::new();

    let peak_rows: Vec<Vec<Out>> = periods.iter().rev().map(|p| peak_row(p, feed)).collect();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Peak_values")?;
    write_sheet(
        sheet,
        "PEAK VALUES",
        &styles.title_peak,
        PEAK_COLUMNS,
        true,
        &peak_rows,
        &styles,
    )?;

    let interval_rows: Vec<Vec<Out>> = readings
        .rows
        .iter()
        .rev()
        .map(|r| interval_row(r, readings.anomalies.get(&r.start), feed))
        .collect();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Interval_values")?;
    write_sheet(
        sheet,
        "INTERVAL VALUES",
        &styles.title_interval,
        INTERVAL_COLUMNS,
        false,
        &interval_rows,
        &styles,
    )?;

    workbook.save(path)?;
    Ok(report)
}

/// Every format the workbook uses, built once.
struct Styles {
    title_peak: Format,
    title_interval: Format,
    header: Format,
    machine: Format,
    date: Format,
    count: Format,
    count_filled: Format,
    num: Format,
    local_dt: Format,
    utc_dt: Format,
    text: Format,
    text_filled: Format,
}

impl Styles {
    fn new() -> Self {
        let base = || Format::new().set_font_name(FONT).set_font_size(10);
        let centred = || base().set_align(FormatAlign::Center);
        Self {
            title_peak: Format::new()
                .set_font_name(FONT)
                .set_font_size(12)
                .set_bold(),
            title_interval: Format::new()
                .set_font_name(FONT)
                .set_font_size(13)
                .set_bold(),
            header: base()
                .set_bold()
                .set_align(FormatAlign::Center)
                .set_align(FormatAlign::Top)
                .set_text_wrap(),
            machine: Format::new()
                .set_font_name(FONT)
                .set_font_size(7)
                .set_bold(),
            date: base()
                .set_num_format(DATE_FORMAT)
                .set_align(FormatAlign::Left),
            count: centred().set_num_format(COUNT_FORMAT),
            count_filled: centred()
                .set_num_format(COUNT_FORMAT)
                .set_background_color(LIGHT_RED),
            num: centred().set_num_format(NUM_FORMAT),
            local_dt: centred().set_num_format(LOCAL_DT_FORMAT),
            utc_dt: centred().set_num_format(UTC_DT_FORMAT),
            text: centred(),
            text_filled: centred().set_background_color(LIGHT_RED),
        }
    }

    fn for_cell(&self, kind: Kind, fill: bool) -> Option<&Format> {
        Some(match (kind, fill) {
            (Kind::Spacer, _) => return None,
            (Kind::Date, _) => &self.date,
            (Kind::Count, false) => &self.count,
            (Kind::Count, true) => &self.count_filled,
            (Kind::Num, _) => &self.num,
            (Kind::LocalDt, _) => &self.local_dt,
            (Kind::UtcDt, _) => &self.utc_dt,
            (Kind::Text, false) => &self.text,
            (Kind::Text, true) => &self.text_filled,
        })
    }
}

/// Writes a title row, header row(s) and the data, driven by the column table.
///
/// `machine_row` distinguishes the two sheets: `Peak_values` carries human headers on row 3 and
/// machine names on row 4, `Interval_values` only the one row of names.
fn write_sheet(
    sheet: &mut Worksheet,
    title: &str,
    title_format: &Format,
    columns: &[Col],
    machine_row: bool,
    rows: &[Vec<Out>],
    styles: &Styles,
) -> Result<(), Box<dyn Error>> {
    sheet.write_string_with_format(0, 0, title, title_format)?;

    let header_row: u32 = 2;
    for (i, column) in columns.iter().enumerate() {
        let c = i as u16;
        sheet.set_column_width(c, column.width)?;
        if column.kind == Kind::Spacer {
            continue;
        }
        sheet.write_string_with_format(header_row, c, column.header, &styles.header)?;
        if machine_row {
            sheet.write_string_with_format(header_row + 1, c, column.machine, &styles.machine)?;
        }
    }

    // Match the template's header geometry: the wrapped header row is taller, the rest are not.
    sheet.set_row_height(0, if machine_row { 15.0 } else { 16.15 })?;
    sheet.set_row_height(header_row, if machine_row { 24.4 } else { 13.8 })?;

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
            let c = i as u16;
            let Some(format) = styles.for_cell(columns[i].kind, out.fill) else {
                continue;
            };
            match &out.cell {
                Cell::Blank => {
                    if out.fill {
                        sheet.write_blank(excel_row, c, format)?;
                    }
                }
                Cell::Num(v) => sheet.write_number_with_format(excel_row, c, *v, format)?,
                Cell::Text(s) => sheet.write_string_with_format(excel_row, c, s, format)?,
            };
        }
    }

    // One column and three rows, as in the template. Peak_values freezes at row 3 even though its
    // data starts at row 5, so the machine-name row scrolls away and the human headers stay.
    sheet.set_freeze_panes(3, 1)?;
    Ok(())
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

    #[test]
    fn anomaly_counts_render_with_their_totals() {
        let counts = BTreeMap::from([(Anomaly::MissingKw, 2), (Anomaly::MissingInterval, 3)]);
        assert_eq!(format_counts(&counts), "MissingKw(2),MissingInterval(3)");
        assert_eq!(format_counts(&BTreeMap::new()), "");
    }
}
