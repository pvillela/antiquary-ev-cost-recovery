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
const AVG_POWER_FORMAT: &str = "0.000";
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
    AvgPower,
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
    ("Avg_power", Source::AvgPower),
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
///   value it derives from, and `Adj_conn_end`, `Adj_conn_duration` and `Avg_power` are inserted
///   as described in README.md.
/// - Timestamp columns are Excel date/time numbers formatted `yyyy-mm-dd hh:mm:ss ddd`, left-
///   justified; duration columns are Excel durations formatted `[h]:mm:ss`, which does not wrap
///   past 24 hours, and are centered.
/// - `Adj_conn_duration` and `Avg_power` are live formulas. `Adj_conn_duration` subtracts the two
///   *UTC* columns, so it is true elapsed time even across a DST fold; `Avg_power` is
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

        // Avg_power is a division by Active_Charge_Time. The sheet shows it as #DIV/0!; it is
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
            let avg_power = self.energy_use / (self.active_charge_time.as_secs_f64() / 3600.0);
            if avg_power > BREAKER_RATING_KW {
                common.push(AnomalyKind::ExcessiveAvgPower);
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
                Source::AvgPower => {
                    // Written unconditionally: with zero Active_Charge_Time
                    // this evaluates to #DIV/0!, which is the honest answer — energy delivered in
                    // no time at all has no finite average power.
                    sheet.cell_mut((col, excel_row)).set_formula(format!(
                        "{energy_col}{excel_row}/({active_col}{excel_row}*24)"
                    ));
                    set_format(sheet, col, excel_row, AVG_POWER_FORMAT);
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
            Source::AvgPower,
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
    /// consumes unaltered. A session with zero `Energy_Use` belongs here — its `avg_power` is
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
/// `avg_power` is recomputed here rather than read from the sheet's `Avg_power` column, which
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
