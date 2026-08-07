use crate::{Anomaly, AnomalyKind, BREAKER_RATING_KW, Session, TIME_GRID_STEP, time_zone};
use jiff::{
    SignedDuration, Timestamp, civil,
    tz::{AmbiguousOffset, TimeZone},
};
use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    time::Duration,
};
use umya_spreadsheet::{Comment, HorizontalAlignmentValues, Workbook, Worksheet};

/// Excel's day-zero for the 1900 date system, as a Unix timestamp.
/// 1899-12-30T00:00:00Z; verified by [`test::excel_epoch_constant_matches_jiff`].
const EXCEL_EPOCH_UNIX_SECS: i64 = -2_209_161_600;

/// [`TIME_GRID_STEP`] in the signed form the timestamp arithmetic needs.
/// See README.md, "Excel workbook".
const END_PADDING: SignedDuration = SignedDuration::from_secs(TIME_GRID_STEP.as_secs() as i64);

/// Widest gap between `Conn_start + Conn_Duration` and the reported end that truncation alone can
/// explain. Both reported timestamps are truncated to the minute while `Conn_Duration` carries
/// seconds, so for a consistent record the two land strictly within a minute of each other, either
/// side. See README.md, "Time zone".
const TRUNCATION_SLACK: SignedDuration = SignedDuration::from_secs(60);

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
    ("Conn_start_UTC", Source::ConnStartUtc),
    ("Conn_DateTime_End", Source::ConnEndLocal),
    ("Conn_end_UTC", Source::ConnEndUtc),
    ("Adj_conn_end", Source::AdjConnEndLocal),
    ("Adj_conn_end_UTC", Source::AdjConnEndUtc),
    ("Conn_Duration", Source::Duration("Conn_Duration")),
    ("Adj_conn_duration", Source::AdjConnDuration),
    ("Charge_Duration", Source::Duration("Charge_Duration")),
    ("Active_Charge_Time", Source::Duration("Active_Charge_Time")),
    ("Charging_Level", Source::Text("Charging_Level")),
    ("Energy_Use", Source::Number("Energy_Use")),
    ("Avg_kw", Source::AvgKw),
    ("Total_Fee", Source::Number("Total_Fee")),
    ("Vehicle_Make", Source::Text("Vehicle_Make")),
    ("Vehicle_Model", Source::Text("Vehicle_Model")),
    ("Vehicle_Year", Source::Number("Vehicle_Year")),
    ("Anomalies", Source::Anomalies),
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
/// `README.md` under "Time zone", "Excel workbook" and "Other". They are shared with the peak power
/// contribution logic and are not restated here.
///
/// What this function adds on top of those rules:
///
/// - Column order is given by the private `COLUMNS` table: each UTC column sits beside the local
///   value it derives from, and `Adj_conn_end`, `Adj_conn_duration` and `Avg_kw` are inserted
///   as described in README.md.
/// - Timestamp columns are Excel date/time numbers formatted `yyyy-mm-dd hh:mm:ss ddd`, left-
///   justified; duration columns are Excel durations formatted `[h]:mm:ss`, which does not wrap
///   past 24 hours, and are centered.
/// - `Adj_conn_duration` and `Avg_kw` are live formulas. `Adj_conn_duration` subtracts the two
///   *UTC* columns, so it is true elapsed time even across a DST fold; `Avg_kw` is
///   `=Energy_Use/(Active_Charge_Time*24)`, in kW, displayed to 3 decimal
///   places, matching `Energy_Use`. The formula is written on every row, so a session with
///   zero `Active_Charge_Time` shows `#DIV/0!` rather than an empty cell:
///   it delivered energy in no time at all, and the sheet says so. `Total_Fee` is displayed to
///   2 decimal places.
/// - The last column, `Anomalies`, carries the [`AnomalyKind`]s found for the row as a
///   comma-separated list of variant names. [`session_list`] reads it back, so it is the channel
///   by which a judgement call made here reaches the peak power contribution logic.
/// - The remaining columns are copied over with an explicit per-column type, so values that merely
///   look numeric — postal codes, station ids — keep their text form.
/// - The sheet is named by the private `sheet_name`.
///
/// Every session in the report is written to the workbook, anomalous ones included: the sheet is a
/// faithful rendering of the session report, and which sessions take part in an estimate is decided
/// on the reading side.
///
/// # Errors
///
/// Returns `Err` only for conditions that invalidate the whole file: it cannot be read, a required
/// header from the private `REQUIRED_HEADERS` is missing, a timestamp or duration does not parse,
/// or the workbook cannot be written. Per-row judgement calls do not abort the conversion; they are
/// collected in [`ConversionReport::anomalies`].
pub fn session_csv_to_xlsx(path: &Path) -> Result<ConversionReport, Box<dyn Error>> {
    let tz = time_zone();
    let (headers, records) = read_csv(path)?;

    let mut anomalies = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    for (i, record) in records.iter().enumerate() {
        let csv_row = i + 1;
        let session = CsvSession::parse(&headers, record, csv_row)?;
        // The workbook row is tracked separately from the CSV row: a record duplicated to resolve a
        // DST fold occupies two workbook rows, so from there on the two counts diverge.
        let excel_row = rows.len() + 2;
        for (offset, row) in session.resolve(&tz, csv_row)?.into_iter().enumerate() {
            anomalies.extend(row.anomalies.iter().map(|&kind| Anomaly {
                row: excel_row + offset,
                session_id: row.id.clone(),
                kind,
            }));
            rows.push(row);
        }
    }

    let output_path = path.with_extension("xlsx");
    let mut book = umya_spreadsheet::new_file();
    write_sheet(&mut book, &output_path, &headers, &records, &rows)?;

    umya_spreadsheet::writer::xlsx::write(&book, &output_path)?;
    Ok(ConversionReport {
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

/// The parsed fields of one CSV record that participate in the time calculations. Named apart
/// from [`Session`], which is the finished, UTC-resolved article this module hands to the peak
/// power contribution logic.
struct CsvSession {
    id: String,
    start_local: civil::DateTime,
    end_local: civil::DateTime,
    conn_duration: SignedDuration,
    active_charge_time: SignedDuration,
    /// Kept for its parse: a non-numeric `Energy_Use` invalidates the row, and that is caught in
    /// [`CsvSession::parse`]. The value itself is consumed on the reading side.
    #[allow(dead_code)]
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
    /// Everything about this row that needs review. Rendered into the `Anomalies` column and,
    /// stamped with the row's workbook number, into [`ConversionReport::anomalies`].
    anomalies: Vec<AnomalyKind>,
}

impl CsvSession {
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
    fn resolve(&self, tz: &TimeZone, row: usize) -> Result<Vec<Row>, Box<dyn Error>> {
        // Kinds known before the DST branch runs. They describe the record itself, so on
        // duplication both copies inherit them.
        let mut common = Vec::new();

        // Avg_kw is a division by Active_Charge_Time. The sheet shows it as #DIV/0!; it is
        // reported here so it is not left to be noticed by eye. Zero energy is no exception: 0/0
        // is just as undefined, and the session becomes a spike either way.
        if self.active_charge_time.is_zero() {
            common.push(AnomalyKind::ZeroActiveChargeTime);
        } else {
            // Above the breaker rating is something the hardware should not permit, so the record
            // says something is wrong with `Energy_Use` or `Active_Charge_Time` — but not which,
            // which is why this only reports and never excludes.
            //
            // Compared against the rating exactly, with no tolerance.
            let avg_kw = self.energy_use / (self.active_charge_time.as_secs_f64() / 3600.0);
            if avg_kw > BREAKER_RATING_KW {
                common.push(AnomalyKind::ExcessiveAvgKw);
            }
        }

        let ambiguous = tz.to_ambiguous_timestamp(self.start_local);
        let starts: Vec<(Timestamp, Option<&str>)> = match ambiguous.offset() {
            AmbiguousOffset::Unambiguous { .. } => {
                vec![(ambiguous.unambiguous()?, None)]
            }
            AmbiguousOffset::Gap { .. } => {
                // A wall time that never occurred. `compatible` moves to just after the gap; the
                // row is still written, but the shift is reported rather than silently applied.
                common.push(AnomalyKind::DstGapShifted);
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
                        common.push(AnomalyKind::DstAmbiguousDuplicated);
                        vec![(earlier, Some("EDT")), (later, Some("EST"))]
                    }
                    (false, false) => {
                        common.push(AnomalyKind::DstUnresolvable);
                        vec![(earlier, None)]
                    }
                }
            }
        };

        starts
            .into_iter()
            .map(|(start_utc, suffix)| {
                let end_utc = self.resolve_end(tz, start_utc)?;
                // See README.md, "Excel workbook".
                let adj_end_utc = end_utc + END_PADDING;

                let mut anomalies = common.clone();
                // Truncation puts the true start in `[start_utc, start_utc + END_PADDING)` and the
                // true end in `[end_utc, adj_end_utc)`. An honest `Conn_Duration` carries some
                // instant of the first window to some instant of the second, so the record is
                // sound exactly while the first window, shifted by the duration, still meets the
                // second. Both bounds are strict because both windows are half-open at the same
                // end. Outside the band the three fields contradict each other by more than the
                // reporting can explain, and the session is excluded from the estimates
                // downstream. See `AnomalyKind::InconsistentDuration`.
                let implied_end = start_utc + self.conn_duration;
                if implied_end >= adj_end_utc || implied_end <= end_utc - END_PADDING {
                    anomalies.push(AnomalyKind::InconsistentDuration);
                }

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
                    anomalies,
                })
            })
            .collect()
    }

    /// Does `start` plus the reported elapsed duration land back on the reported end?
    ///
    /// Both reported timestamps are truncated to the minute while `Conn_Duration` carries seconds,
    /// so for a consistent record `start + Conn_Duration` falls strictly within a minute of the
    /// reported end, on either side. Requiring equal minutes instead rejects roughly half of all
    /// consistent records — 116 of the 238 rows in this project's `data` directory.
    ///
    /// The comparison is made on *local wall time*, not on instants. That is what lets both fold
    /// candidates match a session short enough to fit inside the repeated hour, which is the very
    /// ambiguity this test exists to detect. The tolerance cannot blur the two candidates together
    /// otherwise: they lie a full hour apart.
    fn reproduces_reported_end(&self, tz: &TimeZone, start: Timestamp) -> bool {
        let end = (start + self.conn_duration).to_zoned(tz.clone()).datetime();
        match (wall_clock_instant(end), wall_clock_instant(self.end_local)) {
            (Ok(implied), Ok(reported)) => {
                implied.duration_since(reported).abs() < TRUNCATION_SLACK
            }
            _ => false,
        }
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

/// Reads a local wall time as though it were UTC, so that two of them can be subtracted to give the
/// wall-clock distance between them. Not a time-zone conversion: the point is to compare wall times
/// as written, without a DST offset moving either one.
fn wall_clock_instant(dt: civil::DateTime) -> Result<Timestamp, jiff::Error> {
    Ok(dt.to_zoned(TimeZone::UTC)?.timestamp())
}

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
                    if !row.anomalies.is_empty() {
                        let tokens: Vec<&str> =
                            row.anomalies.iter().map(AnomalyKind::as_str).collect();
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
            Source::AdjConnEndLocal,
            "Adjusted connection end: Conn_DateTime_End + 60s, and EXCLUSIVE. The report's \
             timestamps carry no seconds, so the true end is only known to fall somewhere in the \
             reported minute; the session is recorded as the half-open span [Conn_DateTime_Start, \
             Adj_conn_end), which contains it wherever in that minute it fell. Because the end is \
             excluded, a session starting at this exact time does NOT overlap this one. See \
             README.md, \"Excel workbook\".",
        ),
        (
            Source::AdjConnDuration,
            "Adj_conn_end_UTC - Conn_start_UTC. Computed from the UTC columns so it is true \
             elapsed time even for a session spanning the DST fold, where local arithmetic would \
             be wrong by an hour.",
        ),
        (
            Source::AvgKw,
            "Energy_Use / Active_Charge_Time, in kW. Active_Charge_Time is an Excel duration, i.e. \
             a fraction of a day, hence the *24 to convert it to hours. A zero Active_Charge_Time \
             yields #DIV/0!, which is the honest answer: energy delivered in no time at all has no \
             finite average power.",
        ),
        (
            Source::Anomalies,
            "Comma-separated list of anomalies found for this row, named after the AnomalyKind \
             variants. Empty means the row needed no judgement call. Read back by session_list, so \
             editing this cell changes which sessions take part in the estimates. See README.md.",
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

/// The sessions in a workbook produced by [`session_csv_to_xlsx`], sorted by how the peak power
/// contribution logic must treat them. The writing direction returns a [`ConversionReport`]
/// instead.
#[derive(Debug)]
pub struct SessionReport {
    /// Sessions with a finite average power. This is what the peak power contribution logic
    /// consumes unaltered. A session with zero `Energy_Use` belongs here — its `avg_kw` is
    /// legitimately zero, and it still occupies a breaker.
    pub sessions: Vec<Session>,
    /// Sessions with zero `Active_Charge_Time`, so [`Session::charge_time`] is zero and energy
    /// over charge time is infinite or `NaN`. Kept out of `sessions` because those values would
    /// swamp or poison any segment they entered, and surfaced rather than dropped because energy
    /// delivered in no time at all is exactly what a demand charge bills on.
    /// [`Session::avg_kw`] substitutes a finite figure for them. See README.md, "Other".
    pub spikes: Vec<Session>,
    /// Sessions flagged [`AnomalyKind::InconsistentDuration`]: their reported start, end and duration
    /// contradict each other, so they cannot be placed on a timeline at all. Excluded from the
    /// estimates and returned only for review. See README.md, "Other".
    pub excluded: Vec<Session>,
}

/// Sheet columns that make a workbook a session report. The reading-side counterpart of
/// [`REQUIRED_HEADERS`].
///
/// This is deliberately wider than the set [`session_list`] strictly consumes. A workbook missing
/// any of these is not a rendering of a session report, and guessing at its contents would produce
/// peak numbers that cannot be trusted. `Anomalies` in particular is load-bearing: without it every
/// session would silently look clean, and inconsistent ones would fold back into the estimates.
const REQUIRED_SHEET_HEADERS: &[&str] = &[
    "Charge_Session_ID",
    "Conn_start_UTC",
    "Conn_end_UTC",
    "Adj_conn_end_UTC",
    "Conn_Duration",
    "Active_Charge_Time",
    "Energy_Use",
    "Anomalies",
];

/// Sheet header name to its 1-based column number. Deliberately distinct from [`Headers`], whose
/// values are 0-based CSV field positions.
type SheetHeaders = HashMap<String, u32>;

/// Reads a workbook written by [`session_csv_to_xlsx`] and returns the charging sessions it
/// describes, ready for the peak power contribution logic.
///
/// Columns are located by the header names in row 1, not by position, so inserting or reordering
/// columns in the sheet does not silently shift what is read. Only the private
/// `REQUIRED_SHEET_HEADERS` are consulted; the first worksheet is used.
///
/// Sorting into the three buckets of [`SessionReport`] happens here rather than at conversion time,
/// because the workbook is meant to be a faithful rendering of the session report. The tests are
/// applied in this order, strongest first:
///
/// 1. Flagged [`AnomalyKind::InconsistentDuration`] — [`SessionReport::excluded`]. Such a session
///    takes no part in the estimates whatever its charge time, and letting one through would put an
///    inverted session in front of the segmenting logic, whose endpoints would then arrive out of
///    order.
/// 2. Zero `Active_Charge_Time` — [`SessionReport::spikes`].
/// 3. Everything else — [`SessionReport::sessions`].
///
/// `avg_kw` is recomputed here rather than read from the sheet's `Avg_kw` column, which
/// holds a formula whose cached value this crate never writes. For a spike that leaves it infinite
/// or `NaN`, which is the honest reading; the estimating logic substitutes a finite value.
///
/// # Errors
///
/// Returns `Err` if the workbook cannot be read, a required column is missing, any cell in a row
/// that has a `Charge_Session_ID` does not hold the number it should, or the `Anomalies` column
/// holds a token that is not an [`AnomalyKind`] variant name. A workbook that cannot be read in
/// full is one whose peak numbers cannot be trusted, so no row is skipped quietly.
/// Rows with no `Charge_Session_ID` at all are treated as trailing blanks and ignored.
pub fn session_list(path: &Path) -> Result<SessionReport, Box<dyn Error>> {
    let book = umya_spreadsheet::reader::xlsx::read(path)?;
    let sheet = book.sheet(0)?;
    let headers = sheet_headers(sheet, path)?;

    let mut sessions = Vec::new();
    let mut spikes = Vec::new();
    let mut excluded = Vec::new();
    for row in 2..=sheet.highest_row() {
        let id = sheet
            .value((headers["Charge_Session_ID"], row))
            .trim()
            .to_owned();
        if id.is_empty() {
            continue;
        }

        let energy_use = number(sheet, &headers, "Energy_Use", row)?;
        let charge_time = duration_of(number(sheet, &headers, "Active_Charge_Time", row)?);
        let anomalies = anomaly_kinds(sheet, &headers, row, path)?;
        let session = Session {
            id,
            row: row as usize,
            conn_start: timestamp_of(number(sheet, &headers, "Conn_start_UTC", row)?)?,
            conn_end: timestamp_of(number(sheet, &headers, "Conn_end_UTC", row)?)?,
            adj_conn_end: timestamp_of(number(sheet, &headers, "Adj_conn_end_UTC", row)?)?,
            conn_duration: duration_of(number(sheet, &headers, "Conn_Duration", row)?),
            charge_time,
            energy_use,
            anomalies,
        };

        if session
            .anomalies
            .contains(&AnomalyKind::InconsistentDuration)
        {
            excluded.push(session);
        } else if charge_time.is_zero() {
            spikes.push(session);
        } else {
            sessions.push(session);
        }
    }

    Ok(SessionReport {
        sessions,
        spikes,
        excluded,
    })
}

/// Parses the `Anomalies` cell. An unrecognised token is an error rather than a shrug: it means the
/// workbook was written by something this crate does not know, and the sessions it excludes cannot
/// be determined.
fn anomaly_kinds(
    sheet: &Worksheet,
    headers: &SheetHeaders,
    row: u32,
    path: &Path,
) -> Result<Vec<AnomalyKind>, Box<dyn Error>> {
    sheet
        .value((headers["Anomalies"], row))
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            AnomalyKind::from_token(token).ok_or_else(|| -> Box<dyn Error> {
                format!(
                    "{}: row {row}, column `Anomalies`: unknown anomaly {token:?}",
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

/// Inverse of [`excel_serial_utc`]. Rounds to the nearest second: the writer stores whole seconds,
/// and truncating what comes back would turn `20:22:00` into `20:21:59`.
fn timestamp_of(serial: f64) -> Result<Timestamp, Box<dyn Error>> {
    Ok(Timestamp::from_second(
        (serial * 86_400.0).round() as i64 + EXCEL_EPOCH_UNIX_SECS,
    )?)
}

/// Inverse of [`excel_duration`], rounded to the nearest second for the same reason.
fn duration_of(days: f64) -> Duration {
    Duration::from_secs((days * 86_400.0).round().max(0.0) as u64)
}

#[cfg(test)]
// cargo test --package ev-peak-contrib --lib --all-features -- excel::test --nocapture
mod test {
    use super::*;
    use crate::time_zone;
    use std::fs;

    /// A row's anomalies with [`AnomalyKind::ExcessiveAvgKw`] removed.
    ///
    /// Nearly every test here is about *timestamps* — DST resolution, the `Adj_conn_end` padding,
    /// the consistency band — and each fixture states an `Energy_Use` and an `Active_Charge_Time`
    /// as fixed text. Whether the average power those imply clears `BREAKER_RATING_KW` therefore
    /// depends on the value of `BREAKER_RATING_A`, and no test may depend on that: lower the
    /// breaker rating and a dozen tests about the DST fold would start failing over a flag that
    /// has nothing to do with what they check.
    ///
    /// Filtering the one power-dependent kind out is what keeps them testing what they are named
    /// for. `ExcessiveAvgKw` is checked where it belongs — against the rating rather than
    /// against a number — in `tests/segment_tiling.rs`.
    fn timing_anomalies(anomalies: &[AnomalyKind]) -> Vec<AnomalyKind> {
        anomalies
            .iter()
            .copied()
            .filter(|k| *k != AnomalyKind::ExcessiveAvgKw)
            .collect()
    }

    /// The same filter applied to an `Anomalies` cell, read back through the wire format.
    ///
    /// Going through [`AnomalyKind::from_token`] rather than comparing the cell text also checks
    /// that what was written is what can be read back, which is the property the column exists for.
    fn timing_anomalies_in_cell(cell: &str) -> Vec<AnomalyKind> {
        let kinds: Vec<AnomalyKind> = cell
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| {
                AnomalyKind::from_token(t).unwrap_or_else(|| panic!("unreadable token {t:?}"))
            })
            .collect();
        timing_anomalies(&kinds)
    }

    fn dt(s: &str) -> civil::DateTime {
        civil::DateTime::strptime("%Y-%m-%d %H:%M", s).unwrap()
    }

    fn session(start: &str, end: &str, conn: &str) -> CsvSession {
        let active_charge_time = parse_duration(conn, 1, "Active_Charge_Time").unwrap();
        CsvSession {
            id: "S1".to_owned(),
            start_local: dt(start),
            end_local: dt(end),
            conn_duration: parse_duration(conn, 1, "Conn_Duration").unwrap(),
            active_charge_time,
            // 6 kW, under the breaker rating, so a record built here carries only the anomaly the
            // test that built it is about. A flat energy figure would draw far above the rating on
            // the shorter durations and pick up `ExcessiveAvgKw` throughout.
            energy_use: 6.0 * active_charge_time.as_secs_f64() / 3600.0,
        }
    }

    fn local_of(ts: Timestamp) -> civil::DateTime {
        ts.to_zoned(time_zone()).datetime()
    }

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
    fn excel_serial_round_trips_to_the_second() {
        for local in [
            civil::date(2026, 6, 1).at(20, 22, 0, 0),
            civil::date(2026, 6, 7).at(23, 41, 28, 0),
            civil::date(2026, 11, 1).at(5, 30, 0, 0),
            civil::date(1900, 1, 1).at(0, 0, 1, 0),
        ] {
            let ts = local.to_zoned(TimeZone::UTC).unwrap().timestamp();
            assert_eq!(timestamp_of(excel_serial_utc(ts).unwrap()).unwrap(), ts);
        }
    }

    #[test]
    fn excel_duration_round_trips_to_the_second() {
        for secs in [0, 1, 59, 3600, 5 * 3600 + 7 * 60 + 53, 30 * 3600] {
            let days = excel_duration(SignedDuration::from_secs(secs));
            assert_eq!(duration_of(days), Duration::from_secs(secs as u64));
        }
    }

    #[test]
    fn column_letters_span_past_z() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(COLUMNS.len()), "AB");
    }

    /// `Adj_conn_end` is the reported end padded past the end of its minute — the exclusive end of
    /// the window the true end lies in, so `21:29` pads to `21:30:00` and not `21:29:59`. Both rows
    /// are real sample rows, and they straddle the case the old `min(...)` rule treated specially:
    /// the second has `start + duration` (23:40:29) *before* the reported end.
    #[test]
    fn adj_conn_end_pads_the_reported_end() {
        let rows = session("2026-06-01 16:22", "2026-06-01 21:29", "5:07:53")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(
            local_of(rows[0].adj_end_utc),
            civil::date(2026, 6, 1).at(21, 30, 0, 0)
        );
        assert!(timing_anomalies(&rows[0].anomalies).is_empty());

        let rows = session("2026-06-07 16:42", "2026-06-07 23:41", "6:58:29")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(
            local_of(rows[0].adj_end_utc),
            civil::date(2026, 6, 7).at(23, 42, 0, 0)
        );
        assert!(timing_anomalies(&rows[0].anomalies).is_empty());
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
            let s = session(start, end, conn);
            let rows = s.resolve(&time_zone(), 1).unwrap();
            let row = &rows[0];
            assert!(
                row.adj_end_utc >= row.end_utc,
                "{start}: adjusted end precedes reported end"
            );
            assert!(
                row.adj_end_utc.duration_since(row.start_utc) >= s.conn_duration,
                "{start}: adjusted duration shorter than Conn_Duration"
            );
            assert!(
                timing_anomalies(&row.anomalies).is_empty(),
                "{start}: unexpected {:?}",
                row.anomalies
            );
        }
    }

    #[test]
    fn utc_conversion_uses_edt_in_june() {
        let rows = session("2026-06-01 16:22", "2026-06-01 21:29", "5:07:53")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(
            rows[0].start_utc.to_zoned(TimeZone::UTC).datetime(),
            civil::date(2026, 6, 1).at(20, 22, 0, 0)
        );
    }

    /// A long session starting inside the Nov 1 fold: the reported end rules out one offset.
    #[test]
    fn dst_fold_resolved_by_reported_end() {
        // 01:30 EDT + 3h elapsed = 03:30 EST. Starting at 01:30 EST would end at 04:30.
        let rows = session("2026-11-01 01:30", "2026-11-01 03:30", "3:00:00")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].start_utc.to_zoned(TimeZone::UTC).datetime(),
            civil::date(2026, 11, 1).at(5, 30, 0, 0), // EDT is UTC-4
        );
        assert!(timing_anomalies(&rows[0].anomalies).is_empty());
    }

    /// The mirror of the test above, and the case a one-sided `start + duration <= Adj_conn_end`
    /// test would get wrong: here EST is correct, and the EDT candidate lands a full hour *early*.
    /// Only a two-sided comparison rejects it; accepting it would duplicate a session that is not
    /// ambiguous at all, double-counting its power.
    #[test]
    fn dst_fold_resolved_to_est_rejects_the_hour_early_candidate() {
        // 01:30 EST + 3h elapsed = 04:30 EST. Starting at 01:30 EDT would end at 03:30.
        let rows = session("2026-11-01 01:30", "2026-11-01 04:30", "3:00:00")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(rows.len(), 1, "should not duplicate: {:?}", rows[0].id);
        assert_eq!(
            rows[0].start_utc.to_zoned(TimeZone::UTC).datetime(),
            civil::date(2026, 11, 1).at(6, 30, 0, 0), // EST is UTC-5
        );
        assert!(timing_anomalies(&rows[0].anomalies).is_empty());
    }

    /// Reported times are truncated to the minute while `Conn_Duration` carries seconds, so a
    /// consistent record's `start + duration` lands up to a minute *either side* of the reported
    /// end. Requiring equal minutes rejected roughly half of all real records; on a fold start that
    /// meant a spurious `DstUnresolvable` and UTC timestamps an hour early.
    #[test]
    fn fold_resolves_when_start_plus_duration_falls_short_of_the_reported_minute() {
        // 01:30 EDT + 2:59:31 = 03:29:31 local, which truncates to 03:29, not the reported 03:30.
        // The EST candidate lands at 04:29:31, an hour out, so only EDT is consistent — but the old
        // equal-minutes test rejected *both* and called the record unresolvable.
        let rows = session("2026-11-01 01:30", "2026-11-01 03:30", "2:59:31")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            !rows[0].anomalies.contains(&AnomalyKind::DstUnresolvable),
            "spurious DstUnresolvable: {:?}",
            rows[0].anomalies
        );
        assert_eq!(
            rows[0].start_utc.to_zoned(TimeZone::UTC).datetime(),
            civil::date(2026, 11, 1).at(5, 30, 0, 0), // EDT is UTC-4
        );
    }

    /// A short session wholly inside the repeated hour: neither offset can be ruled out, so the
    /// record is duplicated with distinct ids.
    #[test]
    fn dst_fold_ambiguous_duplicates_the_record() {
        let rows = session("2026-11-01 01:10", "2026-11-01 01:40", "0:30:00")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "S1-EDT");
        assert_eq!(rows[1].id, "S1-EST");
        // The copies are an hour apart in real time, which is the whole point.
        assert_eq!(
            rows[1].start_utc.duration_since(rows[0].start_utc),
            SignedDuration::from_hours(1)
        );
        // Both copies carry the flag, so each workbook row says why it is there.
        for row in &rows {
            assert_eq!(
                timing_anomalies(&row.anomalies),
                vec![AnomalyKind::DstAmbiguousDuplicated]
            );
        }
    }

    /// A wall time that never occurred, on the March 8 spring-forward.
    #[test]
    fn dst_gap_resolves_forward_and_reports() {
        let rows = session("2026-03-08 02:30", "2026-03-08 04:00", "0:30:00")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(local_of(rows[0].start_utc), dt("2026-03-08 03:30"));
        assert_eq!(
            timing_anomalies(&rows[0].anomalies),
            vec![AnomalyKind::DstGapShifted]
        );
    }

    /// The case local arithmetic gets wrong: a session spanning the fold. Wall clock says 2 hours,
    /// elapsed is 3.
    #[test]
    fn fold_spanning_session_has_true_elapsed_duration() {
        let rows = session("2026-11-01 00:30", "2026-11-01 02:30", "3:00:00")
            .resolve(&time_zone(), 1)
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

    /// Zero `Active_Charge_Time` is flagged whatever the energy: the cell shows `#DIV/0!` either
    /// way, and the session becomes a spike either way.
    #[test]
    fn zero_active_charge_time_is_reported() {
        for energy in [5.0, 0.0] {
            let mut s = session("2026-06-01 10:00", "2026-06-01 10:00", "0:00:00");
            s.energy_use = energy;
            let rows = s.resolve(&time_zone(), 7).unwrap();
            assert_eq!(
                timing_anomalies(&rows[0].anomalies),
                vec![AnomalyKind::ZeroActiveChargeTime],
                "energy {energy}"
            );
        }
    }

    /// `Conn_start + Conn_Duration` must land strictly inside one `TIME_GRID_STEP`
    /// of the reported end, on either side; truncation to the minute explains that much and no
    /// more. Both boundaries are pinned, because getting either off by a second would silently
    /// reclassify real records: the sample data reaches −57s, so the band is exercised almost to
    /// its edge. The exclusive cases are pinned too — landing *exactly* a minute out is a fault,
    /// since a sound record's two truncation windows would then merely touch, not meet.
    #[test]
    fn inconsistent_duration_is_reported() {
        let kinds = |start, end, conn| {
            let all = session(start, end, conn)
                .resolve(&time_zone(), 1)
                .unwrap()
                .swap_remove(0)
                .anomalies;
            timing_anomalies(&all)
        };
        let bad = vec![AnomalyKind::InconsistentDuration];

        // Overshoot: 10:00 + 2h = 12:00, well past the 10:31:00 upper bound.
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 10:30", "2:00:00"),
            bad
        );
        // Ends before it starts — the extreme of the overshoot direction, needing no rule of its own.
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 09:00", "0:10:00"),
            bad
        );
        // Exactly on the bounds, which are exclusive: 10:31:00 and 10:29:00. These are the cases
        // that separate the half-open reading from the closed one.
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 10:30", "0:31:00"),
            bad
        );
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 10:30", "0:29:00"),
            bad
        );

        // One second inside each bound, and both sound.
        assert!(kinds("2026-06-01 10:00", "2026-06-01 10:30", "0:30:59").is_empty());
        assert!(kinds("2026-06-01 10:00", "2026-06-01 10:30", "0:29:01").is_empty());
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

        // Adj_conn_end = 21:30:00 local on the first row — the exclusive end of the minute the
        // reported 21:29 end falls in.
        let adj: f64 = sheet
            .value((col(Source::AdjConnEndLocal), 2))
            .parse()
            .unwrap();
        assert!((adj - 46_174.895_833_333_3).abs() < 1e-9, "{adj}");

        // Formulas, not cached values.
        assert_eq!(
            sheet
                .cell((col(Source::AdjConnDuration), 2))
                .unwrap()
                .formula(),
            "P2-L2"
        );
        assert_eq!(
            sheet.cell((col(Source::AvgKw), 2)).unwrap().formula(),
            "V2/(T2*24)"
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

        // Avg_kw is written on every row, the zero-energy one included, so a row that would
        // divide by zero shows #DIV/0! rather than nothing at all.
        assert_eq!(
            sheet.cell((col(Source::AvgKw), 3)).unwrap().formula(),
            "V3/(T3*24)"
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

    /// The two anomaly items in the conversion report carry *workbook* rows, not CSV rows.
    #[test]
    fn conversion_report_anomalies_carry_excel_rows() {
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
            .map(|a| (a.row, a.session_id.as_str(), a.kind))
            .collect();
        assert_eq!(
            items,
            [
                (2, "S1-EDT", AnomalyKind::DstAmbiguousDuplicated),
                (3, "S1-EST", AnomalyKind::DstAmbiguousDuplicated),
                // CSV row 2, but workbook row 4: the duplication above pushed it down.
                (4, "S2", AnomalyKind::ZeroActiveChargeTime),
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
        assert!(report.excluded[0].adj_conn_end < report.excluded[0].conn_start);

        fs::remove_dir_all(xlsx.parent().unwrap()).ok();
    }

    /// An `Anomalies` cell this crate did not write is an error, not something to shrug at: it
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

    #[test]
    fn session_list_reads_back_what_was_written() {
        let xlsx = convert("session_list", FIXTURE);
        let report = session_list(&xlsx).unwrap();

        // A zero-Energy_Use session with a real charge time is an ordinary session: its average
        // power is legitimately zero and it still occupies a breaker. See README.md, "Other".
        assert!(report.spikes.is_empty());
        assert!(report.excluded.is_empty());
        assert_eq!(report.sessions.len(), 2);
        assert_eq!(report.sessions[1].id, "S13577");
        assert_eq!(report.sessions[1].avg_kw(), 0.0);

        let s = &report.sessions[0];
        assert_eq!(s.id, "S69865");
        assert_eq!(s.row, 2);
        assert!(timing_anomalies(&s.anomalies).is_empty());
        // 16:22 EDT is 20:22 UTC.
        assert_eq!(s.conn_start, utc(civil::date(2026, 6, 1).at(20, 22, 0, 0)));
        // The reported end, 21:29 EDT, unadjusted.
        assert_eq!(s.conn_end, utc(civil::date(2026, 6, 2).at(1, 29, 0, 0)));
        // The adjusted end: 21:30:00 EDT, exclusive.
        assert_eq!(s.adj_conn_end, utc(civil::date(2026, 6, 2).at(1, 30, 0, 0)));
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
            (ordinary.avg_kw() - ordinary.energy_use / (ordinary.charge_time.as_secs_f64() / 3600.0))
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
        assert!(err.contains("Conn_start_UTC"), "{err}");
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
