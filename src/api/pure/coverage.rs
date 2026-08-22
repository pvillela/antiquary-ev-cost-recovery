//! Whether the session reports named reach across a billing period.
//!
//! The two facts this joins live apart, and stay apart. What a report's file name says is a fact
//! about Evolute's exports, so it is
//! [`session::file_name`](crate::session::file_name); what a billing period spans is a fact about
//! the bill, so it is [`hydro_bill`](crate::hydro_bill). Neither module knows the other exists.
//! Asking whether one covers the other is a question in the API's terms, which is why it is asked
//! here.
//!
//! Nothing here opens anything. A `&Path` is read as a string.

use crate::hydro_bill::{NotABillingPeriodEnding, billing_period_dates};
use crate::session::file_name::{report_coverage, reports_cover};
use jiff::civil::Date;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

// Re-exported because it is what this module's function returns and what `CoverageError` carries,
// so a caller cannot spell either without it. Its own module is where it is documented.
pub use crate::session::file_name::SessionReportCoverage;

/// Why the session reports named cannot be checked against a billing period, or do not cover it.
///
/// Every variant is settled from the file *names*. Nothing here has opened anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageError {
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// A session report's file name does not state the dates it covers, so it cannot be checked
    /// against the billing period. See
    /// [`report_coverage`](crate::session::file_name::report_coverage).
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

impl From<NotABillingPeriodEnding> for CoverageError {
    fn from(e: NotABillingPeriodEnding) -> Self {
        Self::NotABillingPeriodEnding(e)
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

impl Error for CoverageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotABillingPeriodEnding(e) => Some(e),
            _ => None,
        }
    }
}

/// Checks that the named session reports cover the billing period completely between them, and
/// returns what each one covers.
///
/// Worth calling before anything is opened: a caller that has handed in the wrong month is told so
/// rather than after a year of meter readings has been parsed.
///
/// # Errors
///
/// [`CoverageError::NotABillingPeriodEnding`]; [`CoverageError::UndatedSessionReport`] for a name
/// that does not say what it covers; and [`CoverageError::PeriodNotCovered`] when the names between
/// them leave any day of the period unaccounted for.
pub fn check_reports_cover_period(
    billing_period_ending: Date,
    report_paths: &[&Path],
) -> Result<Vec<SessionReportCoverage>, CoverageError> {
    let (period_start, period_ending) = billing_period_dates(billing_period_ending)?;

    let coverage = report_paths
        .iter()
        .map(|path| {
            report_coverage(path).ok_or_else(|| CoverageError::UndatedSessionReport {
                path: path.to_path_buf(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if !reports_cover(period_start, period_ending, &coverage) {
        return Err(CoverageError::PeriodNotCovered {
            period_start,
            period_ending,
            coverage,
        });
    }
    Ok(coverage)
}

#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;

    /// The names alone settle whether the reports reach the period, so this answers without any of
    /// the files existing.
    #[test]
    fn coverage_is_checked_from_the_names_alone() {
        let may = Path::new("Session_Report_May_1_2026-May_31_2026.csv");
        let june = Path::new("Session_Report_June_1_2026-June_30_2026.csv");
        let april = Path::new("Session_Report_April_1_2026-April_30_2026.csv");

        assert_eq!(
            check_reports_cover_period(date(2026, 6, 23), &[may, june])
                .expect("May and June cover the period")
                .len(),
            2
        );

        let err = check_reports_cover_period(date(2026, 6, 23), &[april, june])
            .expect_err("April and June do not cover a period starting 24 May");
        assert!(
            matches!(err, CoverageError::PeriodNotCovered { .. }),
            "{err}"
        );
        assert!(err.to_string().contains("2026-05-24"), "{err}");

        let err = check_reports_cover_period(date(2026, 6, 23), &[Path::new("June.csv"), june])
            .expect_err("a name that does not state its dates");
        assert!(
            matches!(err, CoverageError::UndatedSessionReport { .. }),
            "{err}"
        );

        // The closing date is checked first, so a date that labels no period is reported as such
        // rather than as a coverage failure.
        let err = check_reports_cover_period(date(2026, 6, 30), &[may, june])
            .expect_err("30 June does not label a billing period");
        assert!(
            matches!(err, CoverageError::NotABillingPeriodEnding(_)),
            "{err}"
        );
    }
}
