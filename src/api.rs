use jiff::civil::Date;
use std::path::Path;

pub use crate::sessions::IntervalEstimates;

/// Peak power estimates for a billing period.
pub struct PowerEstimates {
    pub kw_estimates: IntervalEstimates,
    pub kva_estimates: IntervalEstimates,
}

pub enum ApiError {
    // Encapsulates the different kinds of error that the library may emit, in a way that is useful
    // for consumers of the API, i.e., the binaries in this crate.
}

/// Returns peak power estimates for the intervals of interest that maximize kW and kVA in the
/// specified billing period.
///
/// # Arguments
/// - `billing_period_ending` - the billing period.
/// - `gb_xml` - source Green Button XML file covering the billing period.
/// - `session_csv1` - Evolute session report covering the left end of the billing period.
/// - `session_csv2` - Evolute session report covering the right end of the billing period.
pub fn peak_power(
    billing_period_ending: Date,
    gb_xml: &Path,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<PowerEstimates, ApiError> {
    todo!()
}
