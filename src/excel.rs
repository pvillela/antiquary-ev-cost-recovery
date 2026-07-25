use std::{
    collections::HashMap,
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use jiff::{
    SignedDuration, Timestamp, civil,
    tz::{AmbiguousOffset, TimeZone},
};
use umya_spreadsheet::{Comment, HorizontalAlignmentValues, Workbook, Worksheet};

/// Time zone the session report's timestamps are stated in. See README.md, "Time zone".
const TZ_NAME: &str = "America/Toronto";

/// Excel's day-zero for the 1900 date system, as a Unix timestamp.
/// 1899-12-30T00:00:00Z; verified by [`test::excel_epoch_constant_matches_jiff`].
const EXCEL_EPOCH_UNIX_SECS: i64 = -2_209_161_600;

/// Seconds added to a truncated end time. See README.md, "New fields".
const END_PADDING: SignedDuration = SignedDuration::from_secs(59);

const DATETIME_FORMAT: &str = "yyyy-mm-dd hh:mm:ss ddd";
/// Elapsed-time format: unlike `hh:mm:ss` it does not wrap a 25-hour duration to `01:00:00`.
const DURATION_FORMAT: &str = "[h]:mm:ss";
const ENERGY_USE_FORMAT: &str = "0.000";
const AVG_POWER_FORMAT: &str = "0.000";
const TOTAL_FEE_FORMAT: &str = "0.00";

/// Outcome of converting one CSV file.
#[derive(Debug)]
pub struct Report {
    /// Where the workbook was written.
    pub output_path: PathBuf,
    /// Rows that needed a judgement call. Empty for a clean conversion.
    pub anomalies: Vec<Anomaly>,
}

/// A single row that needed a judgement call. Does not abort the conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomaly {
    /// 1-based CSV data row, excluding the header.
    pub row: usize,
    pub session_id: String,
    pub kind: AnomalyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyKind {
    /// `Active_Charge_Time` is zero on a session that reported non-zero `Energy_Use`,
    /// so `Avg_power` is not computable and its cell is left empty.
    ZeroActiveChargeTime,
    /// The start fell in the DST fold and both offsets reproduce the reported end,
    /// so the record was duplicated. See README.md, "Time zone".
    DstAmbiguousDuplicated,
    /// The start fell in the DST gap, i.e. a wall time that never occurred.
    /// Resolved forward to the instant just after the gap.
    DstGapShifted,
    /// The start fell in the DST fold and *neither* offset reproduces the reported end.
    /// The earlier offset was assumed.
    DstUnresolvable,
}

impl fmt::Display for AnomalyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ZeroActiveChargeTime => "zero Active_Charge_Time with non-zero Energy_Use",
            Self::DstAmbiguousDuplicated => "ambiguous DST fold; record duplicated as EDT and EST",
            Self::DstGapShifted => "local time falls in the DST gap; resolved forward",
            Self::DstUnresolvable => "DST fold matches neither offset; assumed the earlier one",
        };
        f.write_str(s)
    }
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "row {} ({}): {}", self.row, self.session_id, self.kind)
    }
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
    AdjConnEndLocal,
    AdjConnEndUtc,
    /// Formula: `Adj_conn_end_UTC - Conn_start_UTC`.
    AdjConnDuration,
    /// Formula: `Energy_Use / Active_Charge_Time`, in kW.
    AvgPower,
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
    ("Conn_start_UTC", Source::ConnStartUtc),
    ("Conn_DateTime_End", Source::ConnEndLocal),
    ("Conn_end_UTC", Source::ConnEndUtc),
    ("Adj_conn_end", Source::AdjConnEndLocal),
    ("Adj_conn_end_UTC", Source::AdjConnEndUtc),
    ("Adj_conn_duration", Source::AdjConnDuration),
    ("Conn_Duration", Source::Duration("Conn_Duration")),
    ("Charge_Duration", Source::Duration("Charge_Duration")),
    ("Active_Charge_Time", Source::Duration("Active_Charge_Time")),
    ("Charging_Level", Source::Text("Charging_Level")),
    ("Energy_Use", Source::Number("Energy_Use")),
    ("Avg_power", Source::AvgPower),
    ("Total_Fee", Source::Number("Total_Fee")),
    ("Vehicle_Make", Source::Text("Vehicle_Make")),
    ("Vehicle_Model", Source::Text("Vehicle_Model")),
    ("Vehicle_Year", Source::Number("Vehicle_Year")),
];

/// CSV columns that must be present for the conversion to mean anything.
const REQUIRED_HEADERS: &[&str] = &[
    "Charge_Session_ID",
    "Conn_DateTime_Start",
    "Conn_DateTime_End",
    "Conn_Duration",
    "Active_Charge_Time",
    "Energy_Use",
];

/// Reads the CSV file at `path`, which should have the same format as one on this project's `data`
/// directory, and transforms it into a `.xlsx` file saved to the same directory as the input file,
/// with the extension replaced.
///
/// The domain rules — the UTC conversion and its DST policy, the definitions of `Adj_conn_end` and
/// `Adj_conn_duration`, and the treatment of zero-`Energy_Use` sessions — are specified in
/// `README.md` under "Time zone", "New fields" and "Other". They are shared with the peak power
/// contribution logic and are not restated here.
///
/// What this function adds on top of those rules:
///
/// - Column order is given by [`COLUMNS`]: each UTC column sits beside the local value it derives
///   from, and `Adj_conn_end`, `Adj_conn_duration` and `Avg_power` are inserted as described in
///   README.md.
/// - Timestamp columns are Excel date/time numbers formatted `yyyy-mm-dd hh:mm:ss ddd`, left-
///   justified; duration columns are Excel durations formatted `[h]:mm:ss`, which does not wrap
///   past 24 hours, and are centered.
/// - `Adj_conn_duration` and `Avg_power` are live formulas. `Adj_conn_duration` subtracts the two
///   *UTC* columns, so it is true elapsed time even across a DST fold; `Avg_power` is
///   `=IF(Energy_Use=0, 0, Energy_Use/(Active_Charge_Time*24))`, in kW, displayed to 3 decimal
///   places, matching `Energy_Use`. `Total_Fee` is displayed to 2 decimal places.
/// - The remaining columns are copied over with an explicit per-column type, so values that merely
///   look numeric — postal codes, station ids — keep their text form.
/// - The sheet is named after the output file, minus its `.xlsx` suffix.
///
/// Zero-`Energy_Use` sessions are written to the workbook; they are excluded later, by the peak
/// power contribution logic.
///
/// # Errors
///
/// Returns `Err` only for conditions that invalidate the whole file: it cannot be read, a required
/// header from [`REQUIRED_HEADERS`] is missing, a timestamp or duration does not parse, or the
/// workbook cannot be written. Per-row judgement calls do not abort the conversion; they are
/// collected in [`Report::anomalies`].
pub fn session_csv_to_xlsx(path: &Path) -> Result<Report, Box<dyn Error>> {
    let tz = TimeZone::get(TZ_NAME)?;
    let (headers, records) = read_csv(path)?;

    let mut anomalies = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    for (i, record) in records.iter().enumerate() {
        let row_no = i + 1;
        let session = Session::parse(&headers, record, row_no)?;
        rows.extend(session.resolve(&tz, row_no, &mut anomalies)?);
    }

    let output_path = path.with_extension("xlsx");
    let mut book = umya_spreadsheet::new_file();
    write_sheet(&mut book, &output_path, &headers, &records, &rows)?;

    umya_spreadsheet::writer::xlsx::write(&book, &output_path)?;
    Ok(Report {
        output_path,
        anomalies,
    })
}

// ---------------------------------------------------------------------------
// CSV input
// ---------------------------------------------------------------------------

type Headers = HashMap<String, usize>;

fn read_csv(path: &Path) -> Result<(Headers, Vec<csv::StringRecord>), Box<dyn Error>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers: Headers = reader
        .headers()?
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim().to_owned(), i))
        .collect();

    for required in REQUIRED_HEADERS {
        if !headers.contains_key(*required) {
            return Err(format!("{}: missing required column `{required}`", path.display()).into());
        }
    }

    let records = reader.records().collect::<Result<Vec<_>, _>>()?;
    Ok((headers, records))
}

fn field<'a>(headers: &Headers, record: &'a csv::StringRecord, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|&i| record.get(i))
        .unwrap_or("")
        .trim()
}

/// Local time as `YYYY-MM-DD HH:MM`; the report carries no seconds, which is what makes
/// `Adj_conn_end` necessary in the first place.
fn parse_local(s: &str, row: usize, column: &str) -> Result<civil::DateTime, Box<dyn Error>> {
    civil::DateTime::strptime("%Y-%m-%d %H:%M", s).map_err(|e| {
        format!("row {row}, column `{column}`: cannot parse timestamp {s:?}: {e}").into()
    })
}

/// `H:MM:SS`, with hours unbounded so a session longer than a day still parses.
fn parse_duration(s: &str, row: usize, column: &str) -> Result<SignedDuration, Box<dyn Error>> {
    let bad = || -> Box<dyn Error> {
        format!("row {row}, column `{column}`: cannot parse duration {s:?}").into()
    };
    let mut parts = s.split(':');
    let (Some(h), Some(m), Some(sec), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let h: i64 = h.trim().parse().map_err(|_| bad())?;
    let m: i64 = m.trim().parse().map_err(|_| bad())?;
    let sec: i64 = sec.trim().parse().map_err(|_| bad())?;
    if !(0..60).contains(&m) || !(0..60).contains(&sec) {
        return Err(bad());
    }
    Ok(SignedDuration::from_secs(h * 3600 + m * 60 + sec))
}

/// The parsed fields of one CSV record that participate in the time calculations.
struct Session {
    id: String,
    start_local: civil::DateTime,
    end_local: civil::DateTime,
    conn_duration: SignedDuration,
    active_charge_time: SignedDuration,
    energy_use: f64,
}

/// One output row. A session normally yields one; an unresolvable DST fold yields two.
struct Row {
    /// Index into the original `records`, for the pass-through columns.
    record: usize,
    id: String,
    start_local: civil::DateTime,
    end_local: civil::DateTime,
    start_utc: Timestamp,
    end_utc: Timestamp,
    adj_end_utc: Timestamp,
    adj_end_local: civil::DateTime,
    /// `None` when `Active_Charge_Time` is zero on a session with non-zero energy.
    avg_power_computable: bool,
}

impl Session {
    fn parse(
        headers: &Headers,
        record: &csv::StringRecord,
        row: usize,
    ) -> Result<Self, Box<dyn Error>> {
        let energy_raw = field(headers, record, "Energy_Use");
        Ok(Self {
            id: field(headers, record, "Charge_Session_ID").to_owned(),
            start_local: parse_local(
                field(headers, record, "Conn_DateTime_Start"),
                row,
                "Conn_DateTime_Start",
            )?,
            end_local: parse_local(
                field(headers, record, "Conn_DateTime_End"),
                row,
                "Conn_DateTime_End",
            )?,
            conn_duration: parse_duration(
                field(headers, record, "Conn_Duration"),
                row,
                "Conn_Duration",
            )?,
            active_charge_time: parse_duration(
                field(headers, record, "Active_Charge_Time"),
                row,
                "Active_Charge_Time",
            )?,
            energy_use: energy_raw.parse().map_err(|_| -> Box<dyn Error> {
                format!("row {row}, column `Energy_Use`: cannot parse number {energy_raw:?}").into()
            })?,
        })
    }

    /// Resolves this session's local timestamps to UTC and derives `Adj_conn_end`.
    ///
    /// Returns one row normally, or two when the start falls in the DST fold and the reported end
    /// cannot tell the two offsets apart — see README.md, "Time zone", for why duplication is the
    /// policy and why the copies get distinct ids.
    fn resolve(
        &self,
        tz: &TimeZone,
        row: usize,
        anomalies: &mut Vec<Anomaly>,
    ) -> Result<Vec<Row>, Box<dyn Error>> {
        // Avg_power is a division by Active_Charge_Time; zero energy short-circuits to zero in the
        // sheet, so only a non-zero-energy session with no charge time is a problem.
        let avg_power_computable = self.energy_use == 0.0 || !self.active_charge_time.is_zero();
        if !avg_power_computable {
            anomalies.push(Anomaly {
                row,
                session_id: self.id.clone(),
                kind: AnomalyKind::ZeroActiveChargeTime,
            });
        }

        let ambiguous = tz.to_ambiguous_timestamp(self.start_local);
        let starts: Vec<(Timestamp, Option<&str>)> = match ambiguous.offset() {
            AmbiguousOffset::Unambiguous { .. } => {
                vec![(ambiguous.unambiguous()?, None)]
            }
            AmbiguousOffset::Gap { .. } => {
                // A wall time that never occurred. `compatible` moves to just after the gap; the
                // row is still written, but the shift is reported rather than silently applied.
                anomalies.push(Anomaly {
                    row,
                    session_id: self.id.clone(),
                    kind: AnomalyKind::DstGapShifted,
                });
                vec![(ambiguous.compatible()?, None)]
            }
            AmbiguousOffset::Fold { .. } => {
                let earlier = tz.to_ambiguous_timestamp(self.start_local).earlier()?;
                let later = tz.to_ambiguous_timestamp(self.start_local).later()?;
                let earlier_fits = self.reproduces_reported_end(tz, earlier);
                let later_fits = self.reproduces_reported_end(tz, later);
                match (earlier_fits, later_fits) {
                    (true, false) => vec![(earlier, None)],
                    (false, true) => vec![(later, None)],
                    (true, true) => {
                        // Both offsets are consistent with the report, which happens exactly when
                        // the session is short enough to fit inside the repeated hour. Keep both.
                        anomalies.push(Anomaly {
                            row,
                            session_id: self.id.clone(),
                            kind: AnomalyKind::DstAmbiguousDuplicated,
                        });
                        vec![(earlier, Some("EDT")), (later, Some("EST"))]
                    }
                    (false, false) => {
                        anomalies.push(Anomaly {
                            row,
                            session_id: self.id.clone(),
                            kind: AnomalyKind::DstUnresolvable,
                        });
                        vec![(earlier, None)]
                    }
                }
            }
        };

        starts
            .into_iter()
            .map(|(start_utc, suffix)| {
                let end_utc = self.resolve_end(tz, start_utc)?;
                // min() of the two upper bounds on the true end; see README.md, "New fields".
                let adj_end_utc =
                    (start_utc + END_PADDING + self.conn_duration).min(end_utc + END_PADDING);
                Ok(Row {
                    record: row - 1,
                    id: match suffix {
                        Some(s) => format!("{}-{s}", self.id),
                        None => self.id.clone(),
                    },
                    start_local: self.start_local,
                    end_local: self.end_local,
                    start_utc,
                    end_utc,
                    adj_end_utc,
                    adj_end_local: adj_end_utc.to_zoned(tz.clone()).datetime(),
                    avg_power_computable,
                })
            })
            .collect()
    }

    /// Does `start` plus the reported elapsed duration land back on the reported end?
    ///
    /// The comparison is truncated to the minute because the report's end timestamp carries no
    /// seconds; comparing exactly would reject the correct candidate.
    fn reproduces_reported_end(&self, tz: &TimeZone, start: Timestamp) -> bool {
        let end = (start + self.conn_duration).to_zoned(tz.clone()).datetime();
        end.date() == self.end_local.date()
            && end.hour() == self.end_local.hour()
            && end.minute() == self.end_local.minute()
    }

    /// Resolves the reported end to UTC. When the end itself falls in the fold, the candidate
    /// nearest to `start + Conn_Duration` is the one consistent with this session.
    fn resolve_end(
        &self,
        tz: &TimeZone,
        start_utc: Timestamp,
    ) -> Result<Timestamp, Box<dyn Error>> {
        let ambiguous = tz.to_ambiguous_timestamp(self.end_local);
        Ok(match ambiguous.offset() {
            AmbiguousOffset::Unambiguous { .. } => ambiguous.unambiguous()?,
            AmbiguousOffset::Gap { .. } => ambiguous.compatible()?,
            AmbiguousOffset::Fold { .. } => {
                let reference = start_utc + self.conn_duration;
                let earlier = tz.to_ambiguous_timestamp(self.end_local).earlier()?;
                let later = tz.to_ambiguous_timestamp(self.end_local).later()?;
                let d = |t: Timestamp| (t.as_second() - reference.as_second()).abs();
                if d(earlier) <= d(later) {
                    earlier
                } else {
                    later
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Excel output
// ---------------------------------------------------------------------------

/// Days since Excel's day-zero, as used by the 1900 date system.
fn excel_serial(dt: civil::DateTime) -> Result<f64, Box<dyn Error>> {
    let secs = dt.to_zoned(TimeZone::UTC)?.timestamp().as_second();
    Ok((secs - EXCEL_EPOCH_UNIX_SECS) as f64 / 86_400.0)
}

fn excel_serial_utc(ts: Timestamp) -> Result<f64, Box<dyn Error>> {
    excel_serial(ts.to_zoned(TimeZone::UTC).datetime())
}

/// Excel stores a duration as a fraction of a day.
fn excel_duration(d: SignedDuration) -> f64 {
    d.as_secs() as f64 / 86_400.0
}

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
    headers: &Headers,
    records: &[csv::StringRecord],
    rows: &[Row],
) -> Result<(), Box<dyn Error>> {
    let sheet = book.sheet_mut(0)?;
    sheet.set_name(sheet_name(output_path));

    for (i, (header, _)) in COLUMNS.iter().enumerate() {
        let col = i as u32 + 1;
        sheet.cell_mut((col, 1)).set_value_string(*header);
        sheet.style_mut((col, 1)).font_mut().set_bold(true);
    }

    let start_utc_col = column_letters(column_index(Source::ConnStartUtc));
    let adj_end_utc_col = column_letters(column_index(Source::AdjConnEndUtc));
    let energy_col = column_letters(column_index(Source::Number("Energy_Use")));
    let active_col = column_letters(column_index(Source::Duration("Active_Charge_Time")));

    for (r, row) in rows.iter().enumerate() {
        let excel_row = r as u32 + 2;
        let record = &records[row.record];

        for (i, (_, source)) in COLUMNS.iter().enumerate() {
            let col = i as u32 + 1;
            match source {
                Source::Text(name) => {
                    let value = field(headers, record, name);
                    if !value.is_empty() {
                        sheet.cell_mut((col, excel_row)).set_value_string(value);
                    }
                }
                Source::Number(name) => {
                    let value = field(headers, record, name);
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
                    let raw = field(headers, record, name);
                    if !raw.is_empty() {
                        let d = parse_duration(raw, row.record + 1, name)?;
                        sheet
                            .cell_mut((col, excel_row))
                            .set_value_number(excel_duration(d));
                        set_duration_style(sheet, col, excel_row);
                    }
                }
                Source::SessionId => {
                    sheet
                        .cell_mut((col, excel_row))
                        .set_value_string(row.id.as_str());
                }
                Source::ConnStartLocal => {
                    write_datetime(sheet, col, excel_row, excel_serial(row.start_local)?);
                }
                Source::ConnEndLocal => {
                    write_datetime(sheet, col, excel_row, excel_serial(row.end_local)?);
                }
                Source::AdjConnEndLocal => {
                    write_datetime(sheet, col, excel_row, excel_serial(row.adj_end_local)?);
                }
                Source::ConnStartUtc => {
                    write_datetime(sheet, col, excel_row, excel_serial_utc(row.start_utc)?);
                }
                Source::ConnEndUtc => {
                    write_datetime(sheet, col, excel_row, excel_serial_utc(row.end_utc)?);
                }
                Source::AdjConnEndUtc => {
                    write_datetime(sheet, col, excel_row, excel_serial_utc(row.adj_end_utc)?);
                }
                Source::AdjConnDuration => {
                    // Subtracting the UTC columns, not the local ones: local arithmetic is wrong by
                    // an hour for a session spanning the DST fold.
                    sheet.cell_mut((col, excel_row)).set_formula(format!(
                        "{adj_end_utc_col}{excel_row}-{start_utc_col}{excel_row}"
                    ));
                    set_duration_style(sheet, col, excel_row);
                }
                Source::AvgPower => {
                    if row.avg_power_computable {
                        sheet.cell_mut((col, excel_row)).set_formula(format!(
                            "IF({energy_col}{excel_row}=0,0,{energy_col}{excel_row}/({active_col}{excel_row}*24))"
                        ));
                        set_format(sheet, col, excel_row, AVG_POWER_FORMAT);
                    }
                }
            }
        }
    }

    add_comments(sheet);
    set_widths(sheet);
    let last_col = column_letters(COLUMNS.len());
    let last_row = rows.len() + 1;
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

/// The output file name, minus its `.xlsx` suffix. Excel sheet names are capped at 31 characters
/// and cannot contain `[]:*?/\`.
fn sheet_name(output_path: &Path) -> String {
    let stem = output_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Sessions".to_owned());
    let cleaned: String = stem
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .collect();
    cleaned.chars().take(31).collect()
}

fn add_comments(sheet: &mut Worksheet) {
    let notes = [
        (
            Source::AdjConnEndLocal,
            "Adjusted connection end. The report's timestamps carry no seconds, so the true end is \
             only known to within a minute. This is min(Conn_DateTime_Start + 59s + Conn_Duration, \
             Conn_DateTime_End + 59s): the tighter of the two upper bounds. See README.md, \
             \"New fields\".",
        ),
        (
            Source::AdjConnDuration,
            "Adj_conn_end_UTC - Conn_start_UTC. Computed from the UTC columns so it is true \
             elapsed time even for a session spanning the DST fold, where local arithmetic would \
             be wrong by an hour.",
        ),
        (
            Source::AvgPower,
            "Energy_Use / Active_Charge_Time, in kW. Active_Charge_Time is an Excel duration, i.e. \
             a fraction of a day, hence the *24 to convert it to hours. Zero energy yields zero \
             power.",
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
            _ => (header.len() as f64 + 2.0).max(10.0),
        };
        sheet.column_dimension_mut(&letters).set_width(width);
    }
}

#[cfg(test)]
// cargo test --package ev-peak-contrib --lib --all-features -- excel::test --nocapture
mod test {
    use super::*;
    use std::fs;

    fn tz() -> TimeZone {
        TimeZone::get(TZ_NAME).unwrap()
    }

    fn dt(s: &str) -> civil::DateTime {
        civil::DateTime::strptime("%Y-%m-%d %H:%M", s).unwrap()
    }

    fn session(start: &str, end: &str, conn: &str) -> Session {
        Session {
            id: "S1".to_owned(),
            start_local: dt(start),
            end_local: dt(end),
            conn_duration: parse_duration(conn, 1, "Conn_Duration").unwrap(),
            active_charge_time: parse_duration(conn, 1, "Active_Charge_Time").unwrap(),
            energy_use: 10.0,
        }
    }

    fn local_of(ts: Timestamp) -> civil::DateTime {
        ts.to_zoned(tz()).datetime()
    }

    #[test]
    fn excel_epoch_constant_matches_jiff() {
        let computed = civil::date(1899, 12, 30)
            .at(0, 0, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp()
            .as_second();
        assert_eq!(computed, EXCEL_EPOCH_UNIX_SECS);
    }

    #[test]
    fn excel_serial_matches_known_values() {
        // 1900-01-01 is serial 2 in the 1900 date system.
        assert_eq!(excel_serial(dt("1900-01-01 00:00")).unwrap(), 2.0);
        // Sample row 1: 2026-06-01 21:29:59 local.
        let serial = excel_serial(civil::date(2026, 6, 1).at(21, 29, 59, 0)).unwrap();
        assert!((serial - 46_174.895_821_759_3).abs() < 1e-9, "{serial}");
    }

    #[test]
    fn durations_parse_including_over_24_hours() {
        assert_eq!(
            parse_duration("5:07:53", 1, "d").unwrap(),
            SignedDuration::from_secs(5 * 3600 + 7 * 60 + 53)
        );
        assert_eq!(
            parse_duration("30:00:00", 1, "d").unwrap(),
            SignedDuration::from_secs(30 * 3600)
        );
        assert!(parse_duration("5:70:00", 1, "d").is_err());
        assert!(parse_duration("5:07", 1, "d").is_err());
    }

    #[test]
    fn column_letters_span_past_z() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(COLUMNS.len()), "AA");
    }

    /// The `min(...)` rule takes the tighter of the two upper bounds. Both branches are exercised
    /// by real sample rows.
    #[test]
    fn adj_conn_end_takes_the_tighter_bound() {
        let mut anomalies = Vec::new();

        // end < start + duration: the reported end binds. 16:22 + 5:07:53 = 21:29:53 > 21:29.
        let rows = session("2026-06-01 16:22", "2026-06-01 21:29", "5:07:53")
            .resolve(&tz(), 1, &mut anomalies)
            .unwrap();
        assert_eq!(
            local_of(rows[0].adj_end_utc),
            dt("2026-06-01 21:29").with().second(59).build().unwrap()
        );

        // start + duration <= end: the computed end binds. 16:42 + 6:58:29 = 23:40:29.
        let rows = session("2026-06-07 16:42", "2026-06-07 23:41", "6:58:29")
            .resolve(&tz(), 1, &mut anomalies)
            .unwrap();
        assert_eq!(
            local_of(rows[0].adj_end_utc),
            civil::date(2026, 6, 7).at(23, 41, 28, 0)
        );

        assert!(anomalies.is_empty());
    }

    /// Both invariants the rule exists to guarantee, on the whole-minute durations that are the
    /// awkward case: the adjusted end never precedes the reported end, and the adjusted duration
    /// is never shorter than the reported one.
    #[test]
    fn adjustment_invariants_hold_on_whole_minute_durations() {
        let cases = [
            ("2026-06-06 14:59", "2026-06-06 16:36", "1:37:00"),
            ("2026-06-07 05:46", "2026-06-07 06:19", "0:33:00"),
            ("2026-06-15 01:45", "2026-06-15 01:55", "0:10:00"),
        ];
        for (start, end, conn) in cases {
            let mut anomalies = Vec::new();
            let s = session(start, end, conn);
            let rows = s.resolve(&tz(), 1, &mut anomalies).unwrap();
            let row = &rows[0];
            assert!(
                row.adj_end_utc >= row.end_utc,
                "{start}: adjusted end precedes reported end"
            );
            assert!(
                row.adj_end_utc.duration_since(row.start_utc) >= s.conn_duration,
                "{start}: adjusted duration shorter than Conn_Duration"
            );
        }
    }

    #[test]
    fn utc_conversion_uses_edt_in_june() {
        let mut anomalies = Vec::new();
        let rows = session("2026-06-01 16:22", "2026-06-01 21:29", "5:07:53")
            .resolve(&tz(), 1, &mut anomalies)
            .unwrap();
        assert_eq!(
            rows[0].start_utc.to_zoned(TimeZone::UTC).datetime(),
            civil::date(2026, 6, 1).at(20, 22, 0, 0)
        );
    }

    /// A long session starting inside the Nov 1 fold: the reported end rules out one offset.
    #[test]
    fn dst_fold_resolved_by_reported_end() {
        let mut anomalies = Vec::new();
        // 01:30 EDT + 3h elapsed = 03:30 EST. Starting at 01:30 EST would end at 04:30.
        let rows = session("2026-11-01 01:30", "2026-11-01 03:30", "3:00:00")
            .resolve(&tz(), 1, &mut anomalies)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].start_utc.to_zoned(TimeZone::UTC).datetime(),
            civil::date(2026, 11, 1).at(5, 30, 0, 0), // EDT is UTC-4
        );
        assert!(anomalies.is_empty());
    }

    /// A short session wholly inside the repeated hour: neither offset can be ruled out, so the
    /// record is duplicated with distinct ids.
    #[test]
    fn dst_fold_ambiguous_duplicates_the_record() {
        let mut anomalies = Vec::new();
        let rows = session("2026-11-01 01:10", "2026-11-01 01:40", "0:30:00")
            .resolve(&tz(), 1, &mut anomalies)
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "S1-EDT");
        assert_eq!(rows[1].id, "S1-EST");
        // The copies are an hour apart in real time, which is the whole point.
        assert_eq!(
            rows[1].start_utc.duration_since(rows[0].start_utc),
            SignedDuration::from_hours(1)
        );
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].kind, AnomalyKind::DstAmbiguousDuplicated);
    }

    /// A wall time that never occurred, on the March 8 spring-forward.
    #[test]
    fn dst_gap_resolves_forward_and_reports() {
        let mut anomalies = Vec::new();
        let rows = session("2026-03-08 02:30", "2026-03-08 04:00", "0:30:00")
            .resolve(&tz(), 1, &mut anomalies)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(local_of(rows[0].start_utc), dt("2026-03-08 03:30"));
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].kind, AnomalyKind::DstGapShifted);
    }

    /// The case local arithmetic gets wrong: a session spanning the fold. Wall clock says 2 hours,
    /// elapsed is 3.
    #[test]
    fn fold_spanning_session_has_true_elapsed_duration() {
        let mut anomalies = Vec::new();
        let rows = session("2026-11-01 00:30", "2026-11-01 02:30", "3:00:00")
            .resolve(&tz(), 1, &mut anomalies)
            .unwrap();
        let row = &rows[0];
        let elapsed = row.adj_end_utc.duration_since(row.start_utc);
        assert!(
            elapsed >= SignedDuration::from_hours(3),
            "elapsed {elapsed:?} lost the repeated hour"
        );
        // The same subtraction done on local wall times loses the repeated hour.
        let wall_secs =
            excel_serial(row.adj_end_local).unwrap() - excel_serial(row.start_local).unwrap();
        assert!(
            wall_secs * 86_400.0 < elapsed.as_secs() as f64,
            "local subtraction should undercount here"
        );
    }

    #[test]
    fn zero_active_charge_time_with_energy_is_reported() {
        let mut anomalies = Vec::new();
        let mut s = session("2026-06-01 10:00", "2026-06-01 10:00", "0:00:00");
        s.energy_use = 5.0;
        let rows = s.resolve(&tz(), 7, &mut anomalies).unwrap();
        assert!(!rows[0].avg_power_computable);
        assert_eq!(anomalies[0].kind, AnomalyKind::ZeroActiveChargeTime);
        assert_eq!(anomalies[0].row, 7);

        // Zero energy with zero charge time is not an anomaly: Avg_power is legitimately zero.
        let mut anomalies = Vec::new();
        let mut s = session("2026-06-01 10:00", "2026-06-01 10:00", "0:00:00");
        s.energy_use = 0.0;
        let rows = s.resolve(&tz(), 7, &mut anomalies).unwrap();
        assert!(rows[0].avg_power_computable);
        assert!(anomalies.is_empty());
    }

    const FIXTURE: &str = "\
UR_ID,Location_Address,Location_City,Location_Postal_Code,Station_ID,Station_Network_Provider,Station_Make,Station_Model,Charge_Session_ID,User_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Charge_Duration,Active_Charge_Time,Charging_Level,Energy_Use,Total_Fee,Vehicle_Make,Vehicle_Model,Vehicle_Year
CKT-7,,Toronto,,Station-7,Evolute Inc.,FLO,G5,S69865,,2026-06-01 16:22,2026-06-01 21:29,5:07:53,5:07:53,5:07:52,Level 2,30.6,5.63,VinFast,Vf8,2024
CKT-7,,Toronto,,Station-7,Evolute Inc.,FLO,G5,S13577,,2026-06-02 08:00,2026-06-02 08:00,0:00:11,0:00:11,0:00:10,Level 2,0,0,VinFast,Vf8,2024
";

    #[test]
    fn round_trip_produces_the_expected_workbook() {
        let dir = std::env::temp_dir().join(format!("ev_peak_excel_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let csv_path = dir.join("Session_Report_Test.csv");
        fs::write(&csv_path, FIXTURE).unwrap();

        let report = session_csv_to_xlsx(&csv_path).unwrap();
        assert_eq!(report.output_path, dir.join("Session_Report_Test.xlsx"));
        assert!(report.anomalies.is_empty(), "{:?}", report.anomalies);

        let book = umya_spreadsheet::reader::xlsx::read(&report.output_path).unwrap();
        let sheet = book.sheet(0).unwrap();

        // Header row, in the agreed order.
        let expected: Vec<&str> = COLUMNS.iter().map(|(h, _)| *h).collect();
        for (i, header) in expected.iter().enumerate() {
            assert_eq!(&sheet.value((i as u32 + 1, 1)), header);
        }

        let col = |s: Source| column_index(s) as u32;

        // Adj_conn_end = 21:29:59 local on the first row.
        let adj: f64 = sheet
            .value((col(Source::AdjConnEndLocal), 2))
            .parse()
            .unwrap();
        assert!((adj - 46_174.895_821_759_3).abs() < 1e-9, "{adj}");

        // Formulas, not cached values.
        assert_eq!(
            sheet
                .cell((col(Source::AdjConnDuration), 2))
                .unwrap()
                .formula(),
            "P2-L2"
        );
        assert_eq!(
            sheet.cell((col(Source::AvgPower), 2)).unwrap().formula(),
            "IF(V2=0,0,V2/(T2*24))"
        );

        // Sheet name is the output file's name, minus the .xlsx suffix.
        assert_eq!(sheet.name(), "Session_Report_Test");

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
                .style((col(Source::AvgPower), 2))
                .number_format()
                .unwrap()
                .format_code(),
            AVG_POWER_FORMAT
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

        fs::remove_dir_all(&dir).ok();
    }
}
