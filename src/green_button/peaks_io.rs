use crate::green_button::{Anomaly, PeriodValues};
use jiff::civil::Date;
use std::{error::Error, path::Path};

/// Reads Green Button XML data and returns the values for the billing period ending on
/// `period_ending`.
pub fn period_values_xml(
    xml_path: &Path,
    period_ending: Date,
) -> Result<(PeriodValues, Vec<Anomaly>), Box<dyn Error>> {
    todo!("modify the function signature as needed to do proper error and anomaly reporting")
}
