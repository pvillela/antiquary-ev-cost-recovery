//! The `.xlsx` rendering of a session report, written and read back.
//!
//! Both directions live here; neither parses a CSV. [`session_csv_to_xlsx`] takes the rows
//! [`super::csv`] has already resolved and lays them out as cells, formats, formulas and comments.
//! [`session_list`] reads such a workbook back, recomputing every derived figure and logging any
//! stored cell that disagrees.
//!
//! The `anomalies` column is the channel between the two: a judgement call made when the CSV was
//! parsed is written there and read back here, and it is what removes a session from the
//! estimates. See [`AnomalyKind`].

use crate::time::{
    duration_of_serial, instant_of_serial, serial_of_civil, serial_of_duration, serial_of_instant,
};

use super::csv::{self, SessionRows};
use super::{Anomaly, AnomalyKind, MergedSessions, RSession, RunLog, Session, SessionReport};
use jiff::Timestamp;
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    path::{Path, PathBuf},
    rc::Rc,
};
use umya_spreadsheet::{Comment, HorizontalAlignmentValues, Workbook, Worksheet};

const DATETIME_FORMAT: &str = "yyyy-mm-dd hh:mm:ss ddd";
/// Elapsed-time format: unlike `hh:mm:ss` it does not wrap a 25-hour duration to `01:00:00`.
const DURATION_FORMAT: &str = "[h]:mm:ss";
const ENERGY_USE_FORMAT: &str = "0.000";
const AVG_KW_FORMAT: &str = "0.000";
const TOTAL_FEE_FORMAT: &str = "0.00";

/// Outcome of converting one CSV file. The reading direction returns a [`SessionReport`] instead.
#[derive(Debug)]
pub struct ConversionReport {
    /// Where the workbook was written.
    pub output_path: PathBuf,
    /// Rows that needed a judgement call. Empty for a clean conversion.
    pub anomalies: Vec<Anomaly>,
    /// Where the run log was written. It always exists, and says either that nothing was found or
    /// what was.
    pub log_path: PathBuf,
}

/// How an output column is populated.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Copied verbatim from the named CSV column.
    Text(&'static str),
    /// Parsed from the named CSV column and written as a number.
    Number(&'static str),
    /// Parsed from the named CSV column and written as an Excel duration.
    Duration(&'static str),
    /// The session id, which carries a `-EDT`/`-EST` suffix on duplicated records.
    SessionId,
    ConnStartLocal,
    ConnStartUtc,
    ConnEndLocal,
    ConnEndUtc,
    AdjConnStartLocal,
    AdjConnStartUtc,
    AdjConnEndLocal,
    AdjConnEndUtc,
    /// Formula: `adj_conn_end_utc - adj_conn_start_utc`.
    AdjConnDuration,
    /// Formula: `Energy_Use / Active_Charge_Time`, in kW.
    AvgKw,
    /// Comma-separated [`AnomalyKind`] tokens for this row; empty when the row is clean.
    Anomalies,
}

/// The output sheet's columns, in order. Drives both the header row and every data row,
/// so layout changes need only happen here.
const COLUMNS: &[(&str, Source)] = &[
    ("UR_ID", Source::Text("UR_ID")),
    ("Location_Address", Source::Text("Location_Address")),
    ("Location_City", Source::Text("Location_City")),
    ("Location_Postal_Code", Source::Text("Location_Postal_Code")),
    ("Station_ID", Source::Text("Station_ID")),
    (
        "Station_Network_Provider",
        Source::Text("Station_Network_Provider"),
    ),
    ("Station_Make", Source::Text("Station_Make")),
    ("Station_Model", Source::Text("Station_Model")),
    ("Charge_Session_ID", Source::SessionId),
    ("User_ID", Source::Text("User_ID")),
    ("Conn_DateTime_Start", Source::ConnStartLocal),
    ("Conn_DateTime_End", Source::ConnEndLocal),
    ("Conn_Duration", Source::Duration("Conn_Duration")),
    ("Charge_Duration", Source::Duration("Charge_Duration")),
    ("Active_Charge_Time", Source::Duration("Active_Charge_Time")),
    ("Charging_Level", Source::Text("Charging_Level")),
    ("Energy_Use", Source::Number("Energy_Use")),
    ("Total_Fee", Source::Number("Total_Fee")),
    ("Vehicle_Make", Source::Text("Vehicle_Make")),
    ("Vehicle_Model", Source::Text("Vehicle_Model")),
    ("Vehicle_Year", Source::Number("Vehicle_Year")),
    // Everything from here on is this software's, not Evolute's. Grouped at the right end so the
    // left of the sheet is the session report as received and a reader can tell at a glance which
    // is which.
    ("adj_conn_start", Source::AdjConnStartLocal),
    ("adj_conn_end", Source::AdjConnEndLocal),
    ("conn_start_utc", Source::ConnStartUtc),
    ("conn_end_utc", Source::ConnEndUtc),
    ("adj_conn_start_utc", Source::AdjConnStartUtc),
    ("adj_conn_end_utc", Source::AdjConnEndUtc),
    ("adj_conn_duration", Source::AdjConnDuration),
    ("avg_kw", Source::AvgKw),
    ("anomalies", Source::Anomalies),
];

/// Reads the CSV file at `path`, which should have the same format as one on this project's `data`
/// directory, and transforms it into a `.xlsx` file saved to the same directory as the input file,
/// with the extension replaced.
///
/// The parse is [`csv::session_rows`]; nothing about the session report is interpreted here. The
/// domain rules — the UTC conversion and its DST policy, the definitions of `adj_conn_end` and
/// `adj_conn_duration`, and the treatment of zero-`Energy_Use` sessions — are specified in
/// `docs/time/README.md` under "Time zone" and in `docs/sessions/README.md` under "Excel workbook"
/// and "Other". They are shared with the peak power contribution logic and are not restated here.
///
/// What this function adds on top of those rules:
///
/// - Column order is given by the private `COLUMNS` table: the session report's own columns first,
///   in the order it states them, then everything this software derives. See
///   `docs/sessions/README.md`, "Excel workbook".
/// - Timestamp columns are Excel date/time numbers formatted `yyyy-mm-dd hh:mm:ss ddd`, left-
///   justified; duration columns are Excel durations formatted `[h]:mm:ss`, which does not wrap
///   past 24 hours, and are centered.
/// - `adj_conn_duration` and `avg_kw` are live formulas. `adj_conn_duration` subtracts the two
///   *UTC* columns, so it is true elapsed time even across a DST fold; `avg_kw` is
///   `=Energy_Use/(Active_Charge_Time*24)`, in kW, displayed to 3 decimal
///   places, matching `Energy_Use`. The formula is written on every row, so a session with
///   zero `Active_Charge_Time` shows `#DIV/0!` rather than an empty cell:
///   it delivered energy in no time at all, and the sheet says so. `Total_Fee` is displayed to
///   2 decimal places.
/// - The last column, `anomalies`, carries the [`AnomalyKind`]s found for the row as a
///   comma-separated list of variant names. [`session_list`] reads it back, so it is the channel
///   by which a judgement call made during the parse reaches the peak power contribution logic.
/// - The remaining columns are copied over with an explicit per-column type, so values that merely
///   look numeric — postal codes, station ids — keep their text form.
/// - The sheet is named by the private `sheet_name`.
///
/// Every session in the report is written to the workbook, anomalous ones included: the sheet is a
/// faithful rendering of the session report, and which sessions take part in an estimate is decided
/// on the reading side. A caller that wants the sessions and not the sheet can skip the workbook
/// entirely and call [`csv::session_list`], which runs the same parse.
///
/// # Errors
///
/// Returns `Err` only for conditions that invalidate the whole file: it cannot be read, a required
/// header is missing, a timestamp or duration does not parse, or the workbook cannot be written.
/// Per-row judgement calls do not abort the conversion; they are collected in
/// [`ConversionReport::anomalies`].
pub fn session_csv_to_xlsx(path: &Path) -> Result<ConversionReport, Box<dyn Error>> {
    let rows = csv::session_rows(path)?;

    let output_path = path.with_extension("xlsx");
    let mut book = umya_spreadsheet::new_file();
    write_sheet(&mut book, &output_path, &rows)?;

    umya_spreadsheet::writer::xlsx::write(&book, &output_path)?;

    let log_path =
        rows.log
            .write_beside(&output_path, "convert", "Converted from session report")?;

    Ok(ConversionReport {
        output_path,
        anomalies: rows.anomalies,
        log_path,
    })
}

// ---------------------------------------------------------------------------
// Excel output
// ---------------------------------------------------------------------------

/// 1-based column index to its Excel letters (1 -> A, 27 -> AA).
fn column_letters(mut index: usize) -> String {
    let mut out = Vec::new();
    while index > 0 {
        let rem = (index - 1) % 26;
        out.push(b'A' + rem as u8);
        index = (index - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII")
}

fn column_index(source: Source) -> usize {
    COLUMNS
        .iter()
        .position(|(_, s)| *s == source)
        .expect("column present in COLUMNS")
        + 1
}

fn write_sheet(
    book: &mut Workbook,
    output_path: &Path,
    data: &SessionRows,
) -> Result<(), Box<dyn Error>> {
    let sheet = book.sheet_mut(0)?;
    sheet.set_name(sheet_name(output_path));

    for (i, (header, _)) in COLUMNS.iter().enumerate() {
        let col = i as u32 + 1;
        sheet.cell_mut((col, 1)).set_value_string(*header);
        sheet.style_mut((col, 1)).font_mut().set_bold(true);
    }

    let adj_start_utc_col = column_letters(column_index(Source::AdjConnStartUtc));
    let adj_end_utc_col = column_letters(column_index(Source::AdjConnEndUtc));
    let energy_col = column_letters(column_index(Source::Number("Energy_Use")));
    let active_col = column_letters(column_index(Source::Duration("Active_Charge_Time")));

    for (r, row) in data.rows.iter().enumerate() {
        let excel_row = r as u32 + 2;

        for (i, (_, source)) in COLUMNS.iter().enumerate() {
            let col = i as u32 + 1;
            match source {
                Source::Text(name) => {
                    let value = data.field(row, name);
                    if !value.is_empty() {
                        sheet.cell_mut((col, excel_row)).set_value_string(value);
                    }
                }
                Source::Number(name) => {
                    let value = data.field(row, name);
                    if !value.is_empty() {
                        match value.parse::<f64>() {
                            Ok(n) => {
                                sheet.cell_mut((col, excel_row)).set_value_number(n);
                                if let Some(code) = decimal_format(name) {
                                    set_format(sheet, col, excel_row, code);
                                }
                            }
                            // A non-numeric value in a numeric column is preserved rather than
                            // dropped; the workbook still shows what the report said.
                            Err(_) => {
                                sheet.cell_mut((col, excel_row)).set_value_string(value);
                            }
                        }
                    }
                }
                Source::Duration(name) => {
                    if let Some(d) = data.duration(row, name)? {
                        sheet
                            .cell_mut((col, excel_row))
                            .set_value_number(serial_of_duration(d));
                        set_duration_style(sheet, col, excel_row);
                    }
                }
                Source::SessionId => {
                    sheet
                        .cell_mut((col, excel_row))
                        .set_value_string(row.session.id.as_str());
                }
                Source::ConnStartLocal => {
                    write_datetime(sheet, col, excel_row, serial_of_civil(row.start_local));
                }
                Source::ConnEndLocal => {
                    write_datetime(sheet, col, excel_row, serial_of_civil(row.end_local));
                }
                Source::AdjConnStartLocal => {
                    write_datetime(
                        sheet,
                        col,
                        excel_row,
                        serial_of_civil(row.adj_start_local()),
                    );
                }
                Source::AdjConnEndLocal => {
                    write_datetime(sheet, col, excel_row, serial_of_civil(row.adj_end_local()));
                }
                Source::ConnStartUtc => {
                    write_datetime(
                        sheet,
                        col,
                        excel_row,
                        serial_of_instant(row.session.conn_start),
                    );
                }
                Source::ConnEndUtc => {
                    write_datetime(
                        sheet,
                        col,
                        excel_row,
                        serial_of_instant(row.session.conn_end),
                    );
                }
                Source::AdjConnStartUtc => {
                    write_datetime(
                        sheet,
                        col,
                        excel_row,
                        serial_of_instant(row.session.adj_conn_start()),
                    );
                }
                Source::AdjConnEndUtc => {
                    write_datetime(
                        sheet,
                        col,
                        excel_row,
                        serial_of_instant(row.session.adj_conn_end()),
                    );
                }
                Source::AdjConnDuration => {
                    // Subtracting the UTC columns, not the local ones: local arithmetic is wrong by
                    // an hour for a session spanning the DST fold. Both ends are the adjusted ones,
                    // so the cell equals `Session::adj_duration` — the span the estimating logic
                    // places the session on, which is the point of showing it.
                    sheet.cell_mut((col, excel_row)).set_formula(format!(
                        "{adj_end_utc_col}{excel_row}-{adj_start_utc_col}{excel_row}"
                    ));
                    set_duration_style(sheet, col, excel_row);
                }
                Source::AvgKw => {
                    // Written unconditionally: with zero Active_Charge_Time
                    // this evaluates to #DIV/0!, which is the honest answer — energy delivered in
                    // no time at all has no finite average power.
                    sheet.cell_mut((col, excel_row)).set_formula(format!(
                        "{energy_col}{excel_row}/({active_col}{excel_row}*24)"
                    ));
                    set_format(sheet, col, excel_row, AVG_KW_FORMAT);
                }
                Source::Anomalies => {
                    if !row.session.anomalies.is_empty() {
                        let tokens: Vec<&str> = row
                            .session
                            .anomalies
                            .iter()
                            .map(AnomalyKind::as_str)
                            .collect();
                        sheet
                            .cell_mut((col, excel_row))
                            .set_value_string(tokens.join(","));
                    }
                }
            }
        }
    }

    add_comments(sheet);
    set_widths(sheet);
    let last_col = column_letters(COLUMNS.len());
    let last_row = data.rows.len() + 1;
    sheet.set_auto_filter(format!("A1:{last_col}{last_row}"));
    Ok(())
}

fn write_datetime(sheet: &mut Worksheet, col: u32, row: u32, serial: f64) {
    sheet.cell_mut((col, row)).set_value_number(serial);
    set_format(sheet, col, row, DATETIME_FORMAT);
    set_alignment(sheet, col, row, HorizontalAlignmentValues::Left);
}

fn set_duration_style(sheet: &mut Worksheet, col: u32, row: u32) {
    set_format(sheet, col, row, DURATION_FORMAT);
    set_alignment(sheet, col, row, HorizontalAlignmentValues::Center);
}

fn set_format(sheet: &mut Worksheet, col: u32, row: u32, code: &str) {
    sheet
        .style_mut((col, row))
        .number_format_mut()
        .set_format_code(code);
}

fn set_alignment(sheet: &mut Worksheet, col: u32, row: u32, horizontal: HorizontalAlignmentValues) {
    sheet
        .style_mut((col, row))
        .alignment_mut()
        .set_horizontal(horizontal);
}

/// Decimal precision for the `Source::Number` columns that need more than Excel's default display.
fn decimal_format(csv_column: &str) -> Option<&'static str> {
    match csv_column {
        "Energy_Use" => Some(ENERGY_USE_FORMAT),
        "Total_Fee" => Some(TOTAL_FEE_FORMAT),
        _ => None,
    }
}

/// Prefix carried by the session report exports. Stripped from the sheet name, which Excel caps
/// at 31 characters — long enough to lose the reporting period that follows it.
const SESSION_REPORT_PREFIX: &str = "Session_Report_";

/// The output file name, minus its `.xlsx` suffix and minus a leading [`SESSION_REPORT_PREFIX`],
/// so `Session_Report_June_1_2026-June_30_2026` names the sheet `June_1_2026-June_30_2026` rather
/// than being truncated to `Session_Report_June_1_2026-June`. Excel sheet names are capped at 31
/// characters and cannot contain `[]:*?/\`.
fn sheet_name(output_path: &Path) -> String {
    let stem = output_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Sessions".to_owned());
    // A name that is *only* the prefix keeps it: an empty sheet name is not a name.
    let stem = match stem.strip_prefix(SESSION_REPORT_PREFIX) {
        Some(rest) if !rest.is_empty() => rest.to_owned(),
        _ => stem,
    };
    let cleaned: String = stem
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .collect();
    cleaned.chars().take(31).collect()
}

fn add_comments(sheet: &mut Worksheet) {
    let notes = [
        (
            Source::AdjConnStartLocal,
            "Adjusted connection start: Conn_DateTime_Start rounded DOWN to the whole minute, and \
             INCLUSIVE. The report states times only to the minute, so the true start is known \
             only to fall somewhere in the minute named; this is the earliest instant it could \
             have been. Together with adj_conn_end it gives the half-open span \
             [adj_conn_start, adj_conn_end), the tightest window guaranteed to contain the whole \
             connection.",
        ),
        (
            Source::AdjConnEndLocal,
            "Adjusted connection end: EXCLUSIVE -- the first instant the connection is certainly \
             over. The true end is known only to fall somewhere in the minute Conn_DateTime_End \
             names, and it is not known whether that minute's last second counts as inside the \
             session or outside it. So one second is added, the result is rounded DOWN to the \
             whole minute, and one further minute is added. For a reported end on the whole \
             minute -- which is every row the session report currently produces -- that comes to \
             the following minute. Because the end is excluded, a session starting at this exact \
             time does NOT overlap this one.",
        ),
        (
            Source::AdjConnDuration,
            "adj_conn_end_utc - adj_conn_start_utc: the width of the window above, which is the \
             span every estimate places this session on. Computed from the UTC columns so it is \
             true elapsed time even for a session spanning the DST fold, where local arithmetic \
             would be wrong by an hour.",
        ),
        (
            Source::AvgKw,
            "Energy_Use / Active_Charge_Time, in kW. Active_Charge_Time is an Excel duration, i.e. \
             a fraction of a day, hence the *24 to convert it to hours. A zero Active_Charge_Time \
             yields #DIV/0!, which is the honest answer: there is no average power to state. Such \
             a row beside a non-zero Energy_Use is almost certainly a reporting fault -- the \
             session report's three duration fields track the same thing to within about a \
             second -- and is worth looking at individually.",
        ),
        (
            Source::Anomalies,
            "Comma-separated list of anomalies found for this row, named after the AnomalyKind \
             variants. Empty means the row needed no judgement call. This cell IS read back, and \
             InconsistentDuration is what removes a session from every estimate -- so editing it \
             changes the figures. The adjusted columns are not read back: they are recomputed, and \
             a disagreement is written to the .xlsx.read.log rather than obeyed.",
        ),
    ];
    for (source, text) in notes {
        let col = column_index(source) as u32;
        let mut comment = Comment::default();
        comment.new_comment((col, 1));
        comment.set_author("session_csv_to_xlsx");
        comment.set_text_string(text);
        sheet.add_comments(comment);
    }
}

/// The date/time format needs real width or Excel renders the cell as `####`.
fn set_widths(sheet: &mut Worksheet) {
    for (i, (header, source)) in COLUMNS.iter().enumerate() {
        let letters = column_letters(i + 1);
        let width = match source {
            Source::ConnStartLocal
            | Source::ConnStartUtc
            | Source::ConnEndLocal
            | Source::ConnEndUtc
            | Source::AdjConnEndLocal
            | Source::AdjConnEndUtc => 24.0,
            Source::Duration(_) | Source::AdjConnDuration => 13.0,
            // Room for a couple of variant names side by side.
            Source::Anomalies => 40.0,
            _ => (header.len() as f64 + 2.0).max(10.0),
        };
        sheet.column_dimension_mut(&letters).set_width(width);
    }
}

// ---------------------------------------------------------------------------
// Excel input
// ---------------------------------------------------------------------------

/// Sheet columns that make a workbook a session report. The reading-side counterpart of the
/// required CSV headers in [`super::csv`].
///
/// This is deliberately wider than the set [`session_list`] strictly consumes. A workbook missing
/// any of these is not a rendering of a session report, and guessing at its contents would produce
/// peak numbers that cannot be trusted. `anomalies` in particular is load-bearing: without it every
/// session would silently look clean, and inconsistent ones would fold back into the estimates.
const REQUIRED_SHEET_HEADERS: &[&str] = &[
    "Charge_Session_ID",
    "conn_start_utc",
    "conn_end_utc",
    "adj_conn_end_utc",
    "Conn_Duration",
    "Active_Charge_Time",
    "Energy_Use",
    "anomalies",
];

/// Sheet header name to its 1-based column number.
type SheetHeaders = HashMap<String, u32>;

/// Reads a workbook written by [`session_csv_to_xlsx`] and returns the charging sessions it
/// describes, ready for the peak power contribution logic.
///
/// Columns are located by the header names in row 1, not by position, so inserting or reordering
/// columns in the sheet does not silently shift what is read. Only the private
/// `REQUIRED_SHEET_HEADERS` are consulted; the first worksheet is used.
///
/// Sorting into the three buckets of [`SessionReport`] happens here rather than at conversion time,
/// because the workbook is meant to be a faithful rendering of the session report. The rules are
/// [`SessionReport::new`]'s, shared with [`csv::session_list`] so the two readers cannot disagree.
///
/// `avg_kw` is recomputed here rather than read from the sheet's `avg_kw` column, which
/// holds a formula whose cached value this crate never writes. For a spike that leaves it infinite
/// or `NaN`, which is the honest reading; the estimating logic substitutes a finite value.
///
/// A `<stem>.xlsx.read.log` is written beside the workbook, listing any stored column that
/// disagreed with the recomputed value. That channel has no counterpart in [`csv::session_list`],
/// which reads the source and so has nothing to compare against.
///
/// # Errors
///
/// Returns `Err` if the workbook cannot be read, a required column is missing, any cell in a row
/// that has a `Charge_Session_ID` does not hold the number it should, or the `anomalies` column
/// holds a token that is not an [`AnomalyKind`] variant name. A workbook that cannot be read in
/// full is one whose peak numbers cannot be trusted, so no row is skipped quietly.
/// Rows with no `Charge_Session_ID` at all are treated as trailing blanks and ignored.
pub fn session_list(path: &Path) -> Result<SessionReport, Box<dyn Error>> {
    let book = umya_spreadsheet::reader::xlsx::read(path)?;
    let sheet = book.sheet(0)?;
    let headers = sheet_headers(sheet, path)?;

    let mut log = RunLog::new();
    // Counted rather than logged per row. Every formula column of a freshly written workbook is
    // unevaluated, so a line each would bury the real discrepancies under one per row per column.
    let mut unevaluated: BTreeMap<&'static str, usize> = BTreeMap::new();
    // One allocation for the workbook, shared by every session read from it.
    let source = Rc::new(path.to_path_buf());
    let mut sessions: Vec<RSession> = Vec::new();
    for row in 2..=sheet.highest_row() {
        let id = sheet
            .value((headers["Charge_Session_ID"], row))
            .trim()
            .to_owned();
        if id.is_empty() {
            continue;
        }

        let energy_use = number(sheet, &headers, "Energy_Use", row)?;
        let charge_time = duration_of_serial(number(sheet, &headers, "Active_Charge_Time", row)?);
        let anomalies = anomaly_kinds(sheet, &headers, row, path)?;
        let session = Session {
            path: source.clone(),
            row: row as usize,
            id,
            conn_start: instant_of_serial(number(sheet, &headers, "conn_start_utc", row)?)?,
            conn_end: instant_of_serial(number(sheet, &headers, "conn_end_utc", row)?)?,
            conn_duration: duration_of_serial(number(sheet, &headers, "Conn_Duration", row)?),
            charge_time,
            energy_use,
            anomalies,
        };

        check_stored_columns(sheet, &headers, row, &session, &mut log, &mut unevaluated);

        sessions.push(Rc::new(session));
    }

    for (name, count) in &unevaluated {
        log.note(format!(
            "{name}: {count} row(s) hold a formula with no stored value, so there was nothing to \
             check. Normal for a workbook this software wrote — it writes formulas and Excel \
             computes them, so values appear only once Excel has opened and saved the file. The \
             recomputed figures are used throughout either way."
        ));
    }
    let log_path = log.write_beside(path, "xlsx.read", "Read back from workbook")?;

    // Through the merge for the same reason `csv::session_list` is: it is where a shared
    // `Charge_Session_ID` is noticed, and one file can carry one as readily as two can.
    Ok(SessionReport::new(
        MergedSessions::merge_sessions(vec![sessions]),
        vec![log_path],
    ))
}

/// Compares the workbook's stored derived columns against what the [`Session`] methods recompute,
/// noting any disagreement in the run log.
///
/// **The recomputed value always wins.** Nothing here changes a `Session`, and nothing here raises
/// an [`AnomalyKind`]. A disagreement means the sheet is stale or was edited by hand, which is a
/// fact about the file and not about the session — and letting it feed the estimates would make an
/// edited cell silently change which sessions count. See `sessions::log`.
///
/// `adj_conn_duration` and `avg_kw` hold formulas. This crate writes the formula and no cached
/// value, so Excel has to have opened and saved the workbook for one to be there. A missing cached
/// value is therefore the normal state of a freshly written workbook, not a fault — it is counted
/// into `unevaluated` and summarised once per column, rather than logged per row. Per row it would
/// be two lines for every session, which is a log nobody reads and real discrepancies buried in
/// it.
fn check_stored_columns(
    sheet: &Worksheet,
    headers: &SheetHeaders,
    row: u32,
    session: &Session,
    log: &mut RunLog,
    unevaluated: &mut BTreeMap<&'static str, usize>,
) {
    let id = &session.id;

    let mut check_instant = |name: &str, expected: Timestamp| {
        let Some(&col) = headers.get(name) else {
            return; // not a required header; absence is not a discrepancy
        };
        match sheet.value((col, row)).trim().parse::<f64>() {
            Ok(serial) => match instant_of_serial(serial) {
                Ok(stored) if stored != expected => log.note(format!(
                    "row {row} ({id}): stored {name} is {stored}, recomputed {expected}; \
                     using the recomputed value"
                )),
                Ok(_) => {}
                Err(e) => log.note(format!(
                    "row {row} ({id}): {name} is not a valid instant: {e}"
                )),
            },
            Err(_) => log.note(format!(
                "row {row} ({id}): {name} does not hold a number; using the recomputed value"
            )),
        }
    };
    check_instant("adj_conn_start_utc", session.adj_conn_start());
    check_instant("adj_conn_end_utc", session.adj_conn_end());

    // `adj_duration` subtracts one adjusted bound from the other and panics if they are inverted.
    // This runs before the exclusion sort, so an `InconsistentDuration` row reaches here — and the
    // whole point of check 1 is that such a row exists. Skip the column rather than compute it: a
    // stored duration for a session that has no duration is not a discrepancy worth a line.
    let adj_duration = (session.adj_conn_start() <= session.adj_conn_end())
        .then(|| serial_of_duration(session.adj_duration()));
    let expected_values: Vec<(&str, f64)> = adj_duration
        .map(|d| ("adj_conn_duration", d))
        .into_iter()
        .chain([("avg_kw", session.avg_kw())])
        .collect();

    for (name, expected) in expected_values {
        let Some(&col) = headers.get(name) else {
            continue;
        };
        let raw = sheet.value((col, row));
        let raw = raw.trim();
        if raw.is_empty() {
            *unevaluated.entry(name).or_default() += 1;
            continue;
        }
        match raw.parse::<f64>() {
            // Serials and kilowatts both come back through floating point, so compare to the
            // resolution the sheet actually shows rather than for equality.
            Ok(stored) if (stored - expected).abs() > 1e-6 => log.note(format!(
                "row {row} ({id}): stored {name} is {stored}, recomputed {expected}; \
                 using the recomputed value"
            )),
            Ok(_) => {}
            Err(_) => log.note(format!(
                "row {row} ({id}): {name} does not hold a number; using the recomputed value"
            )),
        }
    }
}

/// Parses the `anomalies` cell. An unrecognised token is an error rather than a shrug: it means the
/// workbook was written by something this crate does not know, and the sessions it excludes cannot
/// be determined.
fn anomaly_kinds(
    sheet: &Worksheet,
    headers: &SheetHeaders,
    row: u32,
    path: &Path,
) -> Result<Vec<AnomalyKind>, Box<dyn Error>> {
    sheet
        .value((headers["anomalies"], row))
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            AnomalyKind::from_token(token).ok_or_else(|| -> Box<dyn Error> {
                format!(
                    "{}: row {row}, column `anomalies`: unknown anomaly {token:?}",
                    path.display()
                )
                .into()
            })
        })
        .collect()
}

fn sheet_headers(sheet: &Worksheet, path: &Path) -> Result<SheetHeaders, Box<dyn Error>> {
    let mut headers = SheetHeaders::new();
    for col in 1..=sheet.highest_column() {
        let name = sheet.value((col, 1)).trim().to_owned();
        if !name.is_empty() {
            // First wins, so a duplicated header cannot displace the column it shadows.
            headers.entry(name).or_insert(col);
        }
    }

    for required in REQUIRED_SHEET_HEADERS {
        if !headers.contains_key(*required) {
            return Err(format!("{}: missing required column `{required}`", path.display()).into());
        }
    }
    Ok(headers)
}

/// Reads a numeric cell. `name` must be one of [`REQUIRED_SHEET_HEADERS`], which
/// [`sheet_headers`] has already proven present.
fn number(
    sheet: &Worksheet,
    headers: &SheetHeaders,
    name: &str,
    row: u32,
) -> Result<f64, Box<dyn Error>> {
    let col = headers[name];
    sheet.value_number((col, row)).ok_or_else(|| {
        let found = sheet.value((col, row));
        format!("row {row}, column `{name}`: expected a number, found {found:?}").into()
    })
}

#[cfg(test)]
// cargo test --lib -- sessions::excel::test --nocapture
mod test {
    use super::*;
    use crate::sessions::BREAKER_RATING_KW;
    use crate::sessions::test_support::{timing_anomalies, timing_anomalies_in_cell};
    use jiff::{civil, tz::TimeZone};
    use std::fs;
    use std::time::Duration;

    fn utc(dt: civil::DateTime) -> Timestamp {
        dt.to_zoned(TimeZone::UTC).unwrap().timestamp()
    }

    /// A scratch directory of its own per test, since these run in parallel within one process.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ev_peak_excel_{}_{tag}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sheet_name_strips_the_report_prefix() {
        let name = |s: &str| sheet_name(Path::new(s));
        assert_eq!(
            name("Session_Report_June_1_2026-June_30_2026.xlsx"),
            "June_1_2026-June_30_2026"
        );
        // No prefix: the stem is used as it stands.
        assert_eq!(name("July_data.xlsx"), "July_data");
        // Stripping would leave nothing, so the prefix stays.
        assert_eq!(name("Session_Report_.xlsx"), "Session_Report_");
        // Excel's 31-character cap still applies, and it applies after stripping.
        assert_eq!(
            name("Session_Report_a_very_long_reporting_period_name.xlsx"),
            "a_very_long_reporting_period_na"
        );
        assert_eq!(name("bad[name]:here.xlsx"), "bad_name__here");
    }

    #[test]
    fn column_letters_span_past_z() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(COLUMNS.len()), "AD");
    }

    const FIXTURE: &str = "\
UR_ID,Location_Address,Location_City,Location_Postal_Code,Station_ID,Station_Network_Provider,Station_Make,Station_Model,Charge_Session_ID,User_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Charge_Duration,Active_Charge_Time,Charging_Level,Energy_Use,Total_Fee,Vehicle_Make,Vehicle_Model,Vehicle_Year
CKT-7,,Toronto,,Station-7,Evolute Inc.,FLO,G5,S69865,,2026-06-01 16:22,2026-06-01 21:29,5:07:53,5:07:53,5:07:52,Level 2,30.6,5.63,VinFast,Vf8,2024
CKT-7,,Toronto,,Station-7,Evolute Inc.,FLO,G5,S13577,,2026-06-02 08:00,2026-06-02 08:00,0:00:11,0:00:11,0:00:10,Level 2,0,0,VinFast,Vf8,2024
";

    #[test]
    fn round_trip_produces_the_expected_workbook() {
        let dir = temp_dir("round_trip");
        let csv_path = dir.join("Session_Report_Test.csv");
        fs::write(&csv_path, FIXTURE).unwrap();

        let report = session_csv_to_xlsx(&csv_path).unwrap();
        assert_eq!(report.output_path, dir.join("Session_Report_Test.xlsx"));
        assert!(
            timing_anomalies(&report.anomalies.iter().map(|a| a.kind).collect::<Vec<_>>())
                .is_empty(),
            "{:?}",
            report.anomalies
        );

        let book = umya_spreadsheet::reader::xlsx::read(&report.output_path).unwrap();
        let sheet = book.sheet(0).unwrap();

        // Header row, in the agreed order.
        let expected: Vec<&str> = COLUMNS.iter().map(|(h, _)| *h).collect();
        for (i, header) in expected.iter().enumerate() {
            assert_eq!(&sheet.value((i as u32 + 1, 1)), header);
        }

        let col = |s: Source| column_index(s) as u32;

        // adj_conn_end = 21:30:00 local on the first row — the exclusive end of the minute the
        // reported 21:29 end falls in.
        let adj: f64 = sheet
            .value((col(Source::AdjConnEndLocal), 2))
            .parse()
            .unwrap();
        assert!((adj - 46_174.895_833_333_3).abs() < 1e-9, "{adj}");

        // Formulas, not cached values. Both operands are the *adjusted* UTC columns, so the cell
        // equals `Session::adj_duration` rather than a span starting at the reported start.
        let expect_formula = format!(
            "{}2-{}2",
            column_letters(column_index(Source::AdjConnEndUtc)),
            column_letters(column_index(Source::AdjConnStartUtc))
        );
        assert_eq!(
            sheet
                .cell((col(Source::AdjConnDuration), 2))
                .unwrap()
                .formula(),
            expect_formula
        );
        let avg_kw_formula = |r: u32| {
            format!(
                "{}{r}/({}{r}*24)",
                column_letters(column_index(Source::Number("Energy_Use"))),
                column_letters(column_index(Source::Duration("Active_Charge_Time")))
            )
        };
        assert_eq!(
            sheet.cell((col(Source::AvgKw), 2)).unwrap().formula(),
            avg_kw_formula(2)
        );

        // Sheet name is the output file's name, minus the .xlsx suffix and the report prefix.
        assert_eq!(sheet.name(), "Test");

        // Number formats.
        assert_eq!(
            sheet
                .style((col(Source::ConnStartLocal), 2))
                .number_format()
                .unwrap()
                .format_code(),
            DATETIME_FORMAT
        );
        assert_eq!(
            sheet
                .style((col(Source::Number("Energy_Use")), 2))
                .number_format()
                .unwrap()
                .format_code(),
            ENERGY_USE_FORMAT
        );
        assert_eq!(
            sheet
                .style((col(Source::AvgKw), 2))
                .number_format()
                .unwrap()
                .format_code(),
            AVG_KW_FORMAT
        );
        assert_eq!(
            sheet
                .style((col(Source::Number("Total_Fee")), 2))
                .number_format()
                .unwrap()
                .format_code(),
            TOTAL_FEE_FORMAT
        );

        // Date/time values are left-justified, duration values are centered.
        assert_eq!(
            *sheet
                .style((col(Source::ConnStartLocal), 2))
                .alignment()
                .unwrap()
                .horizontal(),
            HorizontalAlignmentValues::Left
        );
        assert_eq!(
            *sheet
                .style((col(Source::Duration("Conn_Duration")), 2))
                .alignment()
                .unwrap()
                .horizontal(),
            HorizontalAlignmentValues::Center
        );
        assert_eq!(
            *sheet
                .style((col(Source::AdjConnDuration), 2))
                .alignment()
                .unwrap()
                .horizontal(),
            HorizontalAlignmentValues::Center
        );
        assert_eq!(
            sheet
                .style((col(Source::Duration("Conn_Duration")), 2))
                .number_format()
                .unwrap()
                .format_code(),
            DURATION_FORMAT
        );

        // Explicit typing: Vehicle_Year is numeric, Station_ID stays text.
        assert_eq!(
            sheet.value((col(Source::Number("Vehicle_Year")), 2)),
            "2024"
        );
        assert_eq!(
            sheet.value((col(Source::Text("Station_ID")), 2)),
            "Station-7"
        );

        // The zero-energy session is present, not filtered out here.
        assert_eq!(sheet.value((col(Source::SessionId), 3)), "S13577");

        // avg_kw is written on every row, the zero-energy one included, so a row that would
        // divide by zero shows #DIV/0! rather than nothing at all.
        assert_eq!(
            sheet.cell((col(Source::AvgKw), 3)).unwrap().formula(),
            avg_kw_formula(3)
        );

        // Neither fixture row has anything wrong with its times, so the Anomalies column carries
        // no timing kind.
        for row in [2, 3] {
            assert!(
                timing_anomalies_in_cell(&sheet.value((col(Source::Anomalies), row))).is_empty()
            );
        }

        fs::remove_dir_all(&dir).ok();
    }

    /// A record whose start falls in the fold and whose end cannot discriminate the two offsets
    /// occupies two workbook rows. Each gets its own row number, its own id and its own cell, so
    /// the pair produces two anomaly items rather than one shared between them.
    #[test]
    fn duplicated_record_yields_two_rows_and_two_anomalies() {
        const CSV: &str = "\
Charge_Session_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Active_Charge_Time,Energy_Use
S1,2026-11-01 01:10,2026-11-01 01:40,0:30:00,0:29:00,2.9
S2,2026-11-02 08:00,2026-11-02 09:00,1:00:00,0:59:00,5.9
";
        let xlsx = convert("duplicated", CSV);
        let book = umya_spreadsheet::reader::xlsx::read(&xlsx).unwrap();
        let sheet = book.sheet(0).unwrap();
        let col = |s: Source| column_index(s) as u32;

        assert_eq!(sheet.value((col(Source::SessionId), 2)), "S1-EDT");
        assert_eq!(sheet.value((col(Source::SessionId), 3)), "S1-EST");
        for row in [2, 3] {
            assert_eq!(
                timing_anomalies_in_cell(&sheet.value((col(Source::Anomalies), row))),
                vec![AnomalyKind::DstAmbiguousDuplicated]
            );
        }

        // The row after a duplication is one further down the sheet than its CSV position.
        assert_eq!(sheet.value((col(Source::SessionId), 4)), "S2");

        // And the reader recovers all of it.
        let report = session_list(&xlsx).unwrap();
        let ids: Vec<_> = report.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["S1-EDT", "S1-EST", "S2"]);
        assert_eq!(report.sessions[0].row, 2);
        assert_eq!(report.sessions[1].row, 3);
        assert_eq!(report.sessions[2].row, 4);
        assert_eq!(
            timing_anomalies(&report.sessions[0].anomalies),
            vec![AnomalyKind::DstAmbiguousDuplicated]
        );
        assert!(timing_anomalies(&report.sessions[2].anomalies).is_empty());

        fs::remove_dir_all(xlsx.parent().unwrap()).ok();
    }

    /// The two readers describe the same sessions, and number them by the file each one read.
    ///
    /// They share a parse, so agreeing on the sessions and their buckets is guaranteed by
    /// construction. The row numbers are the one thing they are *not* meant to agree on:
    /// [`Session::row`] is a row of [`Session::path`], and the two readers have different files in
    /// front of them. A resolved DST fold is where that shows — it is one CSV record and two
    /// workbook rows — so the fixture puts a duplication first and every later row is displaced by
    /// it, with one session of each bucket after that.
    #[test]
    fn the_two_readers_agree() {
        const CSV: &str = "\
Charge_Session_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Active_Charge_Time,Energy_Use
S1,2026-11-01 01:10,2026-11-01 01:40,0:30:00,0:29:00,2.9
S2,2026-06-01 16:22,2026-06-01 21:29,5:07:53,5:07:52,30.6
S3,2026-06-02 10:00,2026-06-02 09:00,0:10:00,0:09:00,1.5
S4,2026-06-03 09:00,2026-06-03 09:00,0:00:00,0:00:00,4.2
";
        let dir = temp_dir("both_readers");
        let csv_path = dir.join("Session_Report_Test.csv");
        fs::write(&csv_path, CSV).unwrap();
        let xlsx = session_csv_to_xlsx(&csv_path).unwrap().output_path;

        let from_csv = csv::session_list(&csv_path).unwrap();
        let from_xlsx = session_list(&xlsx).unwrap();

        let ids = |r: &SessionReport| {
            let bucket = |b: &[RSession]| b.iter().map(|s| s.id.clone()).collect::<Vec<String>>();
            (bucket(&r.sessions), bucket(&r.spikes), bucket(&r.excluded))
        };
        assert_eq!(ids(&from_csv), ids(&from_xlsx));

        let rows = |r: &SessionReport| {
            let bucket = |b: &[RSession]| b.iter().map(|s| s.row).collect::<Vec<usize>>();
            (bucket(&r.sessions), bucket(&r.spikes), bucket(&r.excluded))
        };

        // Read from the CSV, the rows are the CSV's. Both halves of the `S1` fold came from record
        // 2 and both say so; `S2` follows on 3, undisplaced.
        assert_eq!(rows(&from_csv), (vec![2, 2, 3], vec![5], vec![4]));

        // Read from the workbook, the rows are the sheet's. The fold occupies two of them, so `S2`
        // sits on row 4 rather than 3, and everything after it is pushed down to match.
        assert_eq!(rows(&from_xlsx), (vec![2, 3, 4], vec![6], vec![5]));

        // Both readers name the file they read, which is what makes a row number resolvable.
        assert!(from_csv.sessions.iter().all(|s| *s.path == csv_path));
        assert!(from_xlsx.sessions.iter().all(|s| *s.path == xlsx));

        fs::remove_dir_all(&dir).ok();
    }

    /// The conversion report's anomalies carry rows of the file they were read from, which for a
    /// conversion is the CSV. The two halves of a resolved DST fold therefore share a row number:
    /// they came from one record, and the `-EDT`/`-EST` suffix on the id is what tells them apart.
    #[test]
    fn conversion_report_anomalies_carry_source_rows() {
        const CSV: &str = "\
Charge_Session_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Active_Charge_Time,Energy_Use
S1,2026-11-01 01:10,2026-11-01 01:40,0:30:00,0:29:00,2.9
S2,2026-11-02 08:00,2026-11-02 08:00,0:00:00,0:00:00,4.2
";
        let dir = temp_dir("excel_rows");
        let csv_path = dir.join("Session_Report_Test.csv");
        fs::write(&csv_path, CSV).unwrap();
        let report = session_csv_to_xlsx(&csv_path).unwrap();

        let items: Vec<_> = report
            .anomalies
            .iter()
            .filter(|a| a.kind != AnomalyKind::ExcessiveAvgKw)
            .map(|a| (a.session.row, a.session.id.as_str(), a.kind))
            .collect();
        assert_eq!(
            items,
            [
                (2, "S1-EDT", AnomalyKind::DstAmbiguousDuplicated),
                (2, "S1-EST", AnomalyKind::DstAmbiguousDuplicated),
                // CSV row 3. It sits on workbook row 4, the duplication above having pushed it
                // down, but the workbook row is the workbook's business and not this report's.
                (3, "S2", AnomalyKind::ZeroActiveChargeTime),
            ]
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// A session whose start, end and duration contradict each other cannot be placed on a
    /// timeline. It is written to the sheet, flagged, and kept out of the estimates.
    #[test]
    fn inconsistent_session_is_excluded_on_read() {
        const CSV: &str = "\
Charge_Session_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Active_Charge_Time,Energy_Use
S1,2026-06-01 16:22,2026-06-01 21:29,5:07:53,5:07:52,30.6
S2,2026-06-02 10:00,2026-06-02 09:00,0:10:00,0:09:00,1.5
";
        let xlsx = convert("inconsistent", CSV);
        let report = session_list(&xlsx).unwrap();

        assert_eq!(report.sessions.len(), 1);
        assert_eq!(report.sessions[0].id, "S1");
        assert!(report.spikes.is_empty());
        assert_eq!(report.excluded.len(), 1);
        assert_eq!(report.excluded[0].id, "S2");
        assert!(
            report.excluded[0]
                .anomalies
                .contains(&AnomalyKind::InconsistentDuration)
        );
        // The inverted session would put a Right end-point before its own Left.
        assert!(report.excluded[0].adj_conn_end() < report.excluded[0].adj_conn_start());

        fs::remove_dir_all(xlsx.parent().unwrap()).ok();
    }

    /// An `anomalies` cell this crate did not write is an error, not something to shrug at: it
    /// decides which sessions take part in the estimates.
    #[test]
    fn unknown_anomaly_token_is_rejected() {
        let xlsx = convert("unknown_anomaly", FIXTURE);
        let mut book = umya_spreadsheet::reader::xlsx::read(&xlsx).unwrap();
        let col = column_index(Source::Anomalies) as u32;
        book.sheet_mut(0)
            .unwrap()
            .cell_mut((col, 2))
            .set_value_string("SomethingElse");
        umya_spreadsheet::writer::xlsx::write(&book, &xlsx).unwrap();

        let err = session_list(&xlsx).unwrap_err().to_string();
        assert!(err.contains("SomethingElse"), "{err}");
        fs::remove_dir_all(xlsx.parent().unwrap()).ok();
    }

    /// Two rows whose energy arrived in no time at all, alongside an ordinary one.
    ///
    /// Two, because a spike's substituted average power turns on whether any energy was delivered:
    /// `S00001` carries 4.2 kWh and `S00002` carries none, so the fixture reaches both branches.
    const SPIKE_FIXTURE: &str = "\
UR_ID,Location_Address,Location_City,Location_Postal_Code,Station_ID,Station_Network_Provider,Station_Make,Station_Model,Charge_Session_ID,User_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Charge_Duration,Active_Charge_Time,Charging_Level,Energy_Use,Total_Fee,Vehicle_Make,Vehicle_Model,Vehicle_Year
CKT-7,,Toronto,,Station-7,Evolute Inc.,FLO,G5,S69865,,2026-06-01 16:22,2026-06-01 21:29,5:07:53,5:07:53,5:07:52,Level 2,30.6,5.63,VinFast,Vf8,2024
CKT-7,,Toronto,,Station-7,Evolute Inc.,FLO,G5,S00001,,2026-06-03 09:00,2026-06-03 09:00,0:00:00,0:00:00,0:00:00,Level 2,4.2,0,VinFast,Vf8,2024
CKT-7,,Toronto,,Station-7,Evolute Inc.,FLO,G5,S00002,,2026-06-03 10:00,2026-06-03 10:00,0:00:00,0:00:00,0:00:00,Level 2,0,0,VinFast,Vf8,2024
";

    /// Converts `csv` in a scratch directory of its own and returns the workbook path.
    fn convert(tag: &str, csv: &str) -> PathBuf {
        let dir = temp_dir(tag);
        let csv_path = dir.join("Session_Report_Test.csv");
        fs::write(&csv_path, csv).unwrap();
        session_csv_to_xlsx(&csv_path).unwrap().output_path
    }

    /// A stale `adj_conn_end_utc` is logged and **ignored**, not obeyed.
    ///
    /// The property the whole discrepancy channel exists for: editing a cell in a workbook must
    /// not change which sessions feed an estimate. Here the cell is moved an hour forward, which
    /// under the old read-back would have moved the session an hour forward with it. The
    /// recomputed value wins and the disagreement goes to the log.
    #[test]
    fn a_stale_stored_column_is_logged_and_overruled() {
        let xlsx = convert("stale_column", FIXTURE);

        // Move adj_conn_end_utc on row 2 an hour later than it should be.
        let mut book = umya_spreadsheet::reader::xlsx::read(&xlsx).unwrap();
        let expected = {
            let sheet = book.sheet(0).unwrap();
            let headers = sheet_headers(sheet, &xlsx).unwrap();
            let col = headers["adj_conn_end_utc"];
            let stored: f64 = sheet.value((col, 2)).parse().unwrap();
            let moved = stored + 1.0 / 24.0;
            (col, instant_of_serial(stored).unwrap(), moved)
        };
        let (col, correct, moved) = expected;
        book.sheet_mut(0)
            .unwrap()
            .cell_mut((col, 2))
            .set_value_number(moved);
        umya_spreadsheet::writer::xlsx::write(&book, &xlsx).unwrap();

        let report = session_list(&xlsx).unwrap();
        let session = &report.sessions[0];

        // Recomputed, not read: the edited cell had no effect on the session at all.
        assert_eq!(
            session.adj_conn_end(),
            correct,
            "the stored value overruled the recomputed one"
        );

        let log = fs::read_to_string(&report.log_paths[0]).unwrap();
        assert!(
            log.contains("adj_conn_end_utc"),
            "the discrepancy was not logged:\n{log}"
        );
        assert!(
            log.contains("using the recomputed value"),
            "the log does not say what was done:\n{log}"
        );
        // A discrepancy is not an anomaly. Nothing about the session changed classification.
        assert!(
            !session
                .anomalies
                .contains(&AnomalyKind::InconsistentDuration),
            "a stale cell raised an anomaly: {:?}",
            session.anomalies
        );
        assert!(
            report.excluded.is_empty(),
            "a stale cell excluded a session"
        );
    }

    #[test]
    fn session_list_reads_back_what_was_written() {
        let xlsx = convert("session_list", FIXTURE);
        let report = session_list(&xlsx).unwrap();

        // Beside the workbook, under its own suffix, so it cannot collide with the convert log or
        // with the CSV reader's.
        assert_eq!(
            report.log_paths[0],
            xlsx.with_file_name("Session_Report_Test.xlsx.read.log")
        );

        // A zero-Energy_Use session with a real charge time is an ordinary session: its average
        // power is legitimately zero and it still occupies a breaker. See docs/sessions/README.md, "Other".
        assert!(report.spikes.is_empty());
        assert!(report.excluded.is_empty());
        assert_eq!(report.sessions.len(), 2);
        assert_eq!(report.sessions[1].id, "S13577");
        assert_eq!(report.sessions[1].avg_kw(), 0.0);

        let s = &report.sessions[0];
        assert_eq!(s.id, "S69865");
        assert_eq!(s.row, 2);
        assert!(timing_anomalies(&s.anomalies).is_empty());
        // The reported start, 16:22 EDT is 20:22 UTC, unadjusted.
        assert_eq!(s.conn_start, utc(civil::date(2026, 6, 1).at(20, 22, 0, 0)));
        // The adjusted start, 16:22 EDT is 20:22 UTC.
        assert_eq!(
            s.adj_conn_start(),
            utc(civil::date(2026, 6, 1).at(20, 22, 0, 0))
        );
        // The reported end, 21:29 EDT, unadjusted.
        assert_eq!(s.conn_end, utc(civil::date(2026, 6, 2).at(1, 29, 0, 0)));
        // The adjusted end: 21:30:00 EDT, exclusive.
        assert_eq!(
            s.adj_conn_end(),
            utc(civil::date(2026, 6, 2).at(1, 30, 0, 0))
        );
        assert_eq!(s.conn_duration, Duration::from_secs(5 * 3600 + 7 * 60 + 53));
        assert_eq!(s.charge_time, Duration::from_secs(5 * 3600 + 7 * 60 + 52));
        assert!((s.energy_use - 30.6).abs() < 1e-9);

        let expected = 30.6 / (s.charge_time.as_secs_f64() / 3600.0);
        assert!((s.avg_kw() - expected).abs() < 1e-9, "{}", s.avg_kw());
        // Matches the sheet's own formula, Energy_Use / (Active_Charge_Time * 24).
        assert!(
            (s.avg_kw() - 5.963_620_614_984_84).abs() < 1e-9,
            "{}",
            s.avg_kw()
        );

        fs::remove_dir_all(xlsx.parent().unwrap()).ok();
    }

    /// Sorting into the spikes bucket keys on the degenerate input, not on the figure derived from
    /// it.
    ///
    /// That separation is the point. `Session::avg_kw` never returns a non-finite figure — it
    /// substitutes one, because an infinity would swamp any segment the session entered — so the
    /// bucket cannot be recognised by its average power and is recognised by the zero charge time
    /// that produced it. What the substitution *is* is asserted separately, once per branch.
    #[test]
    fn zero_active_charge_time_becomes_a_spike() {
        let xlsx = convert("spike", SPIKE_FIXTURE);
        let report = session_list(&xlsx).unwrap();

        assert_eq!(report.sessions.len(), 1);
        let ordinary = &report.sessions[0];
        assert_eq!(ordinary.id, "S69865");
        assert!(
            (ordinary.avg_kw()
                - ordinary.energy_use / (ordinary.charge_time.as_secs_f64() / 3600.0))
                .abs()
                < 1e-9
        );

        assert_eq!(report.spikes.len(), 2);
        // Detection keys on the degenerate input.
        assert!(report.spikes.iter().all(|s| s.charge_time.is_zero()));

        // Energy delivered in no time at all: the breaker rating stands in for a figure that has
        // none.
        let with_energy = &report.spikes[0];
        assert_eq!(with_energy.id, "S00001");
        assert_eq!(with_energy.avg_kw(), BREAKER_RATING_KW);
        // The energy is still there to be accounted for; that is why it is returned at all.
        assert!((with_energy.energy_use - 4.2).abs() < 1e-9);

        // No energy in no time draws nothing, so there is nothing to stand in for.
        let without_energy = &report.spikes[1];
        assert_eq!(without_energy.id, "S00002");
        assert_eq!(without_energy.energy_use, 0.0);
        assert_eq!(without_energy.avg_kw(), 0.0);

        fs::remove_dir_all(xlsx.parent().unwrap()).ok();
    }

    #[test]
    fn session_list_rejects_a_workbook_it_cannot_read_in_full() {
        // A required column renamed out of existence.
        let xlsx = convert("missing_header", FIXTURE);
        let mut book = umya_spreadsheet::reader::xlsx::read(&xlsx).unwrap();
        let start_col = column_index(Source::ConnStartUtc) as u32;
        book.sheet_mut(0)
            .unwrap()
            .cell_mut((start_col, 1))
            .set_value_string("Renamed");
        umya_spreadsheet::writer::xlsx::write(&book, &xlsx).unwrap();

        let err = session_list(&xlsx).unwrap_err().to_string();
        assert!(err.contains("conn_start_utc"), "{err}");
        fs::remove_dir_all(xlsx.parent().unwrap()).ok();

        // Text where a number belongs.
        let xlsx = convert("bad_number", FIXTURE);
        let mut book = umya_spreadsheet::reader::xlsx::read(&xlsx).unwrap();
        let energy_col = column_index(Source::Number("Energy_Use")) as u32;
        book.sheet_mut(0)
            .unwrap()
            .cell_mut((energy_col, 2))
            .set_value_string("n/a");
        umya_spreadsheet::writer::xlsx::write(&book, &xlsx).unwrap();

        let err = session_list(&xlsx).unwrap_err().to_string();
        assert!(err.contains("Energy_Use") && err.contains("row 2"), "{err}");
        fs::remove_dir_all(xlsx.parent().unwrap()).ok();
    }
}
