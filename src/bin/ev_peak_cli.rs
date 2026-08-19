use ev_cost_recovery::{
    sessions::{IoiLength, checked_interval, interval_estimates},
    time::Interval,
    time::TZ_OFFSETS,
};
use jiff::civil;
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "\
Estimates the EV charging contribution to peak power demand over an interval of interest.

Usage: ev_estimate_cli <SESSION_REPORT.xlsx> <YYYY-MM-DD HH:MM [EST|EDT]> [15m|1h]

The interval start is given in local time (ET), because that is what Toronto Hydro's metering data
is stated in. Length defaults to 1h when the start is on the hour and 15m otherwise.

  ev_estimate_cli June.xlsx \"2026-06-01 16:00\" 1h
  ev_estimate_cli June.xlsx \"2026-06-01 16:45\"
  ev_estimate_cli Nov.xlsx  \"2026-11-01 01:30 EDT\" 15m

On the night DST ends, one hour of wall time occurs twice; add EST or EDT to say which is meant.
The designator is accepted at any time and checked against the date, so a wrong one is an error
rather than a figure for the wrong hour. A time in the DST gap is rejected outright: it never
occurred, so there is nothing to choose between.

Per README.md the interval is constrained: it must start at HH:00, HH:15, HH:30 or HH:45, and may
run for one hour only from HH:00. An interval breaking those rules is rejected rather than
estimated, since the figures would be compared against a real bill.

The report is written to stdout as markdown that also reads as plain text.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return if args.is_empty() {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }
    if args.len() < 2 || args.len() > 3 {
        eprintln!(
            "expected 2 or 3 arguments - a workbook, an interval start, and optionally a length - \
             but got {}\n\n{USAGE}",
            args.len()
        );
        return ExitCode::FAILURE;
    }

    let path = PathBuf::from(&args[0]);
    match workbook_fault(&path) {
        // The arguments shifted, so the usage is the thing to show.
        Some(PathFault::NotAWorkbook(msg)) => {
            eprintln!("{msg}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
        // A workbook named but not found is an ordinary typo; the usage would not help.
        Some(PathFault::Missing(msg)) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
        None => {}
    }

    let interval = match parse_interval(&args[1], args.get(2).map(String::as_str)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    match interval_estimates(interval, &path) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// Why the first argument cannot be the session report.
enum PathFault {
    /// Not a workbook name at all. Most likely the arguments shifted.
    NotAWorkbook(String),
    /// A workbook name that is not there.
    Missing(String),
}

/// Checks the first argument before the interval is parsed, or `None` when it looks like a workbook
/// that exists.
///
/// Worth doing separately because the arguments shift *silently* when the workbook is omitted: the
/// length is optional, so `estimates "2026-06-01 16:00" 1h` is a legal two-argument call in which
/// the start time is read as the path and the length as the start time. Left to `parse_interval`,
/// that comes out as a complaint about `1h` — a message about the argument that is present rather
/// than the one that is missing.
fn workbook_fault(path: &Path) -> Option<PathFault> {
    if path
        .extension()
        .is_none_or(|e| !e.eq_ignore_ascii_case("xlsx"))
    {
        return Some(PathFault::NotAWorkbook(format!(
            "first argument \"{}\" is not a .xlsx workbook - the session report comes first, then \
             the interval start",
            path.display()
        )));
    }
    if !path.is_file() {
        return Some(PathFault::Missing(format!(
            "no such workbook: {}",
            path.display()
        )));
    }
    None
}

/// Parses the local start and optional length into a UTC interval.
///
/// Only the *reading* of the arguments happens here. What makes an interval legal — the start
/// minute rule, the length rule, and what a wall time means on the two nights the clocks change —
/// lives in [`checked_interval`], so that this and the GUI cannot drift apart on a question a bill
/// will be argued from.
fn parse_interval(start: &str, length: Option<&str>) -> Result<Interval, String> {
    let (stamp, designator) = split_designator(start.trim());
    let start_local: civil::DateTime = stamp
        .replace('T', " ")
        .parse()
        .map_err(|e| format!("cannot read \"{stamp}\" as YYYY-MM-DD HH:MM: {e}"))?;

    let length = match length {
        None => None,
        Some("1h") => Some(IoiLength::Hour),
        Some("15m") => Some(IoiLength::FifteenMinutes),
        Some(other) => return Err(format!("unknown length \"{other}\"; expected 15m or 1h")),
    };

    checked_interval(start_local, length, designator)
}

/// Splits an optional trailing `EST`/`EDT` off the timestamp argument.
///
/// Safe to do by looking at the last whitespace-separated token: a bare `YYYY-MM-DD HH:MM` ends in
/// the time, which never spells either name.
fn split_designator(s: &str) -> (&str, Option<&str>) {
    match s.rsplit_once(char::is_whitespace) {
        Some((head, tail))
            if TZ_OFFSETS
                .iter()
                .any(|(name, _)| tail.eq_ignore_ascii_case(name)) =>
        {
            (head.trim_end(), Some(tail))
        }
        _ => (s, None),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn utc(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    fn itvl(lo: &str, hi: &str) -> Interval {
        Interval::from_start_end(utc(lo), utc(hi))
    }

    /// The first argument is checked before the interval is, so omitting the workbook is reported as
    /// the missing workbook rather than as an unreadable timestamp.
    ///
    /// `ev_estimate_cli "2026-06-01 16:00" 1h` is what prompted this: two arguments is a legal shape,
    /// since the length is optional, so the start was read as the path and `1h` as the start.
    #[test]
    fn a_first_argument_that_is_not_a_workbook_is_caught_before_the_interval() {
        assert!(
            matches!(
                workbook_fault(Path::new("2026-06-01 16:00")),
                Some(PathFault::NotAWorkbook(_))
            ),
            "a shifted first argument should be reported as such"
        );
        // Named as a workbook, so the arguments are in the right order; it is simply not there.
        assert!(matches!(
            workbook_fault(Path::new("no_such_file_here.xlsx")),
            Some(PathFault::Missing(_))
        ));
        // A real workbook is a real workbook whatever the case of its extension.
        assert!(matches!(
            workbook_fault(Path::new("Nope.XLSX")),
            Some(PathFault::Missing(_))
        ));
        // Cargo.toml exists but is not a workbook, so the extension is what decides.
        assert!(matches!(
            workbook_fault(Path::new("Cargo.toml")),
            Some(PathFault::NotAWorkbook(_))
        ));
    }

    /// The text layer's own job: read the arguments and hand them on. What a legal interval *is*
    /// is tested where it is decided, in the library's `interval` module.
    #[test]
    fn arguments_are_read_and_handed_on() {
        // 16:00 EDT is 20:00Z in June.
        assert_eq!(
            parse_interval("2026-06-01 16:00", Some("1h")).unwrap(),
            itvl("2026-06-01T20:00:00Z", "2026-06-01T21:00:00Z")
        );
        assert_eq!(
            parse_interval("2026-06-01 16:45", Some("15m")).unwrap(),
            itvl("2026-06-01T20:45:00Z", "2026-06-01T21:00:00Z")
        );
        // An omitted length is the library's default, not this layer's guess.
        assert_eq!(
            parse_interval("2026-06-01 16:00", None).unwrap(),
            parse_interval("2026-06-01 16:00", Some("1h")).unwrap()
        );
        // A designator is carried through rather than dropped.
        assert_eq!(
            parse_interval("2026-11-01 01:00 EST", Some("1h"))
                .unwrap()
                .start,
            utc("2026-11-01T06:00:00Z")
        );
    }

    /// Text that is not an interval at all, which is this layer's to complain about.
    #[test]
    fn unreadable_arguments_are_rejected() {
        assert!(parse_interval("yesterday", None).is_err());
        let msg = parse_interval("2026-06-01 16:00", Some("30m")).unwrap_err();
        assert!(msg.contains("unknown length"), "{msg}");
    }

    /// Rules the library owns still reach the caller through this layer.
    #[test]
    fn library_rules_still_apply() {
        // Not on a quarter hour.
        assert!(parse_interval("2026-06-01 16:07", None).is_err());
        // An hour must start on the hour.
        assert!(parse_interval("2026-06-01 16:15", Some("1h")).is_err());
        // A wall time that occurs twice is refused until it is said which is meant.
        assert!(parse_interval("2026-11-01 01:00", Some("1h")).is_err());
        // A wall time that never occurred is refused outright.
        assert!(parse_interval("2026-03-08 02:00", Some("1h")).is_err());
    }

    /// Case does not matter, and a bare timestamp is never mistaken for carrying one.
    #[test]
    fn designator_splitting_is_unambiguous() {
        assert_eq!(
            split_designator("2026-11-01 01:00"),
            ("2026-11-01 01:00", None)
        );
        assert_eq!(
            split_designator("2026-11-01 01:00 edt"),
            ("2026-11-01 01:00", Some("edt"))
        );
        assert_eq!(
            parse_interval("2026-11-01 01:00 edt", Some("1h")).unwrap(),
            parse_interval("2026-11-01 01:00 EDT", Some("1h")).unwrap()
        );
    }
}
