//! Renders an [`IntervalEstimates`] as markdown that also reads as plain text.
//!
//! Both at once is the whole constraint, and it drives every choice here. Not every reader has a
//! markdown renderer, so the output has to survive being read raw:
//!
//! - **Setext headings** (`====`, `----`) rather than `#`, so a heading looks underlined instead of
//!   prefixed with punctuation. That allows two heading levels, which is why the sub-labels in the
//!   estimates section are sentences rather than a third level.
//! - **Every table cell padded** to its column's width, numerics right-aligned. A renderer ignores
//!   the padding; a plain reader depends on it entirely.
//! - **No four-space indentation anywhere**, since markdown would turn it into a code block. Wrapped
//!   list items indent by two.
//! - **No emphasis markers.** Labels are quoted — `"Energy-based"` — which reads identically either
//!   way.
//! - **Session ids live in their own section**, not in a table cell, because a markdown table row is
//!   a single line and a segment holding twelve sessions cannot be wrapped inside one.
//!
//! This is the crate's single rendering module. [`site_load_report`] lives here too, for the same
//! reason [`fmt::Display`] delegates to [`IntervalEstimates::to_markdown`]: one rendering rather
//! than two that could drift.

use crate::{
    Anomaly, AnomalyKind, Bracket, Interval, IntervalEstimates, Segment, Session,
    site_load::{
        BREAKER_COUNT, BREAKER_RATING_A, CONTINUOUS_DUTY_DERATE, PANEL_VOLTAGE_V, XFMR_RATING_KVA,
        ev_load, ev_pilot_current_a, loading_ratio, site_load,
    },
    time_zone,
};
use jiff::{Timestamp, Zoned};
use std::{collections::HashMap, fmt};

/// Width the prose is wrapped to. Comfortably inside 80 columns, leaving room for a quoting prefix
/// in an email reply.
const WRAP: usize = 76;

/// Column alignment for [`table`].
#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Right,
}
use Align::{Left, Right};

/// A markdown pipe table, every cell padded to its column width so it also lines up in monospace.
fn table(headers: &[&str], rows: &[Vec<String>], align: &[Align]) -> String {
    let widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain(std::iter::once(h.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let line = |cells: &[String]| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .zip(align)
            .map(|((c, w), a)| {
                let pad = w.saturating_sub(c.chars().count());
                match a {
                    Left => format!("{c}{}", " ".repeat(pad)),
                    Right => format!("{}{c}", " ".repeat(pad)),
                }
            })
            .collect();
        format!("| {} |", padded.join(" | "))
    };

    // The alignment row carries the `:` markers so a renderer right-aligns the numbers too.
    let rule: Vec<String> = widths
        .iter()
        .zip(align)
        .map(|(w, a)| match a {
            Left => format!(":{}", "-".repeat(w + 1)),
            Right => format!("{}:", "-".repeat(w + 1)),
        })
        .collect();

    let header_cells: Vec<String> = headers.iter().map(|h| (*h).to_owned()).collect();
    let mut out = vec![line(&header_cells), format!("|{}|", rule.join("|"))];
    out.extend(rows.iter().map(|r| line(r)));
    out.join("\n")
}

/// Wraps `text` to [`WRAP`] columns on word boundaries, with `indent` prefixing every line after
/// the first. Long words are never broken, so an identifier stays intact even if it overruns.
fn wrap(text: &str, indent: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() {
            word.len()
        } else {
            cur.chars().count() + 1 + word.chars().count()
        };
        if !cur.is_empty() && candidate > WRAP {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(indent);
        }
        if !cur.is_empty() && !cur.ends_with(' ') && cur != *indent {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines.join("\n")
}

fn h1(s: &str) -> String {
    format!("{s}\n{}", "=".repeat(s.chars().count()))
}

fn h2(s: &str) -> String {
    format!("{s}\n{}", "-".repeat(s.chars().count()))
}

fn local(ts: Timestamp) -> Zoned {
    Zoned::new(ts, time_zone())
}

/// A segment's name: its start as a local clock time.
///
/// To the minute and no finer, and undated. Segments sit on the time grid, so their seconds
/// would be three zeroes in every row; and they all fall inside one interval of interest, whose
/// date the header states once. This is the name the Estimates table's `Segment` column and the
/// membership list both use, so the three sections join on it.
fn hm(ts: Timestamp) -> String {
    local(ts).strftime("%H:%M").to_string()
}

/// Dated, and to the minute. The excluded list covers the whole workbook, so its dates cannot be
/// left implicit the way a segment's can.
fn ymd_hm(ts: Timestamp) -> String {
    local(ts).strftime("%Y-%m-%d %H:%M").to_string()
}

/// The far end of a span whose near end is already printed: the date only when it differs.
///
/// The same convention the header's interval line follows, and it is what keeps the excluded-
/// sessions table inside the width the report has to read at. Nearly every session begins and ends
/// on one day, so repeating the date would spend sixteen columns to say what the previous cell
/// already said; a session that does cross midnight still says so.
fn ymd_hm_to(from: Timestamp, to: Timestamp) -> String {
    let (from, to) = (local(from), local(to));
    match from.date() == to.date() {
        true => to.strftime("%H:%M").to_string(),
        false => to.strftime("%Y-%m-%d %H:%M").to_string(),
    }
}

/// A bracket as one cell, `min-max`. Three decimals, matching every other figure in the report.
///
/// Always both ends, never a midpoint: the two numbers are what the reported times actually
/// support, and collapsing them would state a precision the minute-resolution source does not have.
/// An exact bracket still prints both, so a column of them stays a column of the same shape.
fn bracket_cell(b: Bracket<f64>) -> String {
    format!("{:.3}-{:.3}", b.min, b.max)
}

/// Whether an excluded session's reported span appears to meet the interval of interest, as a
/// report cell.
///
/// [`Session::lenient_intersects`] rather than [`Session::intersects`], and only here: an excluded
/// session may report an end before its start, which is a span the strict test treats as a broken
/// precondition and refuses. This listing exists to show exactly those records, so it answers for
/// them — as an appearance, which is all a contradictory record supports.
fn in_interval(session: &Session, ioi: &Interval) -> String {
    match session.lenient_intersects(ioi) {
        true => "yes".to_owned(),
        false => "no".to_owned(),
    }
}

/// An anomaly's cell: the bare kind, except where the kind is about a figure, in which case the
/// figure is written into it.
///
/// The value lives here rather than on [`AnomalyKind`], which stays a plain classification. That
/// keeps the workbook's `anomalies` column a list of bare variant names that
/// [`AnomalyKind::from_token`] can read back, and keeps the glossary below the table explaining
/// each kind once rather than once per session.
fn anomaly_cell(kind: AnomalyKind, avg_kw: Option<f64>) -> String {
    match (kind, avg_kw) {
        (AnomalyKind::ExcessiveAvgKw, Some(kw)) => format!("{}({kw:.3})", kind.as_str()),
        _ => kind.as_str().to_owned(),
    }
}

/// One glossary entry per kind present, in first-appearance order.
///
/// The prose comes from each kind's [`fmt::Display`], so there is one wording to maintain rather
/// than a second copy here that could drift from it.
fn glossary(kinds: impl IntoIterator<Item = AnomalyKind>, out: &mut Vec<String>) {
    let mut seen: Vec<AnomalyKind> = Vec::new();
    for kind in kinds {
        if !seen.contains(&kind) {
            seen.push(kind);
            out.push(wrap(&format!("- {} - {}.", kind.as_str(), kind), "  "));
        }
    }
}

const ESTIMATE_HEADERS: [&str; 5] = ["Estimate", "Unit", "Min", "Max", "Segment"];
const ESTIMATE_ALIGN: [Align; 5] = [Left, Left, Right, Right, Left];

impl IntervalEstimates {
    /// Renders the report as markdown that is also readable as plain text. See the module docs for
    /// what that constraint rules out.
    ///
    /// [`fmt::Display`] delegates here, so there is one rendering rather than two that could drift.
    pub fn to_markdown(&self) -> String {
        let mut out: Vec<String> = Vec::new();

        out.push(h1("EV Peak Power Contribution"));
        out.push(String::new());
        out.push(format!(
            "Source     {}",
            self.source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.source.to_string_lossy().into_owned())
        ));
        out.push(format!("Interval   {}", interval_line(self.interval)));
        out.push(String::new());
        out.push(String::new());

        self.push_estimates(&mut out);
        self.push_segments(&mut out);
        self.push_membership(&mut out);
        self.push_excluded(&mut out);
        self.push_anomalies(&mut out);

        let mut s = out.join("\n");
        s.push('\n');
        s
    }

    /// Whether no session reached any segment of the interval.
    fn is_deserted(&self) -> bool {
        self.seg_estimates
            .iter()
            .all(|(seg, _)| seg.sessions.is_empty())
    }

    /// Average power per session id, gathered from the segments.
    ///
    /// The segments are the report's only handle on the sessions themselves: an anomaly record
    /// carries a row, an id and a kind, and nothing else. Every session with an anomaly intersects
    /// the interval of interest, and so appears in some segment of it, so anything the anomaly list
    /// can name is reachable here.
    ///
    /// A spike's figure is the one the estimating logic uses, not the sheet's `#DIV/0!` — the
    /// number that actually fed the totals, which is the one worth seeing beside an anomaly.
    fn avg_kw_by_session(&self) -> HashMap<String, f64> {
        self.seg_estimates
            .iter()
            .flat_map(|(seg, _)| seg.sessions.iter())
            .map(|s| (s.id.clone(), s.avg_kw()))
            .collect()
    }

    /// The figures for the maximal segment on each derivation, and the prose that reads them.
    fn push_estimates(&self, out: &mut Vec<String>) {
        out.push(h2("Estimates"));
        out.push(String::new());

        let (energy_seg, energy_est) = &self.energy_based_seg_estimate;
        let (count_seg, count_est) = &self.count_based_seg_estimate;
        let row = |label: &str, unit: &str, b: Bracket<f64>, seg: &Segment| {
            vec![
                label.to_owned(),
                unit.to_owned(),
                format!("{:.3}", b.min),
                format!("{:.3}", b.max),
                hm(seg.start()),
            ]
        };
        let rows = vec![
            row("Energy-based", "kW", energy_est.energy_based_kw, energy_seg),
            row("Energy-based", "kVA", energy_est.energy_based_kva, energy_seg),
            row("Count-based", "kW", count_est.count_based_kw, count_seg),
            row("Count-based", "kVA", count_est.count_based_kva, count_seg),
        ];
        out.push(table(&ESTIMATE_HEADERS, &rows, &ESTIMATE_ALIGN));
        out.push(String::new());

        out.push(wrap(
            "Every figure is a bracket: the reported session times are stated only to the minute, \
             so each estimate runs from what those times least support to what they most support. \
             \"Energy-based\" is derived from the sessions' own consumption, \"Count-based\" from \
             how many of them were charging and the per-EV rating of the infrastructure. \
             \"Segment\" names the 15-minute segment the figure was drawn from - the one where \
             that derivation peaks, which the two need not agree on.",
            "",
        ));
        out.push(String::new());

        out.push(wrap(
            "The peak is always a 15-minute average, whatever the length of the interval asked \
             for, because that is the basis the demand charge is billed on. An hour is reported as \
             the highest of its four segments, not as an average over the whole hour.",
            "",
        ));

        if self.is_deserted() {
            out.push(String::new());
            out.push(wrap(
                "No session intersected the interval of interest, so no vehicle charged in it. \
                 The figures above are not zero all the same: the charging infrastructure draws a \
                 standing block whenever the transformer is energised, and that block is part of \
                 the building's demand whether or not a car is plugged in.",
                "",
            ));
        }

        if !self.excluded_sessions.is_empty() {
            out.push(String::new());
            let n = self.excluded_sessions.len();
            out.push(wrap(
                &format!(
                    "{} in the workbook {} excluded from every figure above, having reported \
                     times that contradict each other. They are listed under Excluded sessions.",
                    if n == 1 {
                        "One session".to_owned()
                    } else {
                        format!("{n} sessions")
                    },
                    if n == 1 { "was" } else { "were" },
                ),
                "",
            ));
        }

        out.push(String::new());
        out.push(String::new());
    }

    /// Every segment of the interval, with the two aggregates every estimate is derived from.
    ///
    /// `agg_count` and `agg_kw` and nothing else: the four estimates are functions of these two,
    /// so a table of the estimates per segment would repeat the Estimates section four times over
    /// while saying no more than this does.
    fn push_segments(&self, out: &mut Vec<String>) {
        out.push(h2("Segments"));
        out.push(String::new());

        let rows: Vec<Vec<String>> = self
            .seg_estimates
            .iter()
            .map(|(seg, _)| {
                vec![
                    hm(seg.start()),
                    bracket_cell(seg.agg_count()),
                    bracket_cell(seg.agg_kw()),
                ]
            })
            .collect();
        out.push(table(
            &["Segment", "Count", "kW"],
            &rows,
            &[Left, Right, Right],
        ));
        out.push(String::new());
        out.push(wrap(
            "Times are local (ET), and each segment is 15 minutes long, named by the minute it \
             starts on. Segments are half-open: each runs from its own start up to but not \
             including the next one's, so no instant falls in two of them and they tile the \
             interval exactly. \"Count\" is a session count weighted by how much of the segment \
             each session covered, so it is fractional; \"kW\" weights each session's average \
             power the same way.",
            "",
        ));
        out.push(String::new());
        out.push(String::new());
    }

    /// Which sessions are in which segment.
    fn push_membership(&self, out: &mut Vec<String>) {
        out.push(h2("Segment membership"));
        out.push(String::new());

        for (seg, _) in &self.seg_estimates {
            let ids: Vec<String> = seg.sessions.iter().map(|s| s.id.clone()).collect();
            let body = if ids.is_empty() {
                "none".to_owned()
            } else {
                ids.join(", ")
            };
            // Wrapped rather than put in a table cell: a markdown row is one line, so a segment of
            // twelve sessions could not be broken across lines inside one.
            out.push(wrap(&format!("- {} - {body}", hm(seg.start())), "  "));
        }

        out.push(String::new());
        out.push(String::new());
    }

    /// Every session excluded from the estimates, whether or not it appears to touch the interval.
    ///
    /// Listed in full rather than filtered, because the filter would be applied to exactly the
    /// timestamps that are in doubt. A session whose fields contradict each other may belong in
    /// this interval and still test as falling outside it, so "In interval" is reported as what it
    /// is — a reading of the same unreliable times — and no row is dropped on its say-so.
    fn push_excluded(&self, out: &mut Vec<String>) {
        if self.excluded_sessions.is_empty() {
            return;
        }
        out.push(h2("Excluded sessions"));
        out.push(String::new());

        let rows: Vec<Vec<String>> = self
            .excluded_sessions
            .iter()
            .map(|s| {
                vec![
                    s.row.to_string(),
                    s.id.clone(),
                    ymd_hm(s.conn_start),
                    ymd_hm_to(s.conn_start, s.adj_conn_end),
                    in_interval(s, &self.interval),
                    // An excluded session is in no segment, but the report holds the session
                    // itself here, so its figure needs no lookup.
                    s.anomalies
                        .iter()
                        .map(|k| anomaly_cell(*k, Some(s.avg_kw())))
                        .collect::<Vec<_>>()
                        .join(", "),
                ]
            })
            .collect();
        out.push(table(
            &["Row", "Session", "From", "To", "In interval", "Anomaly"],
            &rows,
            &[Right, Left, Left, Left, Left, Left],
        ));
        out.push(String::new());
        out.push(wrap(
            "These sessions take no part in any estimate. Times are local (ET), and the list \
             covers the whole workbook rather than the interval estimated, so \"From\" carries its \
             date and \"To\" carries one only when the session crosses midnight. \"In interval\" \
             is whether the session appears to fall in the interval - appears only, because a \
             record whose own fields contradict each other cannot be trusted to say where it \
             belongs. It reads the same doubtful times, so no row was dropped on its say-so.",
            "",
        ));
        out.push(String::new());

        glossary(
            self.excluded_sessions
                .iter()
                .flat_map(|s| s.anomalies.iter().copied()),
            out,
        );
        out.push(String::new());
        out.push(String::new());
    }

    fn push_anomalies(&self, out: &mut Vec<String>) {
        out.push(h2("Anomalies"));
        out.push(String::new());

        if self.session_anomalies.is_empty() {
            out.push(wrap(
                "None. Every session considered for this interval was well formed.",
                "",
            ));
            out.push(String::new());
            return;
        }

        // The sessions themselves are not on an `Anomaly`; only a segment holds them.
        let avg_kw = self.avg_kw_by_session();

        // No "In interval" column here, unlike the Excluded sessions table. Every session listed
        // reaches the interval of interest — that is the condition on which the anomaly was
        // collected at all — so the column would read yes on every row of every report, and a
        // column with one possible value tells a reader nothing while inviting them to look for a
        // distinction that is not there. The scoping is stated in the note below instead.
        let rows: Vec<Vec<String>> = self
            .session_anomalies
            .iter()
            .map(|a: &Anomaly| {
                vec![
                    a.row.to_string(),
                    a.session_id.clone(),
                    anomaly_cell(a.kind, avg_kw.get(a.session_id.as_str()).copied()),
                ]
            })
            .collect();
        out.push(table(
            &["Row", "Session", "Anomaly"],
            &rows,
            &[Right, Left, Left],
        ));
        out.push(String::new());
        let mut note = "Row numbers are workbook rows, so each one can be looked up directly. \
                        Only sessions reaching the interval of interest are listed here"
            .to_owned();
        if self.excluded_sessions.is_empty() {
            note.push_str(
                "; a session anomalous elsewhere in the workbook is not this interval's concern.",
            );
        } else {
            note.push_str(
                ". The Excluded sessions table above is scoped differently - it covers the whole \
                 workbook, and carries an \"In interval\" column for that reason.",
            );
        }
        out.push(wrap(&note, ""));
        out.push(String::new());

        glossary(self.session_anomalies.iter().map(|a| a.kind), out);
        out.push(String::new());
    }
}

/// The header's interval line, naming the UTC offset in force at each end.
///
/// Naming it is not decoration. On the night DST ends an hour of wall time occurs twice, so an
/// interval can begin at `01:30` and end at `01:30` — the same clock reading an hour apart. Written
/// as bare local times that reads as a window of no duration; written with the offsets it reads as
/// what it is. When both ends share an offset, which is every interval but two a year, it is stated
/// once at the end.
fn interval_line(interval: Interval) -> String {
    let (lo, hi) = (interval.start, interval.end());
    let (lo_z, hi_z) = (local(lo), local(hi));
    let (lo_off, hi_off) = (
        lo_z.strftime("%Z").to_string(),
        hi_z.strftime("%Z").to_string(),
    );
    let length = interval_length(lo, hi);
    if lo_off == hi_off {
        format!(
            "{} - {} {lo_off}  ({length})",
            lo_z.strftime("%Y-%m-%d %H:%M"),
            hi_z.strftime("%H:%M"),
        )
    } else {
        format!(
            "{} {lo_off} - {} {hi_off}  ({length})",
            lo_z.strftime("%Y-%m-%d %H:%M"),
            hi_z.strftime("%H:%M"),
        )
    }
}

/// "1 hour" / "15 minutes", for the header.
fn interval_length(lo: Timestamp, hi: Timestamp) -> String {
    let plural = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    let secs = hi.duration_since(lo).as_secs();
    match secs {
        s if s % 3600 == 0 => plural(s / 3600, "hour"),
        s if s % 60 == 0 => plural(s / 60, "minute"),
        s => plural(s, "second"),
    }
}

impl fmt::Display for IntervalEstimates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_markdown())
    }
}

// ---------------------------------------------------------------------------
// Site load
// ---------------------------------------------------------------------------

/// Percentage scaling, so the two places that need it read as one intent rather than as a bare
/// `100.0`.
const PERCENT: f64 = 100.0;

/// The site load model tabulated for every vehicle count the panel can hold.
///
/// Fixed-width plain text rather than markdown: this is a table of the model's own constants, read
/// beside `docs/ev-charger-power-factor-and-kva-allocation.md`, not a document anyone renders.
pub fn site_load_report() -> String {
    let mut out = String::new();
    let per_ev = ev_load();

    out.push_str("Level 2 EV charging site - load at transformer primary\n\n");
    out.push_str(&format!(
        "  Panel            {:.0} V, {} x {:.0} A breakers\n",
        PANEL_VOLTAGE_V, BREAKER_COUNT, BREAKER_RATING_A
    ));
    out.push_str(&format!(
        "  Pilot current    {:.1} A per vehicle ({:.0}% continuous derate)\n",
        ev_pilot_current_a(),
        CONTINUOUS_DUTY_DERATE * PERCENT
    ));
    out.push_str(&format!(
        "  Per vehicle      {:.2} kVA = {:.2} kW + {:.2} kvar + {:.2} kvar distortion\n",
        per_ev.apparent_kva(),
        per_ev.real_kw,
        per_ev.reactive_kvar,
        per_ev.distortion_kvar
    ));
    out.push_str(&format!(
        "  Transformer      {:.0} kVA\n\n",
        XFMR_RATING_KVA
    ));

    out.push_str(&format!(
        "{:>4}  {:>9}  {:>9}  {:>11}  {:>9}  {:>7}  {:>8}\n",
        "EVs", "kW", "kvar", "kvar (dis)", "kVA", "PF", "% rated"
    ));
    out.push_str(&format!("{}\n", "-".repeat(69)));

    for ev_count in 0..=BREAKER_COUNT {
        let load = site_load(ev_count);
        let percent = loading_ratio(load) * PERCENT;
        let flag = if percent > PERCENT {
            "  <- over nameplate"
        } else {
            ""
        };

        out.push_str(&format!(
            "{:>4}  {:>9.2}  {:>9.2}  {:>11.2}  {:>9.2}  {:>7.3}  {:>7.1}%{}\n",
            ev_count,
            load.real_kw,
            load.reactive_kvar,
            load.distortion_kvar,
            load.apparent_kva(),
            load.true_power_factor(),
            percent,
            flag
        ));
    }

    let full = site_load(BREAKER_COUNT);
    out.push_str(&format!(
        "\nAt full occupancy: {:.2} kW, {:.2} kVA, {:.1}% of nameplate.\n",
        full.real_kw,
        full.apparent_kva(),
        loading_ratio(full) * PERCENT
    ));

    out
}
