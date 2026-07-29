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
//! - **No emphasis markers.** Labels are quoted — `"Direct"`, `"Clamped"` — which reads identically
//!   either way. An asterisk means one thing only in this document: it marks an anomalous group.
//! - **Session ids live in their own section**, not in a table cell, because a markdown table row is
//!   a single line and a group of twelve sessions cannot be wrapped inside one.

use crate::{
    AnomalyKind, EstimateSet, GroupAnomaly, PowerEstimatesReport, SessionGroup, time_zone,
};
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
        let (lo, hi) = self.interval;
        push(format!(
            "Interval   {} - {} ET  ({})",
            local(lo).strftime("%Y-%m-%d %H:%M"),
            local(hi).strftime("%H:%M"),
            interval_length(lo, hi)
        ));
        push(String::new());
        push(String::new());

        self.push_estimates(&mut out);
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

        let direct = &estimates.direct;
        match &estimates.clamped {
            None => {
                out.push(table(
                    &ESTIMATE_HEADERS,
                    &estimate_rows(direct),
                    &ESTIMATE_ALIGN,
                ));
                out.push(String::new());
                out.push(wrap(
                    &format!(
                        "The likely kW values are in the range from {:.3} kW \
                         (consumption-based) to {:.3} kW (breaker-spec-based). The likely kVA \
                         values are in the range from {:.3} kVA (consumption-based) to {:.3} kVA \
                         (breaker-spec-based).",
                        direct.consumption_based_kw.value,
                        direct.breaker_specs_based_kw.value,
                        direct.consumption_based_kva.value,
                        direct.breaker_specs_based_kva.value,
                    ),
                    "",
                ));
            }
            Some(clamped) => {
                out.push(wrap(
                    "Some group was reported with more concurrent sessions than a single panel \
                     can run, so two sets are given. Either a second panel is installed, or the \
                     report is wrong.",
                    "",
                ));
                out.push(String::new());
                out.push("\"Direct\" - the groups exactly as reported:".to_owned());
                out.push(String::new());
                out.push(table(
                    &ESTIMATE_HEADERS,
                    &estimate_rows(direct),
                    &ESTIMATE_ALIGN,
                ));
                out.push(String::new());
                out.push(format!(
                    "\"Clamped\" - assuming one panel, capped at {} concurrent sessions:",
                    crate::EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS
                ));
                out.push(String::new());
                out.push(table(
                    &ESTIMATE_HEADERS,
                    &estimate_rows(clamped),
                    &ESTIMATE_ALIGN,
                ));
                out.push(String::new());
                out.push(wrap(
                    &format!(
                        "The likely kW values are in the range from {:.3} kW (\"Clamped\", \
                         consumption-based) to {:.3} kW (\"Direct\", breaker-spec-based). The \
                         likely kVA values are in the range from {:.3} kVA (\"Clamped\", \
                         consumption-based) to {:.3} kVA (\"Direct\", breaker-spec-based).",
                        clamped.consumption_based_kw.value,
                        direct.breaker_specs_based_kw.value,
                        clamped.consumption_based_kva.value,
                        direct.breaker_specs_based_kva.value,
                    ),
                    "",
                ));
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

        let any_flagged = self.session_groups.iter().any(|g| !g.anomalies().is_empty());
        let rows: Vec<Vec<String>> = self
            .session_groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                vec![
                    group_number(i, !g.anomalies().is_empty(), any_flagged),
                    hms(g.start()),
                    hms(g.end()),
                    mmss(g.duration()),
                    g.size().to_string(),
                    format!("{:.3}", g.agg_avg_power()),
                ]
            })
            .collect();
        out.push(table(
            &["#", "From", "To", "Length", "Count", "kW"],
            &rows,
            &[Right, Left, Left, Right, Right, Right],
        ));
        out.push(String::new());
        // Deliberately not "the lengths sum to the interval": groups tile only the part of it that
        // has a session in it, so they sum to the interval exactly just when some session spans
        // the whole window. What always holds is that they neither overlap nor double-count.
        out.push(wrap(
            "Times are local (ET). Groups are half-open: each runs from its From up to but not \
             including its To, so no instant falls in two groups and no session is counted twice.",
            "",
        ));

        if any_flagged {
            out.push(String::new());
            out.push(wrap(
                "An asterisk marks a group holding more sessions than a single panel can run at \
                 once. \"Clamped\" estimates were computed over a subset of them; the figures are \
                 under Anomalies",
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

        let group_findings: Vec<(usize, &SessionGroup, GroupAnomaly)> = self
            .session_groups
            .iter()
            .enumerate()
            .flat_map(|(i, g)| g.anomalies().into_iter().map(move |a| (i, g.as_ref(), a)))
            .collect();

        if self.session_anomalies.is_empty() && group_findings.is_empty() {
            out.push(wrap(
                "None. Every session considered for this interval was well formed.",
                "",
            ));
            out.push(String::new());
            return;
        }

        if !self.session_anomalies.is_empty() {
            let rows: Vec<Vec<String>> = self
                .session_anomalies
                .iter()
                .map(|a| {
                    vec![
                        a.row.to_string(),
                        a.session_id.clone(),
                        a.kind.as_str().to_owned(),
                        if a.kind.excludes_from_estimates() {
                            "yes"
                        } else {
                            "no"
                        }
                        .to_owned(),
                    ]
                })
                .collect();
            out.push(table(
                &["Row", "Session", "Anomaly", "Excluded"],
                &rows,
                &[Right, Left, Left, Left],
            ));
            out.push(String::new());
            out.push(wrap(
                "Row numbers are workbook rows, so each one can be looked up directly.",
                "",
            ));
            out.push(String::new());

            // The prose comes from each kind's `Display`, so there is one wording to maintain
            // rather than a second copy here that could drift from it. Only kinds actually present
            // are explained.
            let mut seen: Vec<AnomalyKind> = Vec::new();
            for a in &self.session_anomalies {
                if !seen.contains(&a.kind) {
                    seen.push(a.kind);
                }
            }
            for kind in seen {
                out.push(wrap(&format!("- {} - {}.", kind.as_str(), kind), "  "));
            }
        }

        if !group_findings.is_empty() {
            if self.session_anomalies.is_empty() {
                // Otherwise the section would open straight into a table with nothing saying what
                // it is or why the per-session table is missing.
                let n = group_findings.len();
                out.push(wrap(
                    &format!(
                        "No session was anomalous, but {}:",
                        if n == 1 {
                            "one group was".to_owned()
                        } else {
                            format!("{n} groups were")
                        }
                    ),
                    "",
                ));
                out.push(String::new());
            } else {
                out.push(String::new());
            }
            let rows: Vec<Vec<String>> = group_findings
                .iter()
                .map(|(i, g, a)| {
                    vec![
                        i.to_string(),
                        hms(g.start()),
                        hms(g.end()),
                        g.size().to_string(),
                        g.clamped_size().to_string(),
                        a.as_str().to_owned(),
                    ]
                })
                .collect();
            out.push(table(
                &["Group", "From", "To", "Reported", "Included", "Anomaly"],
                &rows,
                &[Right, Left, Left, Right, Right, Left],
            ));
            out.push(String::new());
            let mut seen: Vec<&'static str> = Vec::new();
            for (_, _, a) in &group_findings {
                if !seen.contains(&a.as_str()) {
                    seen.push(a.as_str());
                    out.push(wrap(&format!("- {} - {}.", a.as_str(), a), "  "));
                }
            }
        }
        out.push(String::new());
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
    use crate::{Anomaly, RSession, Session, groups_for_interval, max_power_estimates_for_interval};
    use std::{cell::RefCell, path::PathBuf, rc::Rc};

    const LO: &str = "2026-06-01T20:00:00Z";
    const HI: &str = "2026-06-01T21:00:00Z";

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn rsession(id: &str, start: &str, end: &str, avg_power: f64) -> RSession {
        let conn_start = ts(start);
        let conn_end = ts(end);
        Rc::new(RefCell::new(Session {
            id: id.to_owned(),
            row: 2,
            conn_start,
            raw_conn_end: conn_end,
            conn_end,
            conn_duration: conn_end.duration_since(conn_start).unsigned_abs(),
            charge_time: Duration::from_secs(60),
            energy_use: 1.0,
            avg_power,
            anomalies: Vec::new(),
        }))
    }

    /// Builds a report over `sessions` without touching the filesystem.
    fn report_of(sessions: Vec<RSession>, anomalies: Vec<Anomaly>) -> PowerEstimatesReport {
        let mut sessions = sessions;
        let groups = groups_for_interval((ts(LO), ts(HI)), &mut sessions);
        PowerEstimatesReport {
            source: PathBuf::from("/tmp/Session_Report_Test.xlsx"),
            interval: (ts(LO), ts(HI)),
            estimates: crate::peak_est::estimates_for_groups(&groups),
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
        assert!(!md.contains("**"), "bold markers render as noise in plain text");
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
        assert!(md.contains("Interval   2026-06-01 16:00 - 17:00 ET  (1 hour)"));
        // Three sessions at 1 kW: consumption 3.000, breaker 3 x 6.7.
        assert!(md.contains("| Consumption-based  |  3.000 |"), "{md}");
        assert!(md.contains("| Breaker-spec-based | 20.100 |"), "{md}");
        assert!(md.contains("- Group 0 - S00, S01, S02"), "{md}");
        assert!(md.contains("None. Every session considered"), "{md}");
        // Nothing clamped, so no marker slot and no second estimate table.
        assert!(md.contains("| # |"), "{md}");
        assert!(!md.contains("\"Clamped\""), "{md}");
    }

    /// The clamped path, which no fixture or real report reaches.
    #[test]
    fn oversized_group_renders_both_estimate_sets_and_marks_the_group() {
        let report = report_of(concurrent(12), Vec::new());
        let md = report.to_markdown();
        assert_plain_text_safe(&md);

        assert!(md.contains("\"Direct\" - the groups exactly as reported:"), "{md}");
        assert!(
            md.contains("\"Clamped\" - assuming one panel, capped at 10 concurrent sessions:"),
            "{md}"
        );
        // Direct sums all 12; clamped sums 10.
        assert!(md.contains("| Consumption-based  | 12.000 |"), "{md}");
        assert!(md.contains("| Consumption-based  | 10.000 |"), "{md}");
        // The group carries the marker, and the footnote explaining it appears.
        assert!(md.contains("| 0* |"), "{md}");
        assert!(md.contains("An asterisk marks a group"), "{md}");
        assert!(md.contains("ClampedSessionGroup"), "{md}");
        assert!(md.contains("| Group | From"), "{md}");
    }

    /// Every anomaly kind present gets a table row and exactly one legend entry, and the Excluded
    /// column follows `excludes_from_estimates`.
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

        // Exact rows, so the padding is pinned too: InconsistentDuration excludes the session,
        // ZeroActiveChargeTime does not.
        assert!(
            md.contains("|  47 | S31882  | InconsistentDuration | yes      |"),
            "{md}"
        );
        assert!(
            md.contains("| 152 | S70933  | ZeroActiveChargeTime | no       |"),
            "{md}"
        );
        // Two rows share a kind, but it is explained once.
        assert_eq!(md.matches("- InconsistentDuration - ").count(), 1, "{md}");
        assert_eq!(md.matches("- ZeroActiveChargeTime - ").count(), 1, "{md}");
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
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Session_Report_Diagram.csv"),
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

        assert!(md.contains("Source     Session_Report_Diagram.xlsx"), "{md}");
        assert!(md.contains("Interval   2026-06-15 16:00 - 17:00 ET  (1 hour)"), "{md}");
        // The diagram's peak: five sessions, 31.4 kW, in group 5.
        assert!(md.contains("| Consumption-based  | 31.400 | 33.053 |     5 |"), "{md}");
        assert!(md.contains("| Breaker-spec-based | 33.500 | 37.500 |     5 |"), "{md}");
        assert!(md.contains("| 5 | 16:34:00 | 16:35:00 |   1:00 |     5 | 31.400 |"), "{md}");
        assert!(md.contains("- Group 5 - A, C, D, E, F"), "{md}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
