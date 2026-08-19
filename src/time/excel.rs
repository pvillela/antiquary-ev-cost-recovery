//! Excel serial-date arithmetic, shared by both sheet writers.
//!
//! Excel has no concept of a time zone. A serial is a count of days since day zero, and a local
//! column and a UTC column differ only in which instant was converted; they are told apart by
//! their number format, not by anything stored in the cell.
//!
//! The names say which input a function takes, because that is where the two modules previously
//! disagreed: both had a function called `excel_serial`, one taking a [`Timestamp`] and one taking
//! a [`civil::DateTime`], with different meanings. Keeping either name would have silently changed
//! behaviour on one side, so neither survives.
//!
//! All of these are infallible. The fallible forms could only fail on a civil date-time that UTC
//! cannot represent, and UTC has no gaps or folds, so the error was unreachable.

use jiff::{
    Timestamp,
    civil::{Date, DateTime},
    tz::TimeZone,
};
use std::time::Duration;

use super::local_datetime;

/// Excel's day zero for the 1900 date system, as a Unix timestamp: 1899-12-30T00:00:00Z.
/// Verified by [`test::the_epoch_constant_matches_jiff`].
const EXCEL_EPOCH_UNIX_SECS: i64 = -2_209_161_600;

const SECS_PER_DAY: f64 = 86_400.0;

/// The serial for an instant, in whichever offset the caller has already applied.
pub fn serial_of_instant(ts: Timestamp) -> f64 {
    (ts.as_second() - EXCEL_EPOCH_UNIX_SECS) as f64 / SECS_PER_DAY
}

/// The serial for a civil date-time read exactly as written, with no offset applied.
pub fn serial_of_civil(dt: DateTime) -> f64 {
    serial_of_instant(wall_clock_instant(dt))
}

/// The serial for a bare date, for the date-only columns.
pub fn serial_of_date(d: Date) -> f64 {
    serial_of_civil(d.at(0, 0, 0, 0))
}

/// The serial showing an instant's **local** wall clock, for the local-time columns.
pub fn serial_of_local(ts: Timestamp) -> f64 {
    serial_of_civil(local_datetime(ts))
}

/// Excel stores a duration as a fraction of a day.
pub fn serial_of_duration(d: Duration) -> f64 {
    d.as_secs() as f64 / SECS_PER_DAY
}

/// Inverse of [`serial_of_instant`].
///
/// Rounds to the nearest second rather than truncating: the writer stores whole seconds, and
/// truncating what floating point hands back would turn `20:22:00` into `20:21:59`.
pub fn instant_of_serial(serial: f64) -> Result<Timestamp, jiff::Error> {
    Timestamp::from_second((serial * SECS_PER_DAY).round() as i64 + EXCEL_EPOCH_UNIX_SECS)
}

/// Inverse of [`serial_of_duration`], rounded to the nearest second for the same reason.
///
/// A negative serial clamps to zero. Excel has no unsigned cell, so a corrupt or hand-edited
/// duration cell can be negative, and no duration in this crate can be.
pub fn duration_of_serial(days: f64) -> Duration {
    Duration::from_secs((days * SECS_PER_DAY).round().max(0.0) as u64)
}

/// Reads a local wall time as though it were UTC, so that two of them can be subtracted to give the
/// wall-clock distance between them.
///
/// Not a time-zone conversion: the point is to compare wall times as written, without a DST offset
/// moving either one. It lives here because it is also how every serial is computed — a serial is a
/// wall-clock reading, whatever zone produced it.
pub fn wall_clock_instant(dt: DateTime) -> Timestamp {
    dt.to_zoned(TimeZone::UTC)
        .expect("UTC has no gaps or folds, so a civil date-time is never ambiguous in it")
        .timestamp()
}

// cargo test --lib -- time::excel::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;

    /// Pins [`EXCEL_EPOCH_UNIX_SECS`] to the date it claims to be, so the constant cannot drift
    /// from its comment.
    #[test]
    fn the_epoch_constant_matches_jiff() {
        assert_eq!(
            wall_clock_instant(date(1899, 12, 30).at(0, 0, 0, 0)).as_second(),
            EXCEL_EPOCH_UNIX_SECS
        );
    }

    /// Excel's own serial for 2026-06-23 is 46196; this checks the arithmetic against a value that
    /// can be reproduced by typing the date into a spreadsheet.
    #[test]
    fn a_known_date_converts_to_its_excel_serial() {
        assert_eq!(serial_of_date(date(2026, 6, 23)), 46196.0);
        assert_eq!(serial_of_civil(date(2026, 6, 23).at(0, 0, 0, 0)), 46196.0);
        assert_eq!(
            serial_of_instant(wall_clock_instant(date(2026, 6, 23).at(0, 0, 0, 0))),
            46196.0
        );
    }

    /// Midday is half a day past the date's serial, which is what makes a serial a date *and* a
    /// time in one number.
    #[test]
    fn the_time_of_day_is_the_fractional_part() {
        assert_eq!(serial_of_civil(date(2026, 6, 23).at(12, 0, 0, 0)), 46196.5);
    }

    /// The local form reads the wall clock, so its distance from the UTC form is the zone's offset
    /// — four hours in summer, five in winter.
    ///
    /// Stated as a difference rather than as a fractional part. A serial is a number around 46,000
    /// and `% 1.0` on one keeps only the bits left after that magnitude, which is not enough to
    /// compare an hour of a day to any useful tolerance.
    #[test]
    fn the_local_form_follows_the_wall_clock_across_dst() {
        for (s, offset_hours) in [
            ("2026-06-23T16:00:00Z", -4.0),
            ("2026-12-23T16:00:00Z", -5.0),
        ] {
            let ts: Timestamp = s.parse().unwrap();
            let offset_days = serial_of_local(ts) - serial_of_instant(ts);
            assert!(
                (offset_days - offset_hours / 24.0).abs() < 1e-9,
                "{s}: offset came to {} hours",
                offset_days * 24.0
            );
        }
    }

    #[test]
    fn a_duration_is_a_fraction_of_a_day() {
        assert_eq!(serial_of_duration(Duration::from_secs(43_200)), 0.5);
        assert_eq!(serial_of_duration(Duration::ZERO), 0.0);
    }

    /// Both round trips hold to the second, which is the resolution the writer stores.
    #[test]
    fn the_inverses_round_trip_to_the_second() {
        for s in [
            "2026-06-15T20:22:00Z",
            "2026-11-01T05:30:37Z",
            "1970-01-01T00:00:00Z",
        ] {
            let ts: Timestamp = s.parse().unwrap();
            assert_eq!(instant_of_serial(serial_of_instant(ts)).unwrap(), ts, "{s}");
        }
        for secs in [0u64, 1, 59, 3600, 86_399, 400_000] {
            let d = Duration::from_secs(secs);
            assert_eq!(duration_of_serial(serial_of_duration(d)), d, "{secs}");
        }
    }

    /// A negative duration serial clamps rather than wrapping. Excel has no unsigned cell.
    #[test]
    fn a_negative_duration_serial_clamps_to_zero() {
        assert_eq!(duration_of_serial(-0.5), Duration::ZERO);
    }
}
