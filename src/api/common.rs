use jiff::civil::Date;
use std::error::Error;
use std::path::PathBuf;

pub use crate::{green_button::PeriodValues, sessions::IntervalEstimates};

/// Peak power estimates for a billing period.
pub struct PowerEstimates {
    pub kw_estimates: IntervalEstimates,
    pub kva_estimates: IntervalEstimates,
}

#[derive(Debug)]
pub enum ApiError {
    // Encapsulates the different kinds of error that the library may emit, in a way that is useful
    // for consumers of the API, i.e., the binaries in this crate.
    //
    // Each variant carries what the caller would have to state to explain the failure, rather than
    // a message it has to parse. Rendering is [`fmt::Display`]'s job and happens in one place.
    /// The date given does not label a billing period: it is not [`BILL_END_DAY`] of its month.
    NotABillingPeriodEnding { ending: Date },

    /// The Green Button export could not be read, could not be parsed, or carries no reading in the
    /// billing period asked for.
    GreenButton {
        path: PathBuf,
        cause: Box<dyn Error>,
    },

    /// A session report could not be read.
    SessionReport {
        path: PathBuf,
        cause: Box<dyn Error>,
    },

    /// A session report's file name does not state the dates it covers, so it cannot be checked
    /// against the billing period. See [`report_coverage`].
    UndatedSessionReport { path: PathBuf },

    /// The session reports given do not cover the whole billing period between them.
    ///
    /// Almost always the wrong months handed in. The alternative is an estimate that reads as a
    /// small or zero EV contribution, which is a figure someone may go on to argue a bill from.
    PeriodNotCovered {
        period_start: Date,
        period_ending: Date,
        coverage: Vec<SessionReportCoverage>,
    },

    /// The billing period carries no reading in one of the two power series, so it has no maximum
    /// to estimate against. The feed is expected to carry hourly kW *and* kVA.
    NoPeak {
        period_ending: Date,
        unit: &'static str,
    },
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
