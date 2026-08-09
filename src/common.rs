//! Domain types shared across the crate, and the Excel serial-date arithmetic every sheet writer
//! needs.
//!
//! The readings carry raw source integers rather than kilowatt figures. Green Button reports each
//! value as an integer with a `powerOfTenMultiplier`, and every sum and maximum here runs on those
//! integers; the division happens once, at cell-write time. That is not premature caution — the
//! spreadsheet is reconciled against a utility invoice to three decimal places, and accumulating
//! 744 floating-point divisions before summing them loses that agreement.

use std::fmt;
use std::sync::LazyLock;

use ev_peak_contrib::TIME_ZONE_NAME;
use jiff::{Timestamp, civil::Date, civil::DateTime, tz::TimeZone};

/// Resolved once. Every local-time question in the crate goes through here, so there is one answer
/// to "which zone" rather than one per module.
pub(crate) static TIME_ZONE: LazyLock<TimeZone> = LazyLock::new(|| {
    TimeZone::get(TIME_ZONE_NAME).expect("America/Toronto should be a valid time-zone name")
});

/// The local calendar date an instant falls on.
pub(crate) fn local_date(ts: Timestamp) -> Date {
    ts.to_zoned(TIME_ZONE.clone()).date()
}

/// The local wall-clock reading of an instant, for the workbook's local-time columns.
pub(crate) fn local_datetime(ts: Timestamp) -> DateTime {
    ts.to_zoned(TIME_ZONE.clone()).datetime()
}

/// The instant a given local hour begins on a given local date.
///
/// # Panics
///
/// Panics if the local time falls in a daylight-saving gap or fold. Callers pass 0, 7, 11, 17 or
/// 19; Ontario's transitions are at 02:00, so none of them can.
pub(crate) fn local_hour(d: Date, hour: u8) -> Timestamp {
    d.at(hour as i8, 0, 0, 0)
        .to_zoned(TIME_ZONE.clone())
        .expect("callers pass hours that never fall in a daylight-saving transition")
        .timestamp()
}

/// The instant a local date begins.
pub(crate) fn local_midnight(d: Date) -> Timestamp {
    local_hour(d, 0)
}

/// Excel's day zero for the 1900 date system, as a Unix timestamp: 1899-12-30T00:00:00Z.
/// Verified by [`test::excel_epoch_matches_jiff`].
const EXCEL_EPOCH_UNIX_SECS: i64 = -2_209_161_600;

const SECS_PER_DAY: f64 = 86_400.0;

/// One hour of metered data, keyed on the instant the hour starts.
///
/// The three values are independent `Option`s rather than a single "reading is present" flag
/// because the feed can and does carry a timestamp in one series and not another. The Python this
/// replaces substituted zero for a missing companion, which cannot raise a maximum but does write
/// a false `0.000` into the "kVA at interval" columns — a silent wrong number in a cell used to
/// check a bill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub start: Timestamp,
    pub kwh: Option<i64>,
    pub kw: Option<i64>,
    pub kva: Option<i64>,
}

impl Reading {
    /// True when no series carried a value at this timestamp, i.e. the hour is a hole that the
    /// surrounding data implies should exist.
    pub fn is_empty(&self) -> bool {
        self.kwh.is_none() && self.kw.is_none() && self.kva.is_none()
    }

    /// Whether the interval starts on a whole hour.
    ///
    /// Only aligned intervals are eligible to be a reported peak, which is what lets the TOU
    /// column always hold one value: Ontario's price-period boundaries are all on the hour, so an
    /// aligned hour cannot straddle two. Ontario's UTC offsets are whole hours in both seasons, so
    /// a whole hour in UTC is a whole hour locally.
    pub fn is_aligned(&self) -> bool {
        self.start.as_second().rem_euclid(3600) == 0
    }
}

/// A row or period that needs review. Never fatal: the workbook is still written and the figures
/// are still produced.
///
/// The `as_str` tokens are a **stable wire format**. These sheets are meant to be read back by
/// column name, so renaming a variant silently invalidates every workbook already written. Add
/// variants freely; never rename one.
///
/// There is deliberately no DST variant. The feed timestamps every reading as an absolute UTC
/// epoch on a fixed grid, so neither the spring-forward gap nor the fall-back fold can produce an
/// ambiguous or missing record — they are a rendering concern in the local-time column and nothing
/// more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Anomaly {
    /// The hour carried a kW or kVA value but no kWh.
    MissingKwh,
    /// The hour carried a kWh or kVA value but no kW.
    MissingKw,
    /// The hour carried a kWh or kW value but no kVA.
    MissingKva,
    /// No series carried this hour, but the hours around it imply it should exist.
    MissingInterval,
    /// The same interval start appeared more than once within one series.
    DuplicateInterval,
    /// The interval does not start on a whole hour. Excluded from peak selection, so it can never
    /// become a reported maximum.
    MisalignedInterval,
}

impl Anomaly {
    /// The stable token written to the `anomalies` column. See the type-level note.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MissingKwh => "MissingKwh",
            Self::MissingKw => "MissingKw",
            Self::MissingKva => "MissingKva",
            Self::MissingInterval => "MissingInterval",
            Self::DuplicateInterval => "DuplicateInterval",
            Self::MisalignedInterval => "MisalignedInterval",
        }
    }

    /// Inverse of [`Anomaly::as_str`], for reading a workbook back.
    pub fn from_token(s: &str) -> Option<Self> {
        Some(match s {
            "MissingKwh" => Self::MissingKwh,
            "MissingKw" => Self::MissingKw,
            "MissingKva" => Self::MissingKva,
            "MissingInterval" => Self::MissingInterval,
            "DuplicateInterval" => Self::DuplicateInterval,
            "MisalignedInterval" => Self::MisalignedInterval,
            _ => return None,
        })
    }
}

impl fmt::Display for Anomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An Excel serial date for an instant, in whichever offset the caller has already applied.
///
/// Excel has no concept of a time zone, so a local-time column and a UTC column differ only by
/// which instant was handed to this function. Both are written as plain serials and told apart by
/// their number format.
pub fn excel_serial(ts: Timestamp) -> f64 {
    (ts.as_second() - EXCEL_EPOCH_UNIX_SECS) as f64 / SECS_PER_DAY
}

// cargo test --package green-button --lib -- common::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;
    use jiff::tz::TimeZone;

    /// Pins [`EXCEL_EPOCH_UNIX_SECS`] to the date it claims to be, so the constant cannot drift
    /// from its comment.
    #[test]
    fn excel_epoch_matches_jiff() {
        let epoch = date(1899, 12, 30)
            .at(0, 0, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp();
        assert_eq!(epoch.as_second(), EXCEL_EPOCH_UNIX_SECS);
    }

    /// Excel's own serial for 2026-06-23 is 46196; this checks the arithmetic against a value that
    /// can be reproduced by typing the date into a spreadsheet.
    #[test]
    fn a_known_date_converts_to_its_excel_serial() {
        let ts = date(2026, 6, 23)
            .at(0, 0, 0, 0)
            .to_zoned(TimeZone::UTC)
            .unwrap()
            .timestamp();
        assert_eq!(excel_serial(ts), 46196.0);
    }

    #[test]
    fn every_anomaly_token_round_trips() {
        for a in [
            Anomaly::MissingKwh,
            Anomaly::MissingKw,
            Anomaly::MissingKva,
            Anomaly::MissingInterval,
            Anomaly::DuplicateInterval,
            Anomaly::MisalignedInterval,
        ] {
            assert_eq!(Anomaly::from_token(a.as_str()), Some(a));
        }
    }
}
