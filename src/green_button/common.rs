//! Domain types shared across the `green_button` module.
//!
//! The Excel serial-date arithmetic that used to live here is in [`crate::time::excel`], which both
//! sheet writers now share.
//!
//! The readings carry raw source integers rather than kilowatt figures. Green Button reports each
//! value as an integer with a `powerOfTenMultiplier`, and every sum and maximum here runs on those
//! integers; the division happens once, at cell-write time. That is not premature caution — the
//! spreadsheet is reconciled against a utility invoice to three decimal places, and accumulating
//! 744 floating-point divisions before summing them loses that agreement.

use jiff::Timestamp;
use std::{fmt, time::Duration};

/// One hour, the interval every reading in a Toronto Hydro export covers.
///
/// The whole design downstream depends on it: the interval count that drives the red completeness
/// highlight, and the guarantee that an aligned interval cannot straddle a TOU boundary, since
/// Ontario's price-period boundaries all fall on the hour. So it is **checked rather than
/// assumed** — the feed states it per `ReadingType` and again per `IntervalReading`, and
/// `green_button::parse` rejects any other value.
pub const METER_INTERVAL: Duration = Duration::from_secs(3600);

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
        crate::time::is_on_grid(self.start, METER_INTERVAL)
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

// cargo test --lib -- green_button::common::test --nocapture
#[cfg(test)]
mod test {
    use super::*;

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
