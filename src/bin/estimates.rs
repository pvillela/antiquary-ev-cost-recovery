use ev_peak_contrib::{TIME_ZONE_NAME, max_power_estimates_for_interval};
use jiff::{Timestamp, civil, tz::TimeZone};
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
Estimates the EV charging contribution to peak demand over an interval of interest.

Usage: estimates <SESSION_REPORT.xlsx> <YYYY-MM-DD HH:MM> [15m|1h]

The interval start is given in local time (ET), because that is what Toronto Hydro's metering data
is stated in. Length defaults to 1h when the start is on the hour and 15m otherwise.

  estimates June.xlsx \"2026-06-01 16:00\" 1h
  estimates June.xlsx \"2026-06-01 16:45\"

Per README.md the interval is constrained: it must start at HH:00, HH:15, HH:30 or HH:45, and may
run for one hour only from HH:00. An interval breaking those rules is rejected rather than
estimated, since the figures would be compared against a real bill.

The report is written to stdout as markdown that also reads as plain text.";

/// The four legal start minutes. See README.md, \"Interval of interest boundaries\".
const LEGAL_START_MINUTES: [i8; 4] = [0, 15, 30, 45];

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
        eprintln!("expected 2 or 3 arguments, got {}\n\n{USAGE}", args.len());
        return ExitCode::FAILURE;
    }

    let path = PathBuf::from(&args[0]);
    let interval = match parse_interval(&args[1], args.get(2).map(String::as_str)) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    match max_power_estimates_for_interval(interval, &path) {
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

/// Parses the local start and optional length into a UTC interval, enforcing README's boundary
/// rules.
///
/// The rules are checked here rather than in the library, which stays permissive: `groups_for_
/// interval` is happy with any interval, and exploratory callers and tests rely on that. What must
/// not happen is a *bill* being argued from an off-spec window, and that only goes through here.
fn parse_interval(start: &str, length: Option<&str>) -> Result<(Timestamp, Timestamp), String> {
    let start_local: civil::DateTime = start
        .trim()
        .replace('T', " ")
        .parse()
        .map_err(|e| format!("cannot read \"{start}\" as YYYY-MM-DD HH:MM: {e}"))?;

    if start_local.second() != 0 || start_local.subsec_nanosecond() != 0 {
        return Err(format!(
            "interval start {start_local} carries seconds; it must be a whole minute"
        ));
    }
    if !LEGAL_START_MINUTES.contains(&start_local.minute()) {
        return Err(format!(
            "interval start {start_local} is not on a quarter hour; it must be HH:00, HH:15, \
             HH:30 or HH:45. See README.md, \"Interval of interest boundaries\"."
        ));
    }

    let minutes = match length {
        None => {
            if start_local.minute() == 0 {
                60
            } else {
                15
            }
        }
        Some("1h") => 60,
        Some("15m") => 15,
        Some(other) => return Err(format!("unknown length \"{other}\"; expected 15m or 1h")),
    };
    if minutes == 60 && start_local.minute() != 0 {
        return Err(format!(
            "an interval of 1 hour must start at HH:00, but {start_local} starts at :{:02}. See \
             README.md, \"Interval of interest boundaries\".",
            start_local.minute()
        ));
    }

    let tz = TimeZone::get(TIME_ZONE_NAME).map_err(|e| e.to_string())?;
    // A start in the DST gap never occurred, and a start in the fold is two different instants an
    // hour apart. Either would silently estimate a window other than the one asked for, so both
    // are refused. `to_zoned` would not do: its default disambiguation resolves a gap forward and
    // a fold to the earlier offset, without saying so.
    let zoned = tz
        .to_ambiguous_zoned(start_local)
        .unambiguous()
        .map_err(|e| format!("{start_local} is not one unambiguous {TIME_ZONE_NAME} time: {e}"))?;
    let lo = zoned.timestamp();
    let hi = lo + jiff::SignedDuration::from_mins(minutes);
    Ok((lo, hi))
}

#[cfg(test)]
mod test {
    use super::*;

    fn utc(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn legal_intervals_parse_to_utc() {
        // 16:00 EDT is 20:00Z in June.
        assert_eq!(
            parse_interval("2026-06-01 16:00", Some("1h")).unwrap(),
            (utc("2026-06-01T20:00:00Z"), utc("2026-06-01T21:00:00Z"))
        );
        assert_eq!(
            parse_interval("2026-06-01 16:45", Some("15m")).unwrap(),
            (utc("2026-06-01T20:45:00Z"), utc("2026-06-01T21:00:00Z"))
        );
    }

    /// Length defaults to the only one the start permits.
    #[test]
    fn length_defaults_by_start_minute() {
        assert_eq!(
            parse_interval("2026-06-01 16:00", None).unwrap(),
            parse_interval("2026-06-01 16:00", Some("1h")).unwrap()
        );
        assert_eq!(
            parse_interval("2026-06-01 16:15", None).unwrap(),
            parse_interval("2026-06-01 16:15", Some("15m")).unwrap()
        );
    }

    #[test]
    fn off_spec_intervals_are_rejected() {
        // Not on a quarter hour.
        assert!(parse_interval("2026-06-01 16:07", None).is_err());
        // An hour must start on the hour.
        assert!(parse_interval("2026-06-01 16:15", Some("1h")).is_err());
        // Unknown length.
        assert!(parse_interval("2026-06-01 16:00", Some("30m")).is_err());
        // Unparseable.
        assert!(parse_interval("yesterday", None).is_err());
    }

    /// 2026-03-08 02:00 local never happened: DST skips from 02:00 to 03:00. Resolving it forward
    /// would silently estimate a different hour than the one asked for.
    #[test]
    fn a_start_in_the_dst_gap_is_refused() {
        assert!(parse_interval("2026-03-08 02:00", Some("1h")).is_err());
    }
}
