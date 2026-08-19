use crate::time::{
    duration_of_serial, instant_of_serial, is_on_grid, local_datetime, serial_of_civil,
    serial_of_duration, serial_of_instant, time_zone, wall_clock_instant,
};

use super::TIME_GRID_STEP;
use super::{Anomaly, AnomalyKind, BREAKER_RATING_KW, RunLog, Session, duration_is_consistent};
use jiff::{
    SignedDuration, Timestamp, civil,
    tz::{AmbiguousOffset, TimeZone},
};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    path::{Path, PathBuf},
    time::Duration,
};
use umya_spreadsheet::{Comment, HorizontalAlignmentValues, Workbook, Worksheet};

/// The window `Conn_start + Conn_Duration` may land in, stated as an offset from the reported end.
///
/// Both bounds are exclusive, and the window is **asymmetric**: it is
/// [`duration_is_consistent`]'s checks 2 and 3 with the reported end subtracted from each side.
/// The extra second on the late side is there because the reported end is not only truncated —
/// it is also unknown whether the reporting includes or excludes its last second. See
/// `docs/sessions/time-reporting-uncertainty.md`.
///
/// Derived from [`TIME_GRID_STEP`] rather than written out, so a change to the grid moves this
/// with it.
const SLACK_EARLY: SignedDuration = SignedDuration::from_secs(-(TIME_GRID_STEP.as_secs() as i64));
const SLACK_LATE: SignedDuration = SignedDuration::from_secs(TIME_GRID_STEP.as_secs() as i64 + 1);

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
/// The domain rules — the UTC conversion and its DST policy, the definitions of `adj_conn_end` and
/// `adj_conn_duration`, and the treatment of zero-`Energy_Use` sessions — are specified in
/// `README.md` under "Time zone", "Excel workbook" and "Other". They are shared with the peak power
/// contribution logic and are not restated here.
///
/// What this function adds on top of those rules:
///
/// - Column order is given by the private `COLUMNS` table: each UTC column sits beside the local
///   value it derives from, and `adj_conn_end`, `adj_conn_duration` and `avg_kw` are inserted
///   as described in README.md.
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
            anomalies.extend(row.session.anomalies.iter().map(|&kind| Anomaly {
                row: excel_row + offset,
                session_id: row.session.id.clone(),
                kind,
            }));
            rows.push(row);
        }
    }

    let output_path = path.with_extension("xlsx");
    let mut book = umya_spreadsheet::new_file();
    write_sheet(&mut book, &output_path, &headers, &records, &rows)?;

    umya_spreadsheet::writer::xlsx::write(&book, &output_path)?;

    // Anomalies only: the write side has nothing to compare against, since it is what produces the
    // values in the first place. See `sessions::log` for why discrepancies are a separate channel.
    let mut log = RunLog::new();
    note_off_grid_rows(&rows, &mut log);
    for anomaly in &anomalies {
        log.note(anomaly.to_string());
    }
    let log_path = log.write_beside(&output_path, "convert", "Converted from session report")?;

    Ok(ConversionReport {
        output_path,
        anomalies,
        log_path,
    })
}

/// Warns once per file when reported boundaries do not land on [`TIME_GRID_STEP`].
///
/// Every allowance this software makes for the reporting's truncation assumes the reported times
/// are truncated to that step. If Evolute starts reporting seconds, they no longer are, and the
/// allowances become too wide rather than wrong — sessions get a padded end they do not need, and
/// the consistency window admits records it should reject. Nothing crashes and no figure looks
/// odd, which is exactly why it needs saying out loud.
///
/// Once per file with a count and the first three rows, not once per row: on a report that has
/// switched resolution every row qualifies, and a log with 238 identical lines is a log nobody
/// reads.
fn note_off_grid_rows(rows: &[Row], log: &mut RunLog) {
    let offenders: Vec<&Row> = rows
        .iter()
        .filter(|r| {
            !is_on_grid(r.session.conn_start, TIME_GRID_STEP)
                || !is_on_grid(r.session.conn_end, TIME_GRID_STEP)
        })
        .collect();
    if offenders.is_empty() {
        return;
    }
    let examples: Vec<String> = offenders
        .iter()
        .take(3)
        .map(|r| format!("row {} ({})", r.session.row, r.session.id))
        .collect();
    log.note(format!(
        "{} of {} rows report a start or end that is not a whole multiple of {:?}: {}. The \
         session report's resolution has become finer than this software's time grid. Nothing is \
         wrong with these rows, but the padding and the consistency window are now wider than the \
         data needs — see docs/maintenance-manual.md, \"Boundaries and the time grid\".",
        offenders.len(),
        rows.len(),
        TIME_GRID_STEP,
        examples.join(", ")
    ));
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

/// Local time as `YYYY-MM-DD HH:MM`; currently, the report carries no seconds, which is what makes
/// `adj_conn_end` necessary in the first place. However, if seconds are added in the future,
/// we want to be able to handle that.
fn parse_local(s: &str, row: usize, column: &str) -> Result<civil::DateTime, Box<dyn Error>> {
    // 1. Try parsing with seconds first
    if let Ok(dt) = civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", s) {
        return Ok(dt);
    }

    // 2. Fall back to parsing without seconds (seconds default to 00)
    civil::DateTime::strptime("%Y-%m-%d %H:%M", s).map_err(|e| {
        format!("row {row}, column `{column}`: cannot parse timestamp {s:?}: {e}").into()
    })
}

/// `H:MM:SS`, with hours unbounded so a session longer than a day still parses.
///
/// Returns an unsigned [`Duration`], matching [`Session`]'s own fields. The sign is rejected here
/// rather than carried: `Conn_Duration` and `Active_Charge_Time` are elapsed times, and a negative
/// one is a malformed cell, not a value to propagate. Only the DST-fold comparison in
/// [`CsvSession::reproduces_reported_end`] genuinely needs a sign, and it makes its own.
fn parse_duration(s: &str, row: usize, column: &str) -> Result<Duration, Box<dyn Error>> {
    let bad = || -> Box<dyn Error> {
        format!("row {row}, column `{column}`: cannot parse duration {s:?}").into()
    };
    let mut parts = s.split(':');
    let (Some(h), Some(m), Some(sec), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let h: u64 = h.trim().parse().map_err(|_| bad())?;
    let m: u64 = m.trim().parse().map_err(|_| bad())?;
    let sec: u64 = sec.trim().parse().map_err(|_| bad())?;
    if !(0..60).contains(&m) || !(0..60).contains(&sec) {
        return Err(bad());
    }
    Ok(Duration::from_secs(h * 3600 + m * 60 + sec))
}

/// The parsed fields of one CSV record that participate in the time calculations. Named apart
/// from [`Session`], which is the finished, UTC-resolved article this module hands to the peak
/// power contribution logic.
struct CsvSession {
    id: String,
    start_local: civil::DateTime,
    end_local: civil::DateTime,
    conn_duration: Duration,
    active_charge_time: Duration,
    /// Kept for its parse: a non-numeric `Energy_Use` invalidates the row, and that is caught in
    /// [`CsvSession::parse`]. The value itself is consumed on the reading side.
    #[allow(dead_code)]
    energy_use: f64,
}

/// One output row. A session normally yields one; an unresolvable DST fold yields two.
///
/// Carries a whole [`Session`] rather than loose timestamps, so that every derived column is
/// computed by the same methods the estimating logic uses. When they were separate fields the
/// write path had its own `adj_conn_end`, and it was wrong.
///
/// `record` remains, because the CSV pass-through columns are not part of a `Session` and never
/// should be: a `Session` is what the arithmetic needs, not a copy of the source row.
struct Row {
    /// Index into the original `records`, for the pass-through columns.
    record: usize,
    session: Session,
    /// The two reported wall times, kept as written. `Session` holds instants, and the local
    /// columns must show what the report said rather than a re-derivation of it — those differ in
    /// the DST gap, where the reported wall time never occurred.
    start_local: civil::DateTime,
    end_local: civil::DateTime,
}

impl Row {
    fn adj_start_local(&self) -> civil::DateTime {
        local_datetime(self.session.adj_conn_start())
    }

    fn adj_end_local(&self) -> civil::DateTime {
        local_datetime(self.session.adj_conn_end())
    }
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

    /// Resolves this session's local timestamps to UTC and derives `adj_conn_end`.
    ///
    /// Returns one row normally, or two when the start falls in the DST fold and the reported end
    /// cannot tell the two offsets apart — see README.md, "Time zone", for why duplication is the
    /// policy and why the copies get distinct ids.
    ///
    /// # Not the same problem as [`crate::sessions::map_local`]
    ///
    /// Both resolve an ambiguous local time, and the two must **not** be merged. They are asked
    /// different questions and are right to answer differently:
    ///
    /// - `map_local` is asked *"what could this wall time mean?"* by a user picking an interval of
    ///   interest. It has nothing but the wall time, so it returns every candidate and makes the
    ///   caller choose, or say `EST`/`EDT`.
    /// - This is asked *"which offset was this session actually at?"* and has evidence the other
    ///   lacks: `Conn_Duration`, which is untruncated elapsed time. Testing each candidate against
    ///   `start + Conn_Duration` usually settles it, and duplication is the fallback for when it
    ///   does not.
    ///
    /// Their tie-breaks differ for the same reason. Giving this one `map_local`'s behaviour would
    /// throw away the duration evidence; giving `map_local` this one's would have it invent
    /// evidence it does not have.
    fn resolve(&self, tz: &TimeZone, row: usize) -> Result<Vec<Row>, Box<dyn Error>> {
        // Kinds known before the DST branch runs. They describe the record itself, so on
        // duplication both copies inherit them.
        let mut common = Vec::new();

        // avg_kw is a division by Active_Charge_Time. The sheet shows it as #DIV/0!; it is
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
                let mut anomalies = common.clone();
                if !duration_is_consistent(start_utc, end_utc, self.conn_duration) {
                    anomalies.push(AnomalyKind::InconsistentDuration);
                }

                Ok(Row {
                    record: row - 1,
                    session: Session {
                        id: match suffix {
                            Some(s) => format!("{}-{s}", self.id),
                            None => self.id.clone(),
                        },
                        // The workbook row, which is the CSV row plus the header. A duplicated
                        // record occupies two workbook rows, so this diverges from `record` from
                        // the second copy on; `write_workbook` restamps it as it writes.
                        row,
                        conn_start: start_utc,
                        conn_end: end_utc,
                        conn_duration: self.conn_duration,
                        charge_time: self.active_charge_time,
                        energy_use: self.energy_use,
                        anomalies,
                    },
                    start_local: self.start_local,
                    end_local: self.end_local,
                })
            })
            .collect()
    }

    /// Does `start` plus the reported elapsed duration land back on the reported end?
    ///
    /// The same window [`duration_is_consistent`] applies, expressed as an offset — see
    /// [`SLACK_EARLY`] and [`SLACK_LATE`]. Requiring equal minutes instead rejects roughly half of
    /// all consistent records — 116 of the 238 rows in this project's `data` directory.
    ///
    /// The comparison is made on *local wall time*, not on instants. That is what lets both fold
    /// candidates match a session short enough to fit inside the repeated hour, which is the very
    /// ambiguity this test exists to detect. The window cannot blur the two candidates together
    /// otherwise: they lie a full hour apart.
    fn reproduces_reported_end(&self, tz: &TimeZone, start: Timestamp) -> bool {
        let end = (start + self.conn_duration).to_zoned(tz.clone()).datetime();
        let offset = wall_clock_instant(end).duration_since(wall_clock_instant(self.end_local));
        SLACK_EARLY < offset && offset < SLACK_LATE
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

    let adj_start_utc_col = column_letters(column_index(Source::AdjConnStartUtc));
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
             adj_conn_end), which contains it wherever in that minute it fell. Because the end is \
             excluded, a session starting at this exact time does NOT overlap this one. See \
             README.md, \"Excel workbook\".",
        ),
        (
            Source::AdjConnDuration,
            "adj_conn_end_utc - conn_start_utc. Computed from the UTC columns so it is true \
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

/// The sessions in a workbook produced by [`session_csv_to_xlsx`], grouped by how the peak power
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
    /// swamp or poison any segment they entered.
    ///
    /// Surfaced rather than dropped because such a row is almost certainly a **reporting fault**
    /// and someone should see it. That is a correction: the reason given here used to be that
    /// energy delivered in no time at all is what a demand charge bills on, which read the field
    /// as a real measurement of charging. Evolute has since stated that the three duration fields
    /// track the same thing to within about a second and are not measured separately, so a zero
    /// beside a non-zero `Energy_Use` is a contradiction in the report rather than an event. See
    /// `Questions_for_Evolute.md`, "Answers received". [`Session::avg_kw`] substitutes a finite
    /// figure so the row can still be listed. See README.md, "Other".
    pub spikes: Vec<Session>,
    /// Sessions flagged [`AnomalyKind::InconsistentDuration`]: their reported start, end and duration
    /// contradict each other, so they cannot be placed on a timeline at all. Excluded from the
    /// estimates and returned only for review. See README.md, "Other".
    pub excluded: Vec<Session>,
    /// Where the run log was written. It always exists, and says either that nothing was found or
    /// which stored columns disagreed with the recomputed values.
    pub log_path: PathBuf,
}

/// Sheet columns that make a workbook a session report. The reading-side counterpart of
/// [`REQUIRED_HEADERS`].
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
/// `avg_kw` is recomputed here rather than read from the sheet's `avg_kw` column, which
/// holds a formula whose cached value this crate never writes. For a spike that leaves it infinite
/// or `NaN`, which is the honest reading; the estimating logic substitutes a finite value.
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
        let charge_time = duration_of_serial(number(sheet, &headers, "Active_Charge_Time", row)?);
        let anomalies = anomaly_kinds(sheet, &headers, row, path)?;
        let session = Session {
            id,
            row: row as usize,
            conn_start: instant_of_serial(number(sheet, &headers, "conn_start_utc", row)?)?,
            conn_end: instant_of_serial(number(sheet, &headers, "conn_end_utc", row)?)?,
            conn_duration: duration_of_serial(number(sheet, &headers, "Conn_Duration", row)?),
            charge_time,
            energy_use,
            anomalies,
        };

        check_stored_columns(sheet, &headers, row, &session, &mut log, &mut unevaluated);

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

    for (name, count) in &unevaluated {
        log.note(format!(
            "{name}: {count} row(s) hold a formula with no stored value, so there was nothing to \
             check. Normal for a workbook this software wrote — it writes formulas and Excel \
             computes them, so values appear only once Excel has opened and saved the file. The \
             recomputed figures are used throughout either way."
        ));
    }
    let log_path = log.write_beside(path, "read", "Read back from workbook")?;

    Ok(SessionReport {
        sessions,
        spikes,
        excluded,
        log_path,
    })
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
    use std::fs;

    /// A row's anomalies with [`AnomalyKind::ExcessiveAvgKw`] removed.
    ///
    /// Nearly every test here is about *timestamps* — DST resolution, the `adj_conn_end` padding,
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

    /// The same filter applied to an `anomalies` cell, read back through the wire format.
    ///
    /// Going through [`AnomalyKind::from_token`] rather than comparing the cell text also checks
    /// that what was written is what can be read back, which is the property the column exists for.
    fn timing_anomalies_in_cell(cell: &str) -> Vec<AnomalyKind> {
        let kinds: Vec<AnomalyKind> = cell
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| AnomalyKind::from_token(t).unwrap_or_else(|| panic!("unreadable token {t:?}")))
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
    fn durations_parse_including_over_24_hours() {
        assert_eq!(
            parse_duration("5:07:53", 1, "d").unwrap(),
            Duration::from_secs(5 * 3600 + 7 * 60 + 53)
        );
        assert_eq!(
            parse_duration("30:00:00", 1, "d").unwrap(),
            Duration::from_secs(30 * 3600)
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
    fn column_letters_span_past_z() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(COLUMNS.len()), "AD");
    }

    /// `adj_conn_end` is the reported end padded past the end of its minute — the exclusive end of
    /// the window the true end lies in, so `21:29` pads to `21:30:00` and not `21:29:59`. Both rows
    /// are real sample rows, and they straddle the case the old `min(...)` rule treated specially:
    /// the second has `start + duration` (23:40:29) *before* the reported end.
    #[test]
    fn adj_conn_end_pads_the_reported_end() {
        let rows = session("2026-06-01 16:22", "2026-06-01 21:29", "5:07:53")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(
            local_of(rows[0].session.adj_conn_end()),
            civil::date(2026, 6, 1).at(21, 30, 0, 0)
        );
        assert!(timing_anomalies(&rows[0].session.anomalies).is_empty());

        let rows = session("2026-06-07 16:42", "2026-06-07 23:41", "6:58:29")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(
            local_of(rows[0].session.adj_conn_end()),
            civil::date(2026, 6, 7).at(23, 42, 0, 0)
        );
        assert!(timing_anomalies(&rows[0].session.anomalies).is_empty());
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
                row.session.adj_conn_end() >= row.session.conn_end,
                "{start}: adjusted end precedes reported end"
            );
            assert!(
                row.session
                    .adj_conn_end()
                    .duration_since(row.session.conn_start)
                    .unsigned_abs()
                    >= s.conn_duration,
                "{start}: adjusted duration shorter than Conn_Duration"
            );
            assert!(
                timing_anomalies(&row.session.anomalies).is_empty(),
                "{start}: unexpected {:?}",
                row.session.anomalies
            );
        }
    }

    #[test]
    fn utc_conversion_uses_edt_in_june() {
        let rows = session("2026-06-01 16:22", "2026-06-01 21:29", "5:07:53")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(
            rows[0]
                .session
                .conn_start
                .to_zoned(TimeZone::UTC)
                .datetime(),
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
            rows[0]
                .session
                .conn_start
                .to_zoned(TimeZone::UTC)
                .datetime(),
            civil::date(2026, 11, 1).at(5, 30, 0, 0), // EDT is UTC-4
        );
        assert!(timing_anomalies(&rows[0].session.anomalies).is_empty());
    }

    /// The mirror of the test above, and the case a one-sided `start + duration <= adj_conn_end`
    /// test would get wrong: here EST is correct, and the EDT candidate lands a full hour *early*.
    /// Only a two-sided comparison rejects it; accepting it would duplicate a session that is not
    /// ambiguous at all, double-counting its power.
    #[test]
    fn dst_fold_resolved_to_est_rejects_the_hour_early_candidate() {
        // 01:30 EST + 3h elapsed = 04:30 EST. Starting at 01:30 EDT would end at 03:30.
        let rows = session("2026-11-01 01:30", "2026-11-01 04:30", "3:00:00")
            .resolve(&time_zone(), 1)
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "should not duplicate: {:?}",
            rows[0].session.id
        );
        assert_eq!(
            rows[0]
                .session
                .conn_start
                .to_zoned(TimeZone::UTC)
                .datetime(),
            civil::date(2026, 11, 1).at(6, 30, 0, 0), // EST is UTC-5
        );
        assert!(timing_anomalies(&rows[0].session.anomalies).is_empty());
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
            !rows[0]
                .session
                .anomalies
                .contains(&AnomalyKind::DstUnresolvable),
            "spurious DstUnresolvable: {:?}",
            rows[0].session.anomalies
        );
        assert_eq!(
            rows[0]
                .session
                .conn_start
                .to_zoned(TimeZone::UTC)
                .datetime(),
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
        assert_eq!(rows[0].session.id, "S1-EDT");
        assert_eq!(rows[1].session.id, "S1-EST");
        // The copies are an hour apart in real time, which is the whole point.
        assert_eq!(
            rows[1]
                .session
                .conn_start
                .duration_since(rows[0].session.conn_start),
            SignedDuration::from_hours(1)
        );
        // Both copies carry the flag, so each workbook row says why it is there.
        for row in &rows {
            assert_eq!(
                timing_anomalies(&row.session.anomalies),
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
        assert_eq!(local_of(rows[0].session.conn_start), dt("2026-03-08 03:30"));
        assert_eq!(
            timing_anomalies(&rows[0].session.anomalies),
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
        let elapsed = row
            .session
            .adj_conn_end()
            .duration_since(row.session.conn_start);
        assert!(
            elapsed >= SignedDuration::from_hours(3),
            "elapsed {elapsed:?} lost the repeated hour"
        );
        // The same subtraction done on local wall times loses the repeated hour.
        let wall_secs = serial_of_civil(row.adj_end_local()) - serial_of_civil(row.start_local);
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
                timing_anomalies(&rows[0].session.anomalies),
                vec![AnomalyKind::ZeroActiveChargeTime],
                "energy {energy}"
            );
        }
    }

    /// The three checks of [`duration_is_consistent`], each pinned at the boundary it draws.
    ///
    /// Both bounds are exclusive and both are pinned to the second, because getting either off by
    /// one silently reclassifies real records — the sample data reaches to within 3 seconds of the
    /// early edge. With the reported times at 10:00 and 10:30 the sound durations are exactly
    /// `[0:29:01, 0:31:00]`.
    ///
    /// The window is asymmetric: one second wider late than early. That second is not slack, it is
    /// the reporting's uncertainty about whether the last second of the end minute is included.
    /// See `docs/sessions/time-reporting-uncertainty.md`.
    #[test]
    fn inconsistent_duration_is_reported() {
        let kinds = |start, end, conn| {
            let all = session(start, end, conn)
                .resolve(&time_zone(), 1)
                .unwrap()
                .swap_remove(0)
                .session
                .anomalies;
            timing_anomalies(&all)
        };
        let bad = vec![AnomalyKind::InconsistentDuration];

        // Overshoot: 10:00 + 2h = 12:00, well past the 10:31:01 upper bound.
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 10:30", "2:00:00"),
            bad
        );

        // Check 1, doing work no other check does. A one-minute inversion with a zero duration
        // satisfies both of the others -- 10:01 + 0 = 10:01 is under the 10:01:01 upper bound and
        // over the 09:59:00 lower one -- so nothing but the start-before-end test rejects it. It
        // is also the smallest inversion the reporting can express, since both reported times are
        // whole minutes; that in turn forces the duration to zero, hence the extra anomaly here.
        // Letting this row through panics `Session::intersects` downstream.
        assert_eq!(
            kinds("2026-06-01 10:01", "2026-06-01 10:00", "0:00:00"),
            vec![
                AnomalyKind::ZeroActiveChargeTime,
                AnomalyKind::InconsistentDuration
            ]
        );
        // The same fault at a scale the overshoot check would also have caught.
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 09:00", "0:10:00"),
            bad
        );

        // One second outside each bound. 10:31:01 is the first instant check 2 rejects, 10:29:00
        // the last one check 3 does.
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 10:30", "0:31:01"),
            bad
        );
        assert_eq!(
            kinds("2026-06-01 10:00", "2026-06-01 10:30", "0:29:00"),
            bad
        );

        // Exactly on each bound, and both sound. `0:31:00` is the case the old predicate rejected
        // and the document accepts.
        assert!(kinds("2026-06-01 10:00", "2026-06-01 10:30", "0:31:00").is_empty());
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

        let log = fs::read_to_string(&report.log_path).unwrap();
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
