//! The one error type a front-end has to render.
//!
//! Each API function returns the narrowest type that describes what it can actually fail at, and
//! each of those lives with the function that raises it: a pure function cannot fail to read a
//! file, and its signature says so. That is the right shape for a caller who wants to act on the
//! failure, and the wrong shape for one who only wants to print it.
//!
//! [`ApiError`] is what those collapse into — one variant per stage of an API call, rather than one
//! per failure mode of every function. It sits in its own module because it depends on both halves
//! of the API and neither depends on it; putting it beside the narrow types would have pointed that
//! arrow the wrong way.

use std::error::Error;
use std::fmt;

// Re-exported, not merely imported. Matching on `ApiError` past its first level -- which is the
// ordinary thing to do with an error union -- forces a caller to name the payload types, so a
// module that hands out the union has to hand out what its variants carry. Nothing deeper: the
// fields inside those payloads can be read without being named.
pub use crate::api::io::ReadError;
pub use crate::api::pure::energy::EnergyError;
pub use crate::api::pure::peak_power::PeakPowerError;
pub use crate::api::pure::session_report::CoverageError;

/// Every way an API call can fail, in one type, by the stage that failed.
#[derive(Debug)]
pub enum ApiError {
    /// The arguments do not describe a billing period the named reports cover. Settled before
    /// anything is opened.
    Coverage(CoverageError),
    /// A source file could not be read.
    Read(ReadError),
    /// The figures were read but do not yield estimates.
    PeakPower(PeakPowerError),
    /// The figures were read but do not yield an energy attribution.
    Energy(EnergyError),
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

impl From<EnergyError> for ApiError {
    fn from(e: EnergyError) -> Self {
        Self::Energy(e)
    }
}

// No `From<NotABillingPeriodEnding>`. It would have to choose between `Coverage` and `PeakPower`
// arbitrarily, and the choice would be invisible at the call site. Convert through whichever of the
// two the calling function actually reports.

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coverage(e) => e.fmt(f),
            Self::Read(e) => e.fmt(f),
            Self::PeakPower(e) => e.fmt(f),
            Self::Energy(e) => e.fmt(f),
        }
    }
}

impl Error for ApiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Coverage(e) => Some(e),
            Self::Read(e) => Some(e),
            Self::PeakPower(e) => Some(e),
            Self::Energy(e) => Some(e),
        }
    }
}
