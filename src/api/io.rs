//! The half of the API that reads.
//!
//! Every function here turns paths into values and then delegates to [`pure`](super::pure), which
//! is where the arithmetic and the judgement live. Keeping this half thin is the point: what a
//! figure rests on can then be exercised without a filesystem in the way, and the two halves cannot
//! drift apart because there is only one of each calculation.

use crate::api::pure;
use crate::green_button::period_values_xml;
use crate::hydro_bill::{BILL_END_DAY, hydro_bill_from_pdf};
use crate::session::{RSession, csv};
use jiff::civil::Date;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

// Re-exported rather than merely imported: these name what the functions here return, and a caller
// should not have to know which module a call delegates to in order to spell that.
pub use crate::api::error::ApiError;
pub use crate::api::pure::peak_power::{DeliveryCost, PowerEstimates};

/// A source file could not be read.
///
/// The only error kind this module raises on its own. Everything else it returns comes from a pure
/// function it delegated to.
///
/// `path` is held for a caller that wants to act on which file failed rather than print it. It is
/// deliberately *not* written into the message: both readers name the file they concern, so adding
/// it here produced `data/x.XML: data/x.XML: ...`.
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

    /// A Toronto Hydro bill PDF could not be read, or is not laid out the way one is read.
    ///
    /// [`BillError::is_layout`](crate::hydro_bill::BillError::is_layout) tells those two apart,
    /// and `cause` downcasts to [`BillError`](crate::hydro_bill::BillError) for a caller that
    /// wants to ask.
    Bill {
        path: PathBuf,
        cause: Box<dyn Error>,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GreenButton { cause, .. }
            | Self::SessionReport { cause, .. }
            | Self::Bill { cause, .. } => cause.fmt(f),
        }
    }
}

impl Error for ReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GreenButton { cause, .. }
            | Self::SessionReport { cause, .. }
            | Self::Bill { cause, .. } => Some(cause.as_ref()),
        }
    }
}

/// Returns peak power estimates for the intervals of interest that maximize kW and kVA in the
/// specified billing period.
///
/// Reads the meter export and the two session reports, and hands them to
/// [`pure::peak_power`](fn@super::pure::peak_power), which states what is done with them and returns
/// the figures. The two intervals are the hours the *building* peaked in, and each estimate says
/// how much of that hour's demand the chargers can account for.
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
/// Reading a report writes a `.csv.read.log` beside it, as [`csv::session_list`] always does.
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
    // First, because it is the one check that costs nothing: it reads the two file *names*, and a
    // caller who has handed in the wrong month finds out before a byte is parsed.
    pure::check_reports_cover_period(billing_period_ending, &[session_csv1, session_csv2])?;

    let gb_period_values =
        period_values_xml(gb_xml, billing_period_ending, BILL_END_DAY).map_err(|cause| {
            ReadError::GreenButton {
                path: gb_xml.to_path_buf(),
                cause,
            }
        })?;
    let sessions = read_sessions(&[session_csv1, session_csv2])?;

    Ok(pure::peak_power(
        billing_period_ending,
        gb_period_values,
        &sessions,
    )?)
}

/// Returns the delivery cost attributable to EV charging sessions in a billing period.
///
/// Reads the bill, the meter export and the two session reports, and hands them to
/// [`pure::peak_power_cost`](fn@super::pure::peak_power_cost), which states how the figures are
/// arrived at. Every rate used is the bill's own; nothing here assumes a tariff.
///
/// # Arguments
/// - `bill_pdf` - the Toronto Hydro bill PDF for the period.
/// - `gb_xml` - source Green Button XML file covering the billing period.
/// - `session_csv1` - Evolute session report covering the left end of the billing period.
/// - `session_csv2` - Evolute session report covering the right end of the billing period.
///
/// There is no `billing_period_ending` argument. The bill states which period it covers, and it is
/// read first so that every other file is fetched for that period — the meter export selected by
/// it, and the reports checked against it. A period passed alongside could only agree with the bill
/// or contradict it, and [`pure::peak_power_cost`](fn@super::pure::peak_power_cost) drops it for
/// the same reason.
///
/// The two reports must cover the billing period completely between them, checked from their file
/// names. Which is given first makes no difference; the names say what each holds.
///
/// Reading a report writes a `.csv.read.log` beside it, as [`csv::session_list`] always does.
///
/// # Errors
///
/// See [`ApiError`]. The bill is read before the checks that need a period, so an unreadable bill
/// is reported ahead of anything the reports or the export might also be wrong about.
pub fn peak_power_cost(
    bill_pdf: &Path,
    gb_xml: &Path,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<DeliveryCost, ApiError> {
    // First, because it is what says which period this is about. `peak_power` can open with the
    // free name check instead; here that check has nothing to compare against until the bill is in
    // hand.
    let bill = hydro_bill_from_pdf(bill_pdf).map_err(|cause| ReadError::Bill {
        path: bill_pdf.to_path_buf(),
        cause: Box::new(cause),
    })?;
    let billing_period_ending = bill.period_end_date();

    // Still ahead of both parses: it reads the two file *names*, so the wrong month is caught
    // before a year of meter readings is.
    pure::check_reports_cover_period(billing_period_ending, &[session_csv1, session_csv2])?;

    let gb_period_values =
        period_values_xml(gb_xml, billing_period_ending, BILL_END_DAY).map_err(|cause| {
            ReadError::GreenButton {
                path: gb_xml.to_path_buf(),
                cause,
            }
        })?;
    let sessions = read_sessions(&[session_csv1, session_csv2])?;

    Ok(pure::peak_power_cost(&bill, gb_period_values, &sessions)?)
}

/// Every session the named reports hold, in the order the reports are given.
///
/// Flattened rather than kept per file, and deliberately not merged. Which records describe one
/// session is a question about the records, so it belongs to
/// [`pure::peak_power`](fn@super::pure::peak_power); this only fetches them.
///
/// All three of each report's buckets are taken. Bucketing is a function of a session's own
/// anomalies, so it is redone identically on the other side, and dropping the excluded ones here
/// would silently lose the records a reader most needs to see.
fn read_sessions(paths: &[&Path]) -> Result<Vec<RSession>, ReadError> {
    let mut sessions = Vec::new();
    for path in paths {
        let report = csv::session_list(path).map_err(|cause| ReadError::SessionReport {
            path: path.to_path_buf(),
            cause,
        })?;
        sessions.extend(report.sessions);
        sessions.extend(report.spikes);
        sessions.extend(report.excluded);
    }
    Ok(sessions)
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::api::error::CoverageError;
    use jiff::civil::date;

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
            matches!(
                err,
                ApiError::Coverage(CoverageError::NotABillingPeriodEnding(_))
            ),
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
        assert!(
            matches!(
                err,
                ApiError::Coverage(CoverageError::PeriodNotCovered { .. })
            ),
            "{err}"
        );
        assert!(err.to_string().contains("2026-05-24"), "{err}");
    }

    /// The cost takes no period, so the bill is what supplies one and is read before anything that
    /// needs it. With every path bad, the bill is still the failure reported.
    #[test]
    fn the_bill_is_read_first_because_it_is_what_names_the_period() {
        let err = peak_power_cost(
            Path::new("nothing.pdf"),
            Path::new("nothing.XML"),
            // Months that do not cover a period between them, so the report check would fire
            // first if it could run at all. It cannot: it has no period to check against yet.
            Path::new("Session_Report_April_1_2026-April_30_2026.csv"),
            Path::new("Session_Report_June_1_2026-June_30_2026.csv"),
        )
        .err()
        .expect("there is no such bill");
        assert!(
            matches!(err, ApiError::Read(ReadError::Bill { .. })),
            "{err}"
        );
        // Named once, by the reader, as every other file in this module is.
        assert!(err.to_string().contains("nothing.pdf"), "{err}");
    }
}
