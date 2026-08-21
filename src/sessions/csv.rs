//! Reading a session report CSV, as Evolute exports it.
//!
//! This is where the session report becomes [`Session`]s: the CSV is parsed, each record's local
//! wall times are resolved to UTC, the DST fold is settled or the record duplicated, and every
//! judgement call is recorded as an [`AnomalyKind`]. Nothing here knows about workbooks.
//!
//! Two ways out, sharing all of that:
//!
//! - [`session_list`] buckets the sessions for the peak power contribution logic, straight from
//!   the CSV. No workbook is involved.
//! - `session_rows` hands the rows over in report order, with the pass-through CSV fields still
//!   reachable, which is what [`super::excel::session_csv_to_xlsx`] needs to render them.
//!
//! Bucketing is lossy — it sorts sessions into three vectors and drops report order, the link back
//! to the CSV record, and the reported wall times — so the writer cannot be built on
//! [`session_list`]. Both sit on `session_rows` instead.
//!
//! The module is named for what it reads, so the `csv` crate is written `::csv` throughout to keep
//! the two apart.

use crate::time::{is_on_grid, local_datetime, time_zone, wall_clock_instant};

use super::TIME_GRID_STEP;
use super::{
    Anomaly, AnomalyKind, BREAKER_RATING_KW, RSession, RunLog, Session, SessionReport,
    duration_is_consistent,
};
use jiff::{
    SignedDuration, Timestamp, civil,
    tz::{AmbiguousOffset, TimeZone},
};
use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

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

/// CSV columns that must be present for the file to mean anything.
const REQUIRED_HEADERS: &[&str] = &[
    "Charge_Session_ID",
    "Conn_DateTime_Start",
    "Conn_DateTime_End",
    "Conn_Duration",
    "Active_Charge_Time",
    "Energy_Use",
];

/// Reads the session report CSV at `path` and returns the charging sessions it describes, ready
/// for the peak power contribution logic.
///
/// The counterpart of [`super::excel::session_list`], and the way to reach the sessions without a
/// workbook in between. The two agree on every figure — the workbook writer and this function are
/// the same parse — but they differ in what they can tell you afterwards. A workbook has stored
/// derived columns that may have been edited, so reading one compares them against the recomputed
/// values and logs any disagreement. A CSV has nothing to compare against: it is the source.
///
/// The domain rules — the UTC conversion and its DST policy, the definitions of `adj_conn_end` and
/// `adj_conn_duration`, and the treatment of zero-`Energy_Use` sessions — are specified in
/// `docs/time/README.md` under "Time zone" and in `docs/sessions/README.md` under "Other".
///
/// Sessions are sorted into the three buckets of [`SessionReport`] by
/// `SessionReport::from_session_lists`,
/// which carries the rules. Every session in the report reaches one of them; none is dropped.
///
/// A `<stem>.csv.read.log` is written beside the input, holding the anomalies found and the
/// off-grid warning if it applies — the same content [`super::excel::session_csv_to_xlsx`] writes
/// to `<stem>.convert.log`, because the two run the same parse.
///
/// # Errors
///
/// Returns `Err` only for conditions that invalidate the whole file: it cannot be read, a required
/// header from the private `REQUIRED_HEADERS` is missing, or a timestamp or duration does not
/// parse. Per-row judgement calls do not abort the read; they are carried on each
/// [`Session::anomalies`] and summarised in the log.
pub fn session_list(path: &Path) -> Result<SessionReport, Box<dyn Error>> {
    // Every error out of this function names the file it concerns, and does so in one place rather
    // than at each site that raises one. Some underlying errors carry the path and some do not --
    // the `csv` crate's do not, a per-row parse failure knows only its row -- so a caller could
    // rely on neither. It can now: the path is here, once, and a caller holding it should not add
    // it again.
    read_session_list(path).map_err(|e| format!("{}: {e}", path.display()).into())
}

fn read_session_list(path: &Path) -> Result<SessionReport, Box<dyn Error>> {
    let rows = session_rows(path)?;
    let log_path = rows
        .log
        .write_beside(path, "csv.read", "Read from session report")?;
    // One list, so there is nothing to collapse across files -- but it still goes through the
    // merge, because that is where a shared `Charge_Session_ID` is noticed, and one file can carry
    // one as readily as two can.
    let sessions: Vec<RSession> = rows.rows.into_iter().map(|row| row.session).collect();
    Ok(SessionReport::from_session_lists(
        vec![sessions],
        vec![log_path],
    ))
}

/// Every row one session report CSV yields, in the order the report states them.
///
/// The unbucketed form of [`session_list`], for a caller that has to render the report rather than
/// estimate from it. It keeps what bucketing discards: report order, the pass-through CSV fields,
/// and the reported wall times, which differ from a re-derivation inside the DST gap.
///
/// The `records` are held rather than copied out because the pass-through columns are not part of
/// a [`Session`] and should not become part of one — a `Session` is what the arithmetic needs, not
/// a copy of the source row. Reach them through [`SessionRows::field`] and
/// [`SessionRows::duration`], which keep the header lookup and the parsing on this side of the
/// boundary.
pub(crate) struct SessionRows {
    headers: Headers,
    records: Vec<::csv::StringRecord>,
    /// One per output row, in report order. A record duplicated to resolve a DST fold yields two.
    pub(crate) rows: Vec<Row>,
    /// Every judgement call made, numbered by output row rather than by CSV record.
    pub(crate) anomalies: Vec<Anomaly>,
    /// Unwritten. The caller writes it beside its own output file, under its own suffix.
    pub(crate) log: RunLog,
}

impl SessionRows {
    /// The named CSV column for `row`, trimmed. Empty when the column is absent or blank.
    pub(crate) fn field(&self, row: &Row, name: &str) -> &str {
        field(&self.headers, &self.records[row.record], name)
    }

    /// The named CSV column for `row` parsed as an elapsed time, or `None` when the cell is blank.
    pub(crate) fn duration(
        &self,
        row: &Row,
        name: &str,
    ) -> Result<Option<Duration>, Box<dyn Error>> {
        let raw = self.field(row, name);
        if raw.is_empty() {
            return Ok(None);
        }
        parse_duration(raw, row.record + 2, name).map(Some)
    }
}

/// Parses `path` and resolves every record, without writing anything.
///
/// Shared by [`session_list`] and [`super::excel::session_csv_to_xlsx`], which is what makes the
/// two agree by construction rather than by inspection.
pub(crate) fn session_rows(path: &Path) -> Result<SessionRows, Box<dyn Error>> {
    let tz = time_zone();
    let (headers, records) = read_csv(path)?;
    // One allocation for the file, shared by every session read from it.
    let source = Rc::new(path.to_path_buf());

    let mut anomalies = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    for (i, record) in records.iter().enumerate() {
        // The CSV row, counting the header, and the number every session parsed from this record
        // carries. A record duplicated to resolve a DST fold yields two sessions and they share it,
        // because they share the row they came from — see `Session::row`. The workbook row is a
        // different number, and it belongs to the workbook: `super::excel` derives it from a
        // session's position in `rows` when it writes one.
        let csv_row = i + 2;
        let session = CsvSession::parse(&headers, record, csv_row)?;
        for row in session.resolve(&tz, &source, csv_row)? {
            anomalies.extend(row.session.anomalies.iter().map(|&kind| Anomaly {
                session: row.session.clone(),
                kind,
            }));
            rows.push(row);
        }
    }

    // Anomalies only: there is nothing to compare against on this side, since this is what
    // produces the values in the first place. See `sessions::log` for why discrepancies are a
    // separate channel.
    let mut log = RunLog::new();
    note_off_grid_rows(&rows, &mut log);
    for anomaly in &anomalies {
        log.note(anomaly.to_string());
    }

    Ok(SessionRows {
        headers,
        records,
        rows,
        anomalies,
        log,
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

fn read_csv(path: &Path) -> Result<(Headers, Vec<::csv::StringRecord>), Box<dyn Error>> {
    let mut reader = ::csv::Reader::from_path(path)?;
    let headers: Headers = reader
        .headers()?
        .iter()
        .enumerate()
        .map(|(i, h)| (h.trim().to_owned(), i))
        .collect();

    for required in REQUIRED_HEADERS {
        if !headers.contains_key(*required) {
            return Err(format!("missing required column `{required}`").into());
        }
    }

    let records = reader.records().collect::<Result<Vec<_>, _>>()?;
    Ok((headers, records))
}

fn field<'a>(headers: &Headers, record: &'a ::csv::StringRecord, name: &str) -> &'a str {
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
/// The pass-through CSV columns are not part of a `Session` and never should be, so the row keeps
/// an index back into the records instead — see [`SessionRows::field`].
pub(crate) struct Row {
    /// Index into [`SessionRows::records`], for the pass-through columns.
    record: usize,
    pub(crate) session: RSession,
    /// The two reported wall times, kept as written. `Session` holds instants, and the local
    /// columns must show what the report said rather than a re-derivation of it — those differ in
    /// the DST gap, where the reported wall time never occurred.
    pub(crate) start_local: civil::DateTime,
    pub(crate) end_local: civil::DateTime,
}

impl Row {
    pub(crate) fn adj_start_local(&self) -> civil::DateTime {
        local_datetime(self.session.adj_conn_start())
    }

    pub(crate) fn adj_end_local(&self) -> civil::DateTime {
        local_datetime(self.session.adj_conn_end())
    }
}

impl CsvSession {
    fn parse(
        headers: &Headers,
        record: &::csv::StringRecord,
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
    /// cannot tell the two offsets apart — see docs/time/README.md, "Time zone", for why duplication is the
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
    fn resolve(
        &self,
        tz: &TimeZone,
        source: &Rc<PathBuf>,
        row: usize,
    ) -> Result<Vec<Row>, Box<dyn Error>> {
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
                    record: row - 2,
                    session: Rc::new(Session {
                        path: source.clone(),
                        // The CSV row this record occupies. Both halves of a duplicated fold carry
                        // it, since both were read from that one row; the `-EDT`/`-EST` suffix on
                        // the id is what tells them apart.
                        row,
                        id: match suffix {
                            Some(s) => format!("{}-{s}", self.id),
                            None => self.id.clone(),
                        },
                        conn_start: start_utc,
                        conn_end: end_utc,
                        conn_duration: self.conn_duration,
                        charge_time: self.active_charge_time,
                        energy_use: self.energy_use,
                        anomalies,
                    }),
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

#[cfg(test)]
// cargo test --lib -- sessions::csv::test --nocapture
mod test {
    use super::*;
    use crate::sessions::test_support::timing_anomalies;
    use crate::time::serial_of_civil;
    use std::fs;
    use std::path::PathBuf;

    fn dt(s: &str) -> civil::DateTime {
        civil::DateTime::strptime("%Y-%m-%d %H:%M", s).unwrap()
    }

    /// A stand-in source file for tests that call [`CsvSession::resolve`] directly. Nothing here
    /// reads it; it is only what the sessions record as where they came from.
    fn test_source() -> Rc<PathBuf> {
        Rc::new(PathBuf::from("Session_Report_Test.csv"))
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

    /// A scratch directory of its own per test, since these run in parallel within one process.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ev_peak_csv_{}_{tag}", std::process::id()));
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

    /// `adj_conn_end` is the reported end padded past the end of its minute — the exclusive end of
    /// the window the true end lies in, so `21:29` pads to `21:30:00` and not `21:29:59`. Both rows
    /// are real sample rows, and they straddle the case the old `min(...)` rule treated specially:
    /// the second has `start + duration` (23:40:29) *before* the reported end.
    #[test]
    fn adj_conn_end_pads_the_reported_end() {
        let rows = session("2026-06-01 16:22", "2026-06-01 21:29", "5:07:53")
            .resolve(&time_zone(), &test_source(), 2)
            .unwrap();
        assert_eq!(
            local_of(rows[0].session.adj_conn_end()),
            civil::date(2026, 6, 1).at(21, 30, 0, 0)
        );
        assert!(timing_anomalies(&rows[0].session.anomalies).is_empty());

        let rows = session("2026-06-07 16:42", "2026-06-07 23:41", "6:58:29")
            .resolve(&time_zone(), &test_source(), 2)
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
            let rows = s.resolve(&time_zone(), &test_source(), 2).unwrap();
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
            .resolve(&time_zone(), &test_source(), 2)
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
            .resolve(&time_zone(), &test_source(), 2)
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
            .resolve(&time_zone(), &test_source(), 2)
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
            .resolve(&time_zone(), &test_source(), 2)
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
            .resolve(&time_zone(), &test_source(), 2)
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
        // Both copies carry the flag, so each output row says why it is there.
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
            .resolve(&time_zone(), &test_source(), 2)
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
            .resolve(&time_zone(), &test_source(), 2)
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

    /// Zero `Active_Charge_Time` is flagged whatever the energy: the `avg_kw` cell shows `#DIV/0!`
    /// either way, and the session becomes a spike either way.
    #[test]
    fn zero_active_charge_time_is_reported() {
        for energy in [5.0, 0.0] {
            let mut s = session("2026-06-01 10:00", "2026-06-01 10:00", "0:00:00");
            s.energy_use = energy;
            let rows = s.resolve(&time_zone(), &test_source(), 7).unwrap();
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
                .resolve(&time_zone(), &test_source(), 2)
                .unwrap()
                .swap_remove(0)
                .session
                .anomalies
                .clone();
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

    /// The sessions reach the peak power contribution logic straight from the CSV, bucketed the
    /// same way [`super::super::excel::session_list`] buckets them out of a workbook — one row per
    /// bucket here, so all three rules are exercised.
    #[test]
    fn session_list_buckets_straight_from_the_csv() {
        const CSV: &str = "\
Charge_Session_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Active_Charge_Time,Energy_Use
S1,2026-06-01 16:22,2026-06-01 21:29,5:07:53,5:07:52,30.6
S2,2026-06-02 10:00,2026-06-02 09:00,0:10:00,0:09:00,1.5
S3,2026-06-03 09:00,2026-06-03 09:00,0:00:00,0:00:00,4.2
";
        let dir = temp_dir("session_list");
        let csv_path = dir.join("Session_Report_Test.csv");
        fs::write(&csv_path, CSV).unwrap();

        let report = session_list(&csv_path).unwrap();

        assert_eq!(report.sessions.len(), 1);
        assert_eq!(report.sessions[0].id, "S1");
        assert_eq!(report.excluded.len(), 1);
        assert_eq!(report.excluded[0].id, "S2");
        assert_eq!(report.spikes.len(), 1);
        assert_eq!(report.spikes[0].id, "S3");

        // Beside the CSV, under its own suffix, so it cannot collide with the workbook's logs.
        assert_eq!(
            report.log_paths[0],
            dir.join("Session_Report_Test.csv.read.log")
        );
        let log = fs::read_to_string(&report.log_paths[0]).unwrap();
        assert!(
            log.contains("InconsistentDuration") || log.contains("contradict"),
            "{log}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// A missing required column invalidates the whole file rather than one row: the sessions it
    /// would describe cannot be trusted.
    #[test]
    fn a_missing_required_column_is_rejected() {
        const CSV: &str = "\
Charge_Session_ID,Conn_DateTime_Start,Conn_DateTime_End,Conn_Duration,Active_Charge_Time
S1,2026-06-01 16:22,2026-06-01 21:29,5:07:53,5:07:52
";
        let dir = temp_dir("missing_column");
        let csv_path = dir.join("Session_Report_Test.csv");
        fs::write(&csv_path, CSV).unwrap();

        let err = session_list(&csv_path).unwrap_err().to_string();
        assert!(err.contains("Energy_Use"), "{err}");

        fs::remove_dir_all(&dir).ok();
    }
}
