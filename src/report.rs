//! Renders a [`PowerEstimatesReport`] as markdown that also reads as plain text.
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
//! - **No emphasis markers.** Labels are quoted — `"Nominal"`, `"Minimum overlap"` — which reads
//!   identically either way. One marker is used, an asterisk against a dubious *group*. It is not
//!   emphasis, so it does not collide with a renderer's reading of `*`.
//! - **Session ids live in their own section**, not in a table cell, because a markdown table row is
//!   a single line and a group of twelve sessions cannot be wrapped inside one.

use crate::{AnomalyKind, EstimateSet, PowerEstimatesReport, View, time_zone};
use jiff::{Timestamp, Zoned};
use std::{fmt, time::Duration};

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

/// `M:SS`, which is enough: an interval of interest is at most an hour, and a group cannot outlast
/// the interval it tiles.
fn mmss(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn local(ts: Timestamp) -> Zoned {
    Zoned::new(ts, time_zone())
}

fn hms(ts: Timestamp) -> String {
    local(ts).strftime("%H:%M:%S").to_string()
}

/// Dated, and to the minute. The excluded list covers the whole workbook, so its dates cannot be
/// left implicit the way they can inside a single interval of interest — and unlike a group
/// boundary, which the interval can clamp to any instant, a session's reported times are always
/// whole minutes, so seconds here would only imply a precision the workbook does not have.
fn ymd_hm(ts: Timestamp) -> String {
    local(ts).strftime("%Y-%m-%d %H:%M").to_string()
}

/// Group number, with the anomaly marker in a slot of its own so the digits stay in a column:
/// `0 `, `1*`, `2 ` align, whereas `0`, `1*`, `2` stagger under right alignment.
///
/// The slot exists only when some group in the table is marked. Reserving it unconditionally would
/// widen the column by one in every report, for a marker no report has yet carried.
fn group_number(idx: usize, flagged: bool, any_flagged: bool) -> String {
    match (flagged, any_flagged) {
        (true, _) => format!("{idx}*"),
        (false, true) => format!("{idx} "),
        (false, false) => idx.to_string(),
    }
}

fn estimate_rows(set: &EstimateSet) -> Vec<Vec<String>> {
    vec![
        vec![
            "Consumption-based".to_owned(),
            format!("{:.3}", set.consumption_based_kw.value),
            format!("{:.3}", set.consumption_based_kva.value),
            set.consumption_based_kw.session_group_idx.to_string(),
        ],
        vec![
            "Breaker-spec-based".to_owned(),
            format!("{:.3}", set.breaker_specs_based_kw.value),
            format!("{:.3}", set.breaker_specs_based_kva.value),
            set.breaker_specs_based_kw.session_group_idx.to_string(),
        ],
    ]
}

const ESTIMATE_HEADERS: [&str; 4] = ["Estimate", "kW", "kVA", "Group"];
const ESTIMATE_ALIGN: [Align; 4] = [Left, Right, Right, Right];

impl PowerEstimatesReport {
    /// Renders the report as markdown that is also readable as plain text. See the module docs for
    /// what that constraint rules out.
    ///
    /// [`fmt::Display`] delegates here, so there is one rendering rather than two that could drift.
    pub fn to_markdown(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        let mut push = |s: String| out.push(s);

        push(h1("EV Peak Power Contribution"));
        push(String::new());
        push(format!(
            "Source     {}",
            self.source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.source.to_string_lossy().into_owned())
        ));
        push(format!("Interval   {}", interval_line(self.interval)));
        push(String::new());
        push(String::new());

        self.push_estimates(&mut out);
        self.push_excluded(&mut out);
        self.push_groups(&mut out);
        self.push_membership(&mut out);
        self.push_anomalies(&mut out);

        let mut s = out.join("\n");
        s.push('\n');
        s
    }

    fn push_estimates(&self, out: &mut Vec<String>) {
        out.push(h2("Estimates"));
        out.push(String::new());

        let Some(estimates) = &self.estimates else {
            out.push(wrap(
                "No session intersected the interval of interest, so there is nothing to \
                 estimate. EV charging contributed nothing to demand in this window.",
                "",
            ));
            out.push(String::new());
            out.push(String::new());
            return;
        };

        // The name is quoted and the gloss is not, per the module docs: a label has to read as a
        // label in plain text, and quotation marks are the only device available.
        let mut sets: Vec<(&str, &str, &EstimateSet)> = vec![(
            "Nominal",
            "every group's membership taken at face value",
            &estimates.nominal,
        )];
        if let Some(set) = &estimates.min_overlap {
            sets.push((
                "Minimum overlap",
                "assuming the sessions in each dubious group overlapped as little as \
                 their reported times allow",
                set,
            ));
        }

        if sets.len() > 1 {
            out.push(wrap(
                "More than one reading of the data is defensible here, because some group holds \
                 two sessions that need not have overlapped each other - one is reported as \
                 ending in the same minute the other is reported as starting, and the report \
                 states those times only to the minute. Both readings are given below. The first \
                 counts every session and assumes nothing, so it is the one to quote if only one \
                 figure is wanted.",
                "",
            ));
            out.push(String::new());
        }

        for (name, gloss, set) in &sets {
            if sets.len() > 1 {
                out.push(wrap(&format!("\"{name}\" - {gloss}:"), "  "));
                out.push(String::new());
            }
            out.push(table(
                &ESTIMATE_HEADERS,
                &estimate_rows(set),
                &ESTIMATE_ALIGN,
            ));
            out.push(String::new());
        }

        // `min_overlap <= nominal` on all four figures, unconditionally — see `PowerEstimates` —
        // so the bracket needs no search: its floor is the consumption figure of the last set
        // printed and its ceiling the breaker-spec figure of the first. With one set present both
        // come from it, and the sentence reads the same.
        //
        // Selecting on kW serves kVA too. kVA is kW over a fixed power factor on the consumption
        // side, and a fixed per-breaker rating times the same count on the other, so both are
        // strictly increasing in whatever kW ranks on and the two ends cannot differ.
        let low = sets.last().expect("`nominal` is always present");
        let high = sets.first().expect("`nominal` is always present");
        // The gloss belongs over its table, not in a sentence naming both sets; and with only one
        // set there is nothing to name at all.
        let name = |name: &str| {
            if sets.len() > 1 {
                format!("\"{name}\", ")
            } else {
                String::new()
            }
        };
        out.push(wrap(
            &format!(
                "The likely kW values are in the range from {:.3} kW ({}consumption-based) to \
                 {:.3} kW ({}breaker-spec-based). The likely kVA values are in the range from \
                 {:.3} kVA ({}consumption-based) to {:.3} kVA ({}breaker-spec-based).",
                low.2.consumption_based_kw.value,
                name(low.0),
                high.2.breaker_specs_based_kw.value,
                name(high.0),
                low.2.consumption_based_kva.value,
                name(low.0),
                high.2.breaker_specs_based_kva.value,
                name(high.0),
            ),
            "",
        ));

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

    /// Every session excluded from the estimates, whether or not it appears to touch the interval.
    ///
    /// Listed in full rather than filtered to the interval, because the filter would be applied to
    /// exactly the timestamps that are in doubt. A session whose fields contradict each other may
    /// belong in this window and still test as falling outside it, so "In window" is reported as
    /// what it is — a reading of the same unreliable times — and no row is dropped on its say-so.
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
                    ymd_hm(s.adj_conn_end),
                    if s.intersects(self.interval) {
                        "yes"
                    } else {
                        "no"
                    }
                    .to_owned(),
                    s.anomalies
                        .iter()
                        .map(|k| k.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ]
            })
            .collect();
        out.push(table(
            &["Row", "Session", "From", "To", "Window", "Anomaly"],
            &rows,
            &[Right, Left, Left, Left, Left, Left],
        ));
        out.push(String::new());
        out.push(wrap(
            "These sessions take no part in any estimate. Times are local (ET), and the list \
             covers the whole workbook rather than the interval of interest. \"Window\" is whether \
             the session appears to fall in that interval - appears only, because a record whose \
             own fields contradict each other cannot be trusted to say which window it belongs \
             in. It reads the same doubtful times, so no row was dropped on its say-so.",
            "",
        ));
        out.push(String::new());

        let mut seen: Vec<AnomalyKind> = Vec::new();
        for s in &self.excluded_sessions {
            for kind in &s.anomalies {
                if !seen.contains(kind) {
                    seen.push(*kind);
                    out.push(wrap(&format!("- {} - {}.", kind.as_str(), kind), "  "));
                }
            }
        }
        out.push(String::new());
        out.push(String::new());
    }

    fn push_groups(&self, out: &mut Vec<String>) {
        out.push(h2("Session groups"));
        out.push(String::new());

        if self.session_groups.is_empty() {
            out.push("None. No session intersected the interval of interest.".to_owned());
            out.push(String::new());
            out.push(String::new());
            return;
        }

        // One predicate for both the marker and the extra columns: a dubious group is exactly one
        // whose two readings differ, so a marked row is a row where the columns say something and
        // an unmarked table is one where they would repeat each other exactly.
        let any_dubious = self.session_groups.iter().any(|g| g.is_dubious());

        let rows: Vec<Vec<String>> = self
            .session_groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let mut row = vec![
                    group_number(i, g.is_dubious(), any_dubious),
                    hms(g.start()),
                    hms(g.end()),
                    mmss(g.duration()),
                    g.size().to_string(),
                    format!("{:.3}", g.agg_avg_power()),
                ];
                if any_dubious {
                    row.push(g.size_in(View::MinOverlap).to_string());
                    row.push(format!("{:.3}", g.agg_avg_power_in(View::MinOverlap)));
                }
                row
            })
            .collect();

        let mut headers = vec!["#", "From", "To", "Len", "Count", "kW"];
        let mut align = vec![Right, Left, Left, Right, Right, Right];
        if any_dubious {
            headers.extend(["Min Count", "Min kW"]);
            align.extend([Right, Right]);
        }
        out.push(table(&headers, &rows, &align));
        out.push(String::new());
        // Deliberately not "the lengths sum to the interval": groups tile only the part of it that
        // has a session in it, so they sum to the interval exactly just when some session spans
        // the whole window. What always holds is that they neither overlap nor double-count.
        out.push(wrap(
            "Times are local (ET). Groups are half-open: each runs from its From up to but not \
             including its To, so no instant falls in two groups and no session is counted twice.",
            "",
        ));

        if any_dubious {
            out.push(String::new());
            out.push(wrap(
                "An asterisk marks a dubious group: one holding two sessions that need not have \
                 overlapped each other, because one is reported as ending in the same minute the \
                 other is reported as starting. \"Count\" and \"kW\" take its membership at face \
                 value; \"Min Count\" and \"Min kW\" assume as little overlap as the reported \
                 times allow. They differ on exactly the marked rows.",
                "",
            ));
        }
        out.push(String::new());
        out.push(String::new());
    }

    fn push_membership(&self, out: &mut Vec<String>) {
        if self.session_groups.is_empty() {
            return;
        }
        out.push(h2("Group membership"));
        out.push(String::new());
        for (i, g) in self.session_groups.iter().enumerate() {
            let ids: Vec<String> = g.session_iter().map(|s| s.id.clone()).collect();
            // Wrapped rather than put in a table cell: a markdown row is one line, so a group of
            // twelve sessions could not be broken across lines inside one.
            out.push(wrap(&format!("- Group {i} - {}", ids.join(", ")), "  "));
        }
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

        let rows: Vec<Vec<String>> = self
            .session_anomalies
            .iter()
            .map(|a| {
                vec![
                    a.row.to_string(),
                    a.session_id.clone(),
                    a.kind.as_str().to_owned(),
                ]
            })
            .collect();
        out.push(table(
            &["Row", "Session", "Anomaly"],
            &rows,
            &[Right, Left, Left],
        ));
        out.push(String::new());
        out.push(wrap(
            "Row numbers are workbook rows, so each one can be looked up directly.",
            "",
        ));
        out.push(String::new());

        // The prose comes from each kind's `Display`, so there is one wording to maintain rather
        // than a second copy here that could drift from it. Only kinds actually present are
        // explained.
        let mut seen: Vec<AnomalyKind> = Vec::new();
        for a in &self.session_anomalies {
            if !seen.contains(&a.kind) {
                seen.push(a.kind);
            }
        }
        for kind in seen {
            out.push(wrap(&format!("- {} - {}.", kind.as_str(), kind), "  "));
        }

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
fn interval_line(interval: (Timestamp, Timestamp)) -> String {
    let (lo, hi) = interval;
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
    let secs = hi.duration_since(lo).as_secs();
    match secs {
        3600 => "1 hour".to_owned(),
        s if s % 3600 == 0 => format!("{} hours", s / 3600),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

impl fmt::Display for PowerEstimatesReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_markdown())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        Anomaly, RSession, Session, groups_for_interval, max_power_estimates_for_interval,
    };
    use std::{cell::RefCell, path::PathBuf, rc::Rc};

    const LO: &str = "2026-06-01T20:00:00Z";
    const HI: &str = "2026-06-01T21:00:00Z";

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn rsession(id: &str, start: &str, end: &str, avg_power: f64) -> RSession {
        let conn_start = ts(start);
        let adj_conn_end = ts(end);
        Rc::new(RefCell::new(Session {
            id: id.to_owned(),
            row: 2,
            conn_start,
            conn_end: adj_conn_end,
            adj_conn_end,
            conn_duration: adj_conn_end.duration_since(conn_start).unsigned_abs(),
            charge_time: Duration::from_secs(60),
            energy_use: 1.0,
            avg_power,
            anomalies: Vec::new(),
        }))
    }

    /// Builds a report over `sessions` without touching the filesystem.
    fn report_of(sessions: Vec<RSession>, anomalies: Vec<Anomaly>) -> PowerEstimatesReport {
        let groups = groups_for_interval((ts(LO), ts(HI)), &sessions);
        PowerEstimatesReport {
            excluded_sessions: Vec::new(),
            source: PathBuf::from("/tmp/Session_Report_Test.xlsx"),
            interval: (ts(LO), ts(HI)),
            estimates: crate::estimates::estimates_for_groups(&groups),
            session_groups: groups,
            session_anomalies: anomalies,
        }
    }

    /// `n` concurrent sessions at 1 kW each, spanning most of the interval.
    fn concurrent(n: usize) -> Vec<RSession> {
        (0..n)
            .map(|i| {
                rsession(
                    &format!("S{i:02}"),
                    "2026-06-01T20:05:00Z",
                    "2026-06-01T20:55:00Z",
                    1.0,
                )
            })
            .collect()
    }

    /// The constraint the whole module exists for: nothing in the output may rely on a markdown
    /// renderer to be legible, and nothing may accidentally trigger markdown formatting.
    fn assert_plain_text_safe(md: &str) {
        for (i, line) in md.lines().enumerate() {
            assert!(
                !line.starts_with("    "),
                "line {i} is indented four spaces, which markdown renders as a code block: {line:?}"
            );
            assert!(
                !line.contains("<br"),
                "line {i} uses an HTML break, which shows literally in plain text: {line:?}"
            );
            assert!(
                !line.starts_with('#'),
                "line {i} is a hash heading; setext underlines read better raw: {line:?}"
            );
            assert!(
                line.len() <= 90,
                "line {i} is {} columns, too wide to read raw: {line:?}",
                line.len()
            );
        }
        // Emphasis markers would collide with the asterisk that marks an anomalous group.
        assert!(
            !md.contains("**"),
            "bold markers render as noise in plain text"
        );
        assert!(!md.contains('`'), "backticks render as noise in plain text");
    }

    #[test]
    fn report_renders_and_display_agrees_with_to_markdown() {
        let report = report_of(concurrent(3), Vec::new());
        let md = report.to_markdown();
        assert_eq!(format!("{report}"), md);
        assert_plain_text_safe(&md);

        assert!(md.starts_with("EV Peak Power Contribution\n=========================="));
        assert!(md.contains("Source     Session_Report_Test.xlsx"));
        assert!(
            md.contains("Interval   2026-06-01 16:00 - 17:00 EDT  (1 hour)"),
            "{md}"
        );
        // Three sessions at 1 kW: consumption 3.000, breaker 3 x 6.7.
        assert!(md.contains("| Consumption-based  |  3.000 |"), "{md}");
        assert!(md.contains("| Breaker-spec-based | 20.100 |"), "{md}");
        assert!(md.contains("- Group 0 - S00, S01, S02"), "{md}");
        assert!(md.contains("None. Every session considered"), "{md}");
        // No group in doubt, so no marker slot and no second estimate table.
        assert!(md.contains("| # |"), "{md}");
        assert!(!md.contains("\"Minimum overlap\""), "{md}");
        assert!(!md.contains("Min Count"), "{md}");
    }

    /// A dubious group: `E` is reported as ending in the same minute `S` is reported as starting,
    /// so the minute they share may hold both of them or one at a time. Both readings are printed,
    /// the group is marked, and the group table grows the two columns that show the difference.
    #[test]
    fn dubious_group_renders_both_estimate_sets_and_marks_the_group() {
        let sessions = vec![
            rsession("E", "2026-06-01T20:02:00Z", "2026-06-01T20:05:00Z", 5.0),
            rsession("S", "2026-06-01T20:04:00Z", "2026-06-01T20:30:00Z", 4.0),
        ];
        let report = report_of(sessions, Vec::new());
        let md = report.to_markdown();
        assert_plain_text_safe(&md);

        assert!(
            md.contains("\"Nominal\" - every group's membership taken at face value:"),
            "{md}"
        );
        assert!(md.contains("\"Minimum overlap\" - assuming"), "{md}");
        // Nominal sums both; minimum overlap keeps only the larger of the two.
        assert!(md.contains("| Consumption-based  |  9.000 |"), "{md}");
        assert!(md.contains("| Consumption-based  | 5.000 |"), "{md}");
        // The group carries the marker, and the legend explaining it appears.
        assert!(md.contains("| 1* |"), "{md}");
        assert!(md.contains("An asterisk marks a dubious group"), "{md}");
        // The per-group columns appear only when some group is dubious.
        assert!(md.contains("Min Count"), "{md}");
        assert!(md.contains("Min kW"), "{md}");
    }

    /// Every anomaly kind present gets a table row and exactly one legend entry.
    #[test]
    fn anomalies_are_tabulated_and_explained_once_each() {
        let anomalies = vec![
            Anomaly {
                row: 47,
                session_id: "S31882".to_owned(),
                kind: AnomalyKind::InconsistentDuration,
            },
            Anomaly {
                row: 91,
                session_id: "S60417".to_owned(),
                kind: AnomalyKind::InconsistentDuration,
            },
            Anomaly {
                row: 152,
                session_id: "S70933".to_owned(),
                kind: AnomalyKind::ZeroActiveChargeTime,
            },
        ];
        let report = report_of(concurrent(2), anomalies);
        let md = report.to_markdown();
        assert_plain_text_safe(&md);

        // Exact rows, so the padding is pinned too. There is no "Excluded" column: a session that
        // is excluded outright appears under Excluded sessions instead, so every row here would
        // read the same and the column carried no information.
        assert!(
            md.contains("|  47 | S31882  | InconsistentDuration |"),
            "{md}"
        );
        assert!(
            md.contains("| 152 | S70933  | ZeroActiveChargeTime |"),
            "{md}"
        );
        // Two rows share a kind, but it is explained once.
        assert_eq!(md.matches("- InconsistentDuration - ").count(), 1, "{md}");
        assert_eq!(md.matches("- ZeroActiveChargeTime - ").count(), 1, "{md}");
    }

    /// The header names the offset at each end, which matters exactly once a year.
    ///
    /// On the night DST ends, an hour begun at 01:00 EDT finishes at 01:00 EST — the same clock
    /// reading, an hour later. Rendered as bare local times that reads as a window of no duration,
    /// so both offsets are named. No fixture reaches this; it can only be built directly.
    #[test]
    fn a_fold_spanning_interval_names_both_offsets() {
        let lo = ts("2026-11-01T05:00:00Z"); // 01:00 EDT
        let hi = ts("2026-11-01T06:00:00Z"); // 01:00 EST
        let mut sessions = vec![rsession(
            "F",
            "2026-11-01T05:10:00Z",
            "2026-11-01T05:50:00Z",
            6.0,
        )];
        let groups = groups_for_interval((lo, hi), &mut sessions);
        let report = PowerEstimatesReport {
            excluded_sessions: Vec::new(),
            source: PathBuf::from("Fold.xlsx"),
            interval: (lo, hi),
            estimates: crate::estimates::estimates_for_groups(&groups),
            session_groups: groups,
            session_anomalies: Vec::new(),
        };
        let md = report.to_markdown();
        assert_plain_text_safe(&md);
        assert!(
            md.contains("Interval   2026-11-01 01:00 EDT - 01:00 EST  (1 hour)"),
            "{md}"
        );
    }

    /// An interval no session reached still renders, and says so rather than printing nothing.
    #[test]
    fn empty_interval_renders_an_explanation() {
        let report = report_of(Vec::new(), Vec::new());
        let md = report.to_markdown();
        assert_plain_text_safe(&md);
        assert!(md.contains("No session intersected the interval"), "{md}");
        assert!(md.contains("None. No session intersected"), "{md}");
        assert!(!md.contains("Group membership"), "{md}");
    }

    /// Table cells are padded to a common width, which is what makes the output legible without a
    /// renderer. Checked by requiring every row of a table to be the same length.
    #[test]
    fn table_rows_are_padded_to_equal_width() {
        let md = report_of(concurrent(3), Vec::new()).to_markdown();
        let mut block: Vec<usize> = Vec::new();
        for line in md.lines().chain(std::iter::once("")) {
            if line.starts_with('|') {
                block.push(line.chars().count());
            } else if !block.is_empty() {
                assert!(
                    block.iter().all(|w| *w == block[0]),
                    "ragged table in:\n{md}"
                );
                block.clear();
            }
        }
    }

    /// End to end through the public API, so the rendered report is pinned against a real workbook
    /// rather than hand-built groups.
    #[test]
    fn renders_a_report_read_from_a_workbook() {
        let dir = std::env::temp_dir().join(format!("ev_peak_report_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("Session_Report_Diagram.csv");
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/Session_Report_Diagram.csv"),
            &csv,
        )
        .unwrap();
        let xlsx = crate::session_csv_to_xlsx(&csv).unwrap().output_path;

        let report = max_power_estimates_for_interval(
            (ts("2026-06-15T20:00:00Z"), ts("2026-06-15T21:00:00Z")),
            &xlsx,
        )
        .unwrap();
        let md = report.to_markdown();
        assert_plain_text_safe(&md);

        assert!(
            md.contains("Source     Session_Report_Diagram.xlsx"),
            "{md}"
        );
        assert!(
            md.contains("Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)"),
            "{md}"
        );
        // The diagram's peak: five sessions, 31.4 kW, in group 5.
        assert!(
            md.contains("| Consumption-based  | 31.400 | 33.053 |     5 |"),
            "{md}"
        );
        assert!(
            md.contains("| Breaker-spec-based | 33.500 | 37.500 |     5 |"),
            "{md}"
        );
        // Group 5 is the one-minute sliver where D and E are reported as ending in the minute F
        // is reported as starting, so it is dubious and carries the marker. Its minimum reading is
        // group 4's membership exactly - the case where F had not yet started.
        assert!(
            md.contains(
                "| 5* | 16:34:00 | 16:35:00 |  1:00 |     5 | 31.400 |         4 | 24.700 |"
            ),
            "{md}"
        );
        assert!(
            md.contains("| Consumption-based  | 24.700 | 26.000 |     4 |"),
            "the tie with group 4 goes to the group that is not dubious: {md}"
        );
        assert!(md.contains("- Group 5 - A, C, D, E, F"), "{md}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
