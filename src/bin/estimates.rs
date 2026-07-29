use ev_peak_contrib::{TIME_ZONE_NAME, max_power_estimates_for_interval};
use jiff::{
    Timestamp, civil,
    tz::{Offset, TimeZone},
};
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
Estimates the EV charging contribution to peak demand over an interval of interest.

Usage: estimates <SESSION_REPORT.xlsx> <YYYY-MM-DD HH:MM [EST|EDT]> [15m|1h]

The interval start is given in local time (ET), because that is what Toronto Hydro's metering data
is stated in. Length defaults to 1h when the start is on the hour and 15m otherwise.

  estimates June.xlsx \"2026-06-01 16:00\" 1h
  estimates June.xlsx \"2026-06-01 16:45\"
  estimates Nov.xlsx  \"2026-11-01 01:30 EDT\" 15m

On the night DST ends, one hour of wall time occurs twice; add EST or EDT to say which is meant.
The designator is accepted at any time and checked against the date, so a wrong one is an error
rather than a figure for the wrong hour. A time in the DST gap is rejected outright: it never
occurred, so there is nothing to choose between.

Per README.md the interval is constrained: it must start at HH:00, HH:15, HH:30 or HH:45, and may
run for one hour only from HH:00. An interval breaking those rules is rejected rather than
estimated, since the figures would be compared against a real bill.

The report is written to stdout as markdown that also reads as plain text.";

/// The four legal start minutes. See README.md, \"Interval of interest boundaries\".
const LEGAL_START_MINUTES: [i8; 4] = [0, 15, 30, 45];

/// The offsets [`TIME_ZONE_NAME`] uses, under the names a reader of a Toronto Hydro bill will
/// recognise. Naming one resolves a wall time that occurs twice.
const OFFSETS: [(&str, i8); 2] = [("EST", -5), ("EDT", -4)];

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
    let (stamp, designator) = split_designator(start.trim());
    let start_local: civil::DateTime = stamp
        .replace('T', " ")
        .parse()
        .map_err(|e| format!("cannot read \"{stamp}\" as YYYY-MM-DD HH:MM: {e}"))?;

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

    let lo = resolve_local(start_local, designator)?;
    let hi = lo + jiff::SignedDuration::from_mins(minutes);
    Ok((lo, hi))
}

/// Splits an optional trailing `EST`/`EDT` off the timestamp argument.
///
/// Safe to do by looking at the last whitespace-separated token: a bare `YYYY-MM-DD HH:MM` ends in
/// the time, which never spells either name.
fn split_designator(s: &str) -> (&str, Option<&str>) {
    match s.rsplit_once(char::is_whitespace) {
        Some((head, tail))
            if OFFSETS
                .iter()
                .any(|(name, _)| tail.eq_ignore_ascii_case(name)) =>
        {
            (head.trim_end(), Some(tail))
        }
        _ => (s, None),
    }
}

/// Turns a local wall time into the instant it names, refusing rather than guessing when it names
/// none or two.
///
/// Every case falls out of one question asked per offset: read the wall time *as if* at that fixed
/// offset, and check the zone really is at that offset on the instant you land on. The number of
/// offsets that survive says which situation this is — one for an ordinary time, two on the night
/// DST ends, none in the spring gap — so gap and fold need no special-casing, and a designator can
/// be checked against the date rather than merely believed.
fn resolve_local(dt: civil::DateTime, designator: Option<&str>) -> Result<Timestamp, String> {
    let tz = TimeZone::get(TIME_ZONE_NAME).map_err(|e| e.to_string())?;
    let candidates: Vec<(&str, Timestamp)> = OFFSETS
        .iter()
        .filter_map(|(name, hours)| {
            let offset = Offset::constant(*hours);
            let ts = dt.to_zoned(TimeZone::fixed(offset)).ok()?.timestamp();
            (tz.to_offset(ts) == offset).then_some((*name, ts))
        })
        .collect();

    let names =
        |cs: &[(&str, Timestamp)]| cs.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" or ");

    match (designator, candidates.as_slice()) {
        (_, []) => Err(format!(
            "{dt} never occurred in {TIME_ZONE_NAME}: the clocks jump forward over it when DST \
             begins. Pick a time outside the skipped hour."
        )),
        (None, [(_, ts)]) => Ok(*ts),
        (None, several) => Err(format!(
            "{dt} occurs twice in {TIME_ZONE_NAME}, an hour apart, because DST ends that night. \
             Add {} to say which is meant.",
            names(several)
        )),
        (Some(want), cs) => cs
            .iter()
            .find(|(name, _)| want.eq_ignore_ascii_case(name))
            .map(|(_, ts)| *ts)
            .ok_or_else(|| {
                format!(
                    "{dt} is not {} in {TIME_ZONE_NAME}; that date is on {}.",
                    want.to_uppercase(),
                    names(cs)
                )
            }),
    }
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

    /// 2026-03-08 02:00 local never happened: DST skips from 02:00 to 03:00. Nothing can
    /// disambiguate a time that does not exist, so a designator does not help either.
    #[test]
    fn a_start_in_the_dst_gap_is_refused() {
        assert!(parse_interval("2026-03-08 02:00", Some("1h")).is_err());
        assert!(parse_interval("2026-03-08 02:00 EST", Some("1h")).is_err());
        assert!(parse_interval("2026-03-08 02:00 EDT", Some("1h")).is_err());
    }

    /// 2026-11-01 01:00 local happens twice, an hour apart, when DST ends. Bare it is refused; with
    /// a designator it resolves, and the two readings are exactly an hour apart.
    #[test]
    fn a_fold_start_needs_a_designator_and_then_resolves() {
        let bare = parse_interval("2026-11-01 01:00", Some("1h"));
        let msg = bare.unwrap_err();
        assert!(msg.contains("occurs twice"), "{msg}");
        assert!(msg.contains("EST") && msg.contains("EDT"), "{msg}");

        // 01:00 EDT is 05:00Z; 01:00 EST is 06:00Z.
        let (edt_lo, edt_hi) = parse_interval("2026-11-01 01:00 EDT", Some("1h")).unwrap();
        let (est_lo, est_hi) = parse_interval("2026-11-01 01:00 EST", Some("1h")).unwrap();
        assert_eq!(edt_lo, utc("2026-11-01T05:00:00Z"));
        assert_eq!(est_lo, utc("2026-11-01T06:00:00Z"));
        assert_eq!(est_lo.duration_since(edt_lo).as_secs(), 3600);

        // The EDT hour ends where the EST hour begins: the interval spans the fold.
        assert_eq!(edt_hi, est_lo);
        assert_eq!(est_hi, utc("2026-11-01T07:00:00Z"));
    }

    /// A designator is checked against the date rather than believed, so naming the wrong one is an
    /// error and not an estimate for the wrong hour. It is accepted, and redundant, the rest of the
    /// year.
    #[test]
    fn a_designator_is_validated_against_the_date() {
        // June is on EDT, so EDT is redundant but correct and EST is simply wrong.
        assert_eq!(
            parse_interval("2026-06-01 16:00 EDT", Some("1h")).unwrap(),
            parse_interval("2026-06-01 16:00", Some("1h")).unwrap()
        );
        let msg = parse_interval("2026-06-01 16:00 EST", Some("1h")).unwrap_err();
        assert!(msg.contains("is not EST"), "{msg}");
        assert!(msg.contains("EDT"), "{msg}");

        // January is on EST, and the mirror image holds.
        assert_eq!(
            parse_interval("2026-01-15 16:00 EST", Some("1h")).unwrap(),
            parse_interval("2026-01-15 16:00", Some("1h")).unwrap()
        );
        assert!(parse_interval("2026-01-15 16:00 EDT", Some("1h")).is_err());
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
