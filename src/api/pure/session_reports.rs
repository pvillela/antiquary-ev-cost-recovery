//! What a session report's *file name* says, and whether a set of them reaches a billing period.
//!
//! The name is what a billing period is checked against, rather than the records inside. A report
//! legitimately holds no session on a quiet day, so its contents cannot tell "nobody charged" apart
//! from "wrong file", and it is the second of those that would quietly halve an estimate.
//!
//! Nothing here opens anything. A `&Path` is read as a string.

use crate::api::pure::billing_period::{NotABillingPeriodEnding, billing_period_dates};
use jiff::civil::Date;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

/// What a session report's file name says it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReportCoverage {
    pub path: PathBuf,
    /// First calendar date the report covers.
    pub from: Date,
    /// Last calendar date the report covers, inclusive.
    pub to: Date,
}

/// Why the session reports named cannot be checked against a billing period, or do not cover it.
///
/// Every variant is settled from the file *names*. Nothing here has opened anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageError {
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// A session report's file name does not state the dates it covers, so it cannot be checked
    /// against the billing period. See [`report_coverage`].
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

    if !covers(period_start, period_ending, &coverage) {
        return Err(CoverageError::PeriodNotCovered {
            period_start,
            period_ending,
            coverage,
        });
    }
    Ok(coverage)
}

/// The calendar dates a session report's file name says it covers, as in
/// `Session_Report_June_1_2026-June_30_2026.csv`.
///
/// `None` when the name is not of that form. Nothing else about the file is inspected — in
/// particular, not whether it exists.
pub fn report_coverage(path: &Path) -> Option<SessionReportCoverage> {
    let stem = path.file_stem()?.to_str()?;
    // Anything after the closing date is ignored, so a file marked up by hand -- a `-mock`, a
    // `-bak`, a `-what-if` -- still says what it covers. The two dates are what is read; a suffix
    // is a note to a person and says nothing about the sessions inside.
    let mut parts = stem.strip_prefix("Session_Report_")?.split('-');
    let from = report_date(parts.next()?)?;
    let to = report_date(parts.next()?)?;
    Some(SessionReportCoverage {
        path: path.to_path_buf(),
        from,
        to,
    })
}

/// `June_1_2026` as a date.
fn report_date(s: &str) -> Option<Date> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    let [month, day, year] = s.split('_').collect::<Vec<_>>()[..] else {
        return None;
    };
    let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(month))? as i8 + 1;
    // `Date::new` rather than the `date!` macro's panic on a day the month does not have: a file
    // name is input, and `Session_Report_June_31_2026-...` is a name to reject, not to crash on.
    Date::new(year.parse().ok()?, month, day.parse().ok()?).ok()
}

/// Whether the reports cover every date from `first` to `last` inclusive.
///
/// Order-insensitive, and tolerant of the reports overlapping, which they do whenever a session
/// runs past midnight on the last of the month. What it will not accept is a gap between them, or a
/// set that stops short at either end.
pub fn covers(first: Date, last: Date, coverage: &[SessionReportCoverage]) -> bool {
    let mut ranges: Vec<(Date, Date)> = coverage.iter().map(|c| (c.from, c.to)).collect();
    ranges.sort();

    // The last date covered so far, started just before the period rather than at `first` so that a
    // report reaching back before the period connects on the same test as one starting inside it,
    // and so that a report lying entirely before the period neither helps nor blocks.
    let mut through = first.yesterday().unwrap_or(Date::MIN);
    for (from, to) in ranges {
        if from > through.tomorrow().unwrap_or(Date::MAX) {
            break; // A gap. Nothing later can fill it, since the ranges are in order.
        }
        if to > through {
            through = to;
        }
    }
    through >= last
}

#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;

    fn coverage(name: &str) -> SessionReportCoverage {
        report_coverage(Path::new(name)).unwrap_or_else(|| panic!("{name} should parse"))
    }

    /// The name Evolute gives its exports, which is the only form this reads.
    #[test]
    fn a_report_name_states_the_dates_it_covers() {
        let c = coverage("data/Session_Report_June_1_2026-June_30_2026.csv");
        assert_eq!(c.from, date(2026, 6, 1));
        assert_eq!(c.to, date(2026, 6, 30));
        // A period straddling the new year is two years, not one repeated.
        let c = coverage("Session_Report_December_1_2025-January_31_2026.csv");
        assert_eq!(c.from, date(2025, 12, 1));
        assert_eq!(c.to, date(2026, 1, 31));
    }

    /// A suffix on the name is a note to a person, so it is ignored rather than allowed to hide
    /// what the file covers. `data` holds several such files.
    #[test]
    fn a_marked_up_name_still_states_its_dates() {
        for name in [
            "data/Session_Report_July_1_2026-July_31_2026-mock.csv",
            "Session_Report_July_1_2026-July_31_2026-bak.csv",
            "Session_Report_July_1_2026-July_31_2026-what-if.csv",
        ] {
            let c = coverage(name);
            assert_eq!((c.from, c.to), (date(2026, 7, 1), date(2026, 7, 31)));
        }
    }

    /// Anything else is refused rather than guessed at, because the guess would be checked against
    /// the billing period and could pass.
    #[test]
    fn a_name_that_does_not_state_its_dates_is_refused() {
        for name in [
            "June.csv",
            "Session_Report_June_2026.csv",
            "Session_Report_June_1_2026.csv",
            "Session_Report_Jun_1_2026-Jun_30_2026.csv",
            // June has 30 days, so this is a name to reject rather than a date to build.
            "Session_Report_June_1_2026-June_31_2026.csv",
        ] {
            assert!(
                report_coverage(Path::new(name)).is_none(),
                "{name} should not parse"
            );
        }
    }

    /// The billing period ending 23 June 2026 runs from 24 May, so it takes both months' reports.
    #[test]
    fn two_monthly_reports_cover_a_billing_period() {
        let (first, last) = (date(2026, 5, 24), date(2026, 6, 23));
        let may = coverage("Session_Report_May_1_2026-May_31_2026.csv");
        let june = coverage("Session_Report_June_1_2026-June_30_2026.csv");

        assert!(covers(first, last, &[may.clone(), june.clone()]));
        // Which one is named first is not a rule; the names say what each holds.
        assert!(covers(first, last, &[june.clone(), may.clone()]));
        // Either alone falls short at one end.
        assert!(!covers(first, last, &[may.clone(), may.clone()]));
        assert!(!covers(first, last, &[june.clone(), june.clone()]));
    }

    /// A month missing from the middle is what handing in the wrong file looks like.
    #[test]
    fn a_gap_between_the_reports_is_not_coverage() {
        let (first, last) = (date(2026, 5, 24), date(2026, 6, 23));
        let april = coverage("Session_Report_April_1_2026-April_30_2026.csv");
        let june = coverage("Session_Report_June_1_2026-June_30_2026.csv");
        assert!(!covers(first, last, &[april, june]));
    }

    /// A report reaching back before the period covers its part of it, and one lying entirely
    /// before the period neither helps nor blocks the one that does.
    #[test]
    fn a_report_wider_than_the_period_still_counts() {
        let (first, last) = (date(2026, 5, 24), date(2026, 6, 23));
        let spring = coverage("Session_Report_March_1_2026-June_30_2026.csv");
        let january = coverage("Session_Report_January_1_2026-January_31_2026.csv");
        assert!(covers(first, last, &[spring.clone(), january.clone()]));
        assert!(covers(first, last, &[january, spring]));
    }

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
