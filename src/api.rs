//! What a front-end asks the library, stated in the terms a front-end has.
//!
//! The modules below are organised by the source of data they read. A caller checking an invoice
//! has none of those in hand — it has a billing period, a meter export and the charging network's
//! monthly reports — so this module is where those become the calls the modules understand, and
//! where the several error types they return become one a front-end can present.

use crate::green_button::{BillingPeriod, METER_INTERVAL, Peak, period_values_xml};
use crate::hydro_bills::BILL_END_DAY;
use crate::sessions::{SessionReport, csv, estimates_from_report};
use crate::time::{Interval, standard_date};
use jiff::civil::Date;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

pub use crate::sessions::IntervalEstimates;

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
        coverage: Vec<ReportCoverage>,
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
pub struct ReportCoverage {
    pub path: PathBuf,
    /// First calendar date the report covers.
    pub from: Date,
    /// Last calendar date the report covers, inclusive.
    pub to: Date,
}

/// Returns peak power estimates for the intervals of interest that maximize kW and kVA in the
/// specified billing period.
///
/// The two intervals are the hours the *building* peaked in, read from the meter export, and each
/// estimate says how much of that hour's demand the chargers can account for. They are usually
/// different hours, and occasionally the same one.
///
/// Each interval is a whole metering hour, because that is the resolution the Green Button feed
/// states demand at. The estimate within it is still a 15-minute figure: an
/// [`IntervalEstimates`] reports the highest of the hour's four segments, which is the basis the
/// demand charge is billed on. See docs/sessions/README.md, "Interval of interest boundaries".
///
/// The maxima used are the period's unrestricted ones — what an invoice bills as `Demand kW` and
/// `Demand kVA` — not the 07:00-19:00 figures it reports as `Peak kW 7-7`.
///
/// # Arguments
/// - `billing_period_ending` - the billing period, named by the date it closes on. Must be
///   [`BILL_END_DAY`] of its month.
/// - `gb_xml` - source Green Button XML file covering the billing period.
/// - `session_csv1` - Evolute session report covering the left end of the billing period.
/// - `session_csv2` - Evolute session report covering the right end of the billing period.
///
/// The two reports must cover the billing period completely between them, which is checked from
/// their file names before anything is read. Which is given first makes no difference; the names
/// say what each holds.
///
/// # Sessions the two reports share
///
/// Their coverage overlaps, so a session near the boundary appears in both. A record the two state
/// identically is one session and is counted once; counting it twice would inflate every figure
/// drawn from it.
///
/// A `Charge_Session_ID` carried by two records that are *not* identical does not collapse. Such
/// records are kept and estimated from, both of them, and each is flagged
/// [`AnomalyKind::DuplicateId`](crate::sessions::AnomalyKind::DuplicateId) in the returned
/// [`IntervalEstimates::session_anomalies`] — subject to the same scoping as every other anomaly
/// there, so one is listed only if its session reaches that estimate's interval.
///
/// It is not an error, because it is not necessarily a fault. `Charge_Session_ID` is not unique in
/// Evolute's reports: the June 2026 report carries `S37487` on two sessions a week apart, within
/// the one file. Refusing would make that month unestimatable. The flag also cannot tell a reused
/// id from the two reports genuinely disagreeing about one session, since from here the two look
/// the same — which is why both records are kept and the judgement is left to a reader who can go
/// back to the source rows.
///
/// # Errors
///
/// See [`ApiError`]. Nothing is read until the arguments have been checked against each other, so a
/// caller that has handed in the wrong month is told so rather than after parsing a year of meter
/// readings.
pub fn peak_power(
    billing_period_ending: Date,
    gb_xml: &Path,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<PowerEstimates, ApiError> {
    // `BillingPeriod::ending_on` panics on a date that is not a closing date, and this is the
    // outermost layer, where a caller's argument is still a caller's argument rather than a bug.
    if billing_period_ending.day() != BILL_END_DAY {
        return Err(ApiError::NotABillingPeriodEnding {
            ending: billing_period_ending,
        });
    }
    let period = BillingPeriod::ending_on(billing_period_ending, BILL_END_DAY);
    // The period boundary is on standard time, so the calendar dates it spans are the standard-time
    // ones. `period.end` is exclusive and lands on the day after the close, which is why the last
    // date is the closing date itself rather than anything read off `end`.
    let period_start = standard_date(period.start);

    let coverage = [session_csv1, session_csv2]
        .iter()
        .map(|path| {
            report_coverage(path).ok_or_else(|| ApiError::UndatedSessionReport {
                path: path.to_path_buf(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !covers(period_start, billing_period_ending, &coverage) {
        return Err(ApiError::PeriodNotCovered {
            period_start,
            period_ending: billing_period_ending,
            coverage,
        });
    }

    let values =
        period_values_xml(gb_xml, billing_period_ending, BILL_END_DAY).map_err(|cause| {
            ApiError::GreenButton {
                path: gb_xml.to_path_buf(),
                cause,
            }
        })?;
    let kw_ioi = peak_interval(values.max_kw, "kW", billing_period_ending)?;
    let kva_ioi = peak_interval(values.max_kva, "kVA", billing_period_ending)?;

    let reports = [session_csv1, session_csv2]
        .iter()
        .map(|path| {
            csv::session_list(path).map_err(|cause| ApiError::SessionReport {
                path: path.to_path_buf(),
                cause,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Records the two reports carry identically are collapsed; an id they disagree about, or one
    // Evolute reused, leaves both sessions in the estimates and raises
    // `AnomalyKind::DuplicateId` against each. See `MergedSessions::merge_sessions`.
    let sessions = SessionReport::merge(reports);

    // Both estimates come off the one merged report rather than off a second read, so the two
    // figures cannot be drawn from different session data.
    let sources = vec![session_csv1.to_path_buf(), session_csv2.to_path_buf()];
    Ok(PowerEstimates {
        kw_estimates: estimates_from_report(kw_ioi, sources.clone(), &sessions),
        kva_estimates: estimates_from_report(kva_ioi, sources, &sessions),
    })
}

/// The metering hour a peak occurred in, as an interval of interest.
fn peak_interval(
    peak: Option<Peak>,
    unit: &'static str,
    period_ending: Date,
) -> Result<Interval, ApiError> {
    let peak = peak.ok_or(ApiError::NoPeak {
        period_ending,
        unit,
    })?;
    // Only an interval that starts on the hour can be a peak — see `green_button::peaks` — so this
    // is always a legal interval of interest and needs no further checking.
    Ok(Interval::new(peak.at, METER_INTERVAL))
}

/// The calendar dates a session report's file name says it covers, as in
/// `Session_Report_June_1_2026-June_30_2026.csv`.
///
/// The name is what the billing period is checked against, rather than the records inside. A report
/// legitimately holds no session on a quiet day, so its contents cannot tell "nobody charged" apart
/// from "wrong file", and it is the second of those that would quietly halve an estimate.
///
/// `None` when the name is not of that form. Nothing else about the file is inspected.
fn report_coverage(path: &Path) -> Option<ReportCoverage> {
    let stem = path.file_stem()?.to_str()?;
    let (from, to) = stem.strip_prefix("Session_Report_")?.split_once('-')?;
    Some(ReportCoverage {
        path: path.to_path_buf(),
        from: report_date(from)?,
        to: report_date(to)?,
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
/// Order-insensitive, and tolerant of the two overlapping, which they do whenever a session runs
/// past midnight on the last of the month. What it will not accept is a gap between them, or a pair
/// that stops short at either end.
fn covers(first: Date, last: Date, coverage: &[ReportCoverage]) -> bool {
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

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABillingPeriodEnding { ending } => write!(
                f,
                "{ending} does not name a billing period: one is labelled by day {BILL_END_DAY} of \
                 the month it ends in"
            ),
            Self::GreenButton { path, cause } => write!(f, "{}: {cause}", path.display()),
            Self::SessionReport { path, cause } => write!(f, "{}: {cause}", path.display()),
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

impl Error for ApiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GreenButton { cause, .. } | Self::SessionReport { cause, .. } => {
                Some(cause.as_ref())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;

    fn coverage(name: &str) -> ReportCoverage {
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

    /// A date that is not a closing date is the caller's mistake, and is reported as such rather
    /// than reaching the panic in `BillingPeriod::ending_on`.
    #[test]
    fn a_date_that_does_not_close_a_billing_period_is_refused() {
        let err = peak_power(
            date(2026, 6, 30),
            Path::new("nothing.XML"),
            Path::new("Session_Report_May_1_2026-May_31_2026.csv"),
            Path::new("Session_Report_June_1_2026-June_30_2026.csv"),
        )
        .err()
        .expect("30 June does not label a billing period");
        assert!(
            matches!(err, ApiError::NotABillingPeriodEnding { .. }),
            "{err}"
        );
    }

    /// The arguments are checked against each other before any file is opened, so the wrong months
    /// are named as such even though none of the paths exist.
    #[test]
    fn the_reports_are_checked_before_anything_is_read() {
        let err = peak_power(
            date(2026, 6, 23),
            Path::new("nothing.XML"),
            Path::new("Session_Report_April_1_2026-April_30_2026.csv"),
            Path::new("Session_Report_June_1_2026-June_30_2026.csv"),
        )
        .err()
        .expect("April and June do not cover a period starting 24 May");
        assert!(matches!(err, ApiError::PeriodNotCovered { .. }), "{err}");
        assert!(err.to_string().contains("2026-05-24"), "{err}");
    }
}
