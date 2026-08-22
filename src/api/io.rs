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
pub use crate::api::error::{ApiError, CostRecoveryError, EnergyError, PeakPowerError};
pub use crate::api::pure::energy::{Energy, EnergyCost, TouKwh};
pub use crate::api::pure::peak_power::{DeliveryCost, PowerEstimates};
pub use crate::api::pure::recovery::{AtRates, CostRecovery, CostRecoveryRates};

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

    pure::peak_power(billing_period_ending, gb_period_values, &sessions).map_err(|cause| {
        ApiError::PeakPower {
            source: gb_source(&cause, None, gb_xml),
            cause,
        }
    })
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

    pure::peak_power_cost(&bill, gb_period_values, &sessions).map_err(|cause| ApiError::PeakPower {
        source: gb_source(&cause, Some(bill_pdf), gb_xml),
        cause,
    })
}

/// Returns the energy attributable to EV charging sessions in a billing period, split by
/// time-of-use period.
///
/// Reads the two session reports and hands them to [`pure::energy`](fn@super::pure::energy), which
/// states how a session's energy is divided. No meter export and no bill: consumption is billed by
/// the kilowatt-hour, so neither the hour the site peaked in nor any rate on the bill bears on it.
///
/// # Arguments
/// - `billing_period_ending` - the billing period, named by the date it closes on. Must be
///   [`BILL_END_DAY`] of its month.
/// - `session_csv1` - Evolute session report covering the left end of the billing period.
/// - `session_csv2` - Evolute session report covering the right end of the billing period.
///
/// The two reports must cover the billing period completely between them, which is checked from
/// their file names before anything is read. That check matters more here than to a peak estimate:
/// [`pure::energy`](fn@super::pure::energy) sums whatever it is given, so a month's report missing
/// from the call yields a total that is simply too low, with nothing in the figures to say so.
///
/// Reading a report writes a `.csv.read.log` beside it, as [`csv::session_list`] always does.
///
/// # Errors
///
/// See [`ApiError`]. Nothing is read until the file names have been checked against the period.
pub fn energy(
    billing_period_ending: Date,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<Energy, ApiError> {
    pure::check_reports_cover_period(billing_period_ending, &[session_csv1, session_csv2])?;
    let sessions = read_sessions(&[session_csv1, session_csv2])?;
    pure::energy(billing_period_ending, &sessions).map_err(|cause| ApiError::Energy {
        source: bill_source(&cause, None),
        cause,
    })
}

/// Returns the energy cost attributable to EV charging sessions in a billing period.
///
/// Reads the bill and the two session reports, and hands them to
/// [`pure::energy_cost`](fn@super::pure::energy_cost), which states how the figures are arrived at.
/// Every rate used is the bill's own; nothing here assumes a tariff.
///
/// No Green Button XML: consumption is billed by the kilowatt-hour, so the hour the site peaked in
/// does not bear on it. The meter export is [`peak_power_cost`]'s.
///
/// # Arguments
/// - `bill_pdf` - the Toronto Hydro bill PDF for the period.
/// - `session_csv1` - Evolute session report covering the left end of the billing period.
/// - `session_csv2` - Evolute session report covering the right end of the billing period.
///
/// There is no `billing_period_ending` argument. The bill states which period it covers, and it is
/// read first so that the reports are checked against that period rather than against one passed
/// alongside it, which could only agree with the bill or contradict it.
///
/// The two reports must cover the billing period completely between them, checked from their file
/// names. Which is given first makes no difference; the names say what each holds.
///
/// Reading a report writes a `.csv.read.log` beside it, as [`csv::session_list`] always does.
///
/// # Errors
///
/// See [`ApiError`]. The bill is read before the check that needs a period, so an unreadable bill is
/// reported ahead of anything the reports might also be wrong about.
pub fn energy_cost(
    bill_pdf: &Path,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<EnergyCost, ApiError> {
    // First, because it is what says which period this is about. `energy` can open with the free
    // name check instead; here that check has nothing to compare against until the bill is in hand.
    let bill = hydro_bill_from_pdf(bill_pdf).map_err(|cause| ReadError::Bill {
        path: bill_pdf.to_path_buf(),
        cause: Box::new(cause),
    })?;

    pure::check_reports_cover_period(bill.period_end_date(), &[session_csv1, session_csv2])?;
    let sessions = read_sessions(&[session_csv1, session_csv2])?;

    pure::energy_cost(&bill, &sessions).map_err(|cause| ApiError::Energy {
        source: bill_source(&cause, Some(bill_pdf)),
        cause,
    })
}

/// Returns the cost recovery allocated to a billing period, at the EV cost-recovery rates given.
///
/// Reads the two session reports and hands them to
/// [`pure::cost_recovery`](fn@super::pure::cost_recovery), which states how the figures are arrived
/// at. No bill and no meter export: the rates here are ours rather than Toronto Hydro's, so nothing
/// on the bill bears on the answer.
///
/// # Arguments
/// - `billing_period_ending` - the billing period, named by the date it closes on. Must be
///   [`BILL_END_DAY`] of its month.
/// - `session_csv1` - Evolute session report covering the left end of the billing period.
/// - `session_csv2` - Evolute session report covering the right end of the billing period.
/// - `recovery_rates_at_start` - the rates in effect on the period's first day.
/// - `recovery_rates_at_end` - the rates the period changed to, or `None` if it did not.
///
/// The rates are values rather than a path. Nothing in this crate writes them down, so there is no
/// file for this to read them from and no file a rate failure could be about.
///
/// The two reports must cover the billing period completely between them, which is checked from
/// their file names before anything is read. That check matters as much here as to
/// [`energy`]: the recovery is a sum over whatever it is given, so a month's report missing from
/// the call yields a figure that is simply too low, with nothing in it to say so.
///
/// Reading a report writes a `.csv.read.log` beside it, as [`csv::session_list`] always does.
///
/// # Errors
///
/// See [`ApiError`]. Nothing is read until the file names have been checked against the period, and
/// the rates are checked against it before the reports are opened for the same reason.
pub fn cost_recovery(
    billing_period_ending: Date,
    session_csv1: &Path,
    session_csv2: &Path,
    recovery_rates_at_start: CostRecoveryRates,
    recovery_rates_at_end: Option<CostRecoveryRates>,
) -> Result<CostRecovery, ApiError> {
    pure::check_reports_cover_period(billing_period_ending, &[session_csv1, session_csv2])?;
    let sessions = read_sessions(&[session_csv1, session_csv2])?;
    Ok(pure::cost_recovery(
        billing_period_ending,
        &sessions,
        recovery_rates_at_start,
        recovery_rates_at_end,
    )?)
}

/// Names the file a [`PeakPowerError`] is about, given the paths this call was made with.
///
/// Which file that is turns on the variant, not on the call: a period only partly covered is the
/// meter export's, while a date that closes no period is the bill's when a bill supplied it. Pure
/// says what went wrong and this says where, because neither knows both.
///
/// `bill_pdf` is `None` for [`peak_power`], where the date is the caller's own argument and no file
/// is at fault.
fn gb_source(cause: &PeakPowerError, bill_pdf: Option<&Path>, gb_xml: &Path) -> Option<PathBuf> {
    match cause {
        PeakPowerError::NotABillingPeriodEnding(_) => bill_pdf.map(Path::to_path_buf),
        PeakPowerError::NoPeak { .. }
        | PeakPowerError::ValuesAreForAnotherPeriod { .. }
        | PeakPowerError::PeriodNotFullyCovered { .. } => Some(gb_xml.to_path_buf()),
    }
}

/// Names the file an [`EnergyError`] is about.
///
/// Both variants concern the bill: it states the period, and it states the rates. `bill_pdf` is
/// `None` for [`energy`], which takes no bill and gets its date from the caller.
fn bill_source(cause: &EnergyError, bill_pdf: Option<&Path>) -> Option<PathBuf> {
    match cause {
        EnergyError::NotABillingPeriodEnding(_) | EnergyError::NoRate { .. } => {
            bill_pdf.map(Path::to_path_buf)
        }
    }
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
    use crate::hydro_bill::billing_period_dates;
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

    /// The energy total is a sum over whatever it is given, so a month missing from the call comes
    /// back as a figure that is simply too low. The names are what catch it, before either file is
    /// opened.
    #[test]
    fn energy_refuses_reports_that_do_not_cover_the_period() {
        let err = energy(
            date(2026, 6, 23),
            Path::new("Session_Report_April_1_2026-April_30_2026.csv"),
            Path::new("Session_Report_June_1_2026-June_30_2026.csv"),
        )
        .expect_err("April and June do not cover a period starting 24 May");
        assert!(
            matches!(
                err,
                ApiError::Coverage(CoverageError::PeriodNotCovered { .. })
            ),
            "{err}"
        );
        assert!(err.to_string().contains("2026-05-24"), "{err}");
    }

    /// A validation failure names the file it is about, which pure could not: it is handed figures,
    /// not paths, so it can say a period is only partly covered but not which export left the hole.
    #[test]
    fn a_validation_failure_names_the_file_it_is_about() {
        let bill = Path::new("June.pdf");
        let xml = Path::new("meter.XML");

        // The meter export's fault, whether or not a bill was given.
        let gap = PeakPowerError::PeriodNotFullyCovered {
            period_ending: date(2026, 6, 23),
            intervals: 743,
            expected: 744,
        };
        assert_eq!(gb_source(&gap, Some(bill), xml).as_deref(), Some(xml));
        assert_eq!(gb_source(&gap, None, xml).as_deref(), Some(xml));

        // The bill's fault, when a bill is what supplied the date. Same variant, different file:
        // this is why the choice cannot be made from the variant alone.
        let off_cycle = PeakPowerError::NotABillingPeriodEnding(
            billing_period_dates(date(2026, 6, 30)).expect_err("30 June closes no period"),
        );
        assert_eq!(
            gb_source(&off_cycle, Some(bill), xml).as_deref(),
            Some(bill)
        );
        // `peak_power` takes the date from its caller, so no file is at fault.
        assert_eq!(gb_source(&off_cycle, None, xml), None);

        // The rate a bill does not state is the bill's.
        let no_rate = EnergyError::NoRate {
            period_ending: date(2026, 6, 23),
            tou: crate::time::Tou::OnPeak,
        };
        assert_eq!(bill_source(&no_rate, Some(bill)).as_deref(), Some(bill));
        assert_eq!(bill_source(&no_rate, None), None);
    }

    /// The path is written into the message, unlike `ReadError::path`. These errors name no file of
    /// their own, so without it a caller holding four paths is told a period is uncovered and left
    /// to guess by which.
    #[test]
    fn the_named_file_reaches_the_message() {
        let cause = PeakPowerError::PeriodNotFullyCovered {
            period_ending: date(2026, 6, 23),
            intervals: 743,
            expected: 744,
        };
        let named = ApiError::PeakPower {
            source: Some(PathBuf::from("meter.XML")),
            cause: cause.clone(),
        };
        assert!(named.to_string().starts_with("meter.XML: "), "{named}");

        // Without one the message is the pure error's, unchanged - no stray separator.
        let bare = ApiError::PeakPower {
            source: None,
            cause: cause.clone(),
        };
        assert_eq!(bare.to_string(), cause.to_string());
    }

    /// The recovery is a sum over whatever it is given, as the energy total is, so the same name
    /// check has to catch a month missing from the call before either file is opened.
    #[test]
    fn cost_recovery_refuses_reports_that_do_not_cover_the_period() {
        let rates = CostRecoveryRates {
            effective_date: date(2026, 5, 1),
            on_peak: 0.11,
            mid_peak: 0.09,
            off_peak: 0.07,
        };
        let err = cost_recovery(
            date(2026, 6, 23),
            Path::new("Session_Report_April_1_2026-April_30_2026.csv"),
            Path::new("Session_Report_June_1_2026-June_30_2026.csv"),
            rates,
            None,
        )
        .expect_err("April and June do not cover a period starting 24 May");
        assert!(
            matches!(
                err,
                ApiError::Coverage(CoverageError::PeriodNotCovered { .. })
            ),
            "{err}"
        );
    }

    /// The energy cost takes no period either, so the bill is read before the report check, as it
    /// is for the delivery cost. With every path bad, the bill is still the failure reported.
    #[test]
    fn the_energy_cost_reads_the_bill_first_too() {
        let err = energy_cost(
            Path::new("nothing.pdf"),
            // Months that do not cover a period between them, so the report check would fire first
            // if it could run at all. It cannot: it has no period to check against yet.
            Path::new("Session_Report_April_1_2026-April_30_2026.csv"),
            Path::new("Session_Report_June_1_2026-June_30_2026.csv"),
        )
        .err()
        .expect("there is no such bill");
        assert!(
            matches!(err, ApiError::Read(ReadError::Bill { .. })),
            "{err}"
        );
        assert!(err.to_string().contains("nothing.pdf"), "{err}");
    }
}
