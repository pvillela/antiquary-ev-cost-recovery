//! The types [`io`](super::io) and [`pure`](super::pure) share: what an API call returns, and what
//! it returns instead when it cannot.
//!
//! # How the errors are arranged
//!
//! Each function returns the narrowest type that describes what it can actually fail at. A pure
//! function cannot fail to read a file, and its signature says so; nothing that returns
//! [`PeakPowerError`] can hand back a variant naming a path.
//!
//! [`ApiError`] is the union those collapse into, one variant per stage, for a front-end that would
//! rather render one type than match a different error for every call. Every narrow type converts
//! into it, so a caller that wants the detail keeps it and a caller that wants a message uses `?`
//! and forgets the distinction.
//!
//! The alternative — one flat enum for the whole API — is what this replaces. It made every caller
//! match on variants that could not arise for its call, and it grows a variant per failure mode per
//! function rather than a variant per stage.

use crate::hydro_bills::BILL_END_DAY;
use jiff::civil::Date;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

pub use crate::{green_button::PeriodValues, sessions::IntervalEstimates};

/// Peak power estimates for a billing period.
pub struct PowerEstimates {
    pub kw_estimates: IntervalEstimates,
    pub kva_estimates: IntervalEstimates,
}

/// What a session report's file name says it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReportCoverage {
    pub path: PathBuf,
    /// First calendar date the report covers.
    pub from: Date,
    /// Last calendar date the report covers, inclusive.
    pub to: Date,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The date given does not label a billing period: it is not [`BILL_END_DAY`] of its month.
///
/// A struct rather than a one-variant enum, so a function that can fail only this way says exactly
/// that and a caller has nothing to match on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotABillingPeriodEnding {
    pub ending: Date,
}

/// Why the session reports named cannot be checked against a billing period, or do not cover it.
///
/// Every variant is settled from the file *names*. Nothing here has opened anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageError {
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// A session report's file name does not state the dates it covers, so it cannot be checked
    /// against the billing period. See
    /// [`report_coverage`](super::pure::session_reports::report_coverage).
    UndatedSessionReport {
        path: PathBuf,
    },

    /// The session reports given do not cover the whole billing period between them.
    ///
    /// Almost always the wrong months handed in. The alternative is an estimate that reads as a
    /// small or zero EV contribution, which is a figure someone may go on to argue a bill from.
    PeriodNotCovered {
        period_start: Date,
        period_ending: Date,
        coverage: Vec<SessionReportCoverage>,
    },
}

/// A source file could not be read.
///
/// The only error kind [`io`](super::io) raises on its own; everything else it returns comes from a
/// pure function it delegated to.
#[derive(Debug)]
pub enum ReadError {
    /// The Green Button export could not be read, could not be parsed, or carries no reading in
    /// the billing period asked for.
    GreenButton {
        path: PathBuf,
        cause: Box<dyn Error>,
    },

    /// A session report could not be read.
    SessionReport {
        path: PathBuf,
        cause: Box<dyn Error>,
    },
}

/// Why a billing period's figures cannot be turned into peak power estimates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeakPowerError {
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// The billing period carries no reading in one of the two power series, so it has no maximum
    /// to estimate against. The feed is expected to carry hourly kW *and* kVA.
    NoPeak {
        period_ending: Date,
        unit: &'static str,
    },
}

/// Every way an API call can fail, in one type, by the stage that failed.
///
/// What a front-end presents. The narrow types above are what the functions themselves return; each
/// converts into this, so `?` collapses them without a caller writing the mapping.
#[derive(Debug)]
pub enum ApiError {
    /// The arguments do not describe a billing period the named reports cover. Settled before
    /// anything is opened.
    Coverage(CoverageError),
    /// A source file could not be read.
    Read(ReadError),
    /// The figures were read but do not yield estimates.
    PeakPower(PeakPowerError),
}

impl From<NotABillingPeriodEnding> for CoverageError {
    fn from(e: NotABillingPeriodEnding) -> Self {
        Self::NotABillingPeriodEnding(e)
    }
}

impl From<NotABillingPeriodEnding> for PeakPowerError {
    fn from(e: NotABillingPeriodEnding) -> Self {
        Self::NotABillingPeriodEnding(e)
    }
}

impl From<CoverageError> for ApiError {
    fn from(e: CoverageError) -> Self {
        Self::Coverage(e)
    }
}

impl From<ReadError> for ApiError {
    fn from(e: ReadError) -> Self {
        Self::Read(e)
    }
}

impl From<PeakPowerError> for ApiError {
    fn from(e: PeakPowerError) -> Self {
        Self::PeakPower(e)
    }
}

// No `From<NotABillingPeriodEnding> for ApiError`. It would have to choose between `Coverage` and
// `PeakPower` arbitrarily, and the choice would be invisible at the call site. Convert through
// whichever of the two the calling function actually reports.

impl fmt::Display for NotABillingPeriodEnding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ending = self.ending;
        write!(
            f,
            "{ending} does not name a billing period: one is labelled by day {BILL_END_DAY} of the \
             month it ends in"
        )
    }
}

impl fmt::Display for CoverageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABillingPeriodEnding(e) => e.fmt(f),
            Self::UndatedSessionReport { path } => write!(
                f,
                "{}: the file name does not say what the report covers; expected a name of the \
                 form Session_Report_June_1_2026-June_30_2026.csv",
                path.display()
            ),
            Self::PeriodNotCovered {
                period_start,
                period_ending,
                coverage,
            } => {
                write!(
                    f,
                    "the session reports do not cover the billing period {period_start} to \
                     {period_ending}:"
                )?;
                for c in coverage {
                    write!(f, "\n  {} covers {} to {}", c.path.display(), c.from, c.to)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GreenButton { path, cause } | Self::SessionReport { path, cause } => {
                write!(f, "{}: {cause}", path.display())
            }
        }
    }
}

impl fmt::Display for PeakPowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABillingPeriodEnding(e) => e.fmt(f),
            Self::NoPeak {
                period_ending,
                unit,
            } => write!(
                f,
                "the billing period ending {period_ending} carries no {unit} reading, so it has no \
                 {unit} maximum to estimate against"
            ),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coverage(e) => e.fmt(f),
            Self::Read(e) => e.fmt(f),
            Self::PeakPower(e) => e.fmt(f),
        }
    }
}

impl Error for NotABillingPeriodEnding {}

impl Error for CoverageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotABillingPeriodEnding(e) => Some(e),
            _ => None,
        }
    }
}

impl Error for ReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GreenButton { cause, .. } | Self::SessionReport { cause, .. } => {
                Some(cause.as_ref())
            }
        }
    }
}

impl Error for PeakPowerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotABillingPeriodEnding(e) => Some(e),
            _ => None,
        }
    }
}

impl Error for ApiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Coverage(e) => Some(e),
            Self::Read(e) => Some(e),
            Self::PeakPower(e) => Some(e),
        }
    }
}
