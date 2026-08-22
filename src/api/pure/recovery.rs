use crate::session::RSession;
use jiff::civil::Date;

/// EV cost-recovery TOU rates. The rates are effective for at least one month.
pub struct CostRecoveryRates {
    /// Effective date of the rates. Normally, the first day of a month.
    pub effective_date: Date,
    /// On-peak EV cost-recovery rate.
    pub on_peak: f64,
    /// Mid-peak EV cost-recovery rate.
    pub mid_peak: f64,
    /// Off-peak EV cost-recovery rate.
    pub off_peak: f64,
}

pub struct CostRecovery {}

pub enum CostRecoveryError {}

/// Returns the cost recovery allocated to the billing period. Applies the specified EV
/// cost-recovery TOU rates to the corresponding TOU energy use by EV charging sessions.
/// If the cost-recovery rates change during the billing period, a second set of cost-recovery
/// rates is specified.
pub fn cost_recovery(
    billing_period_ending: Date,
    sessions: &[RSession],
    recovery_rates_at_start: CostRecoveryRates,
    recovery_rates_at_end: Option<CostRecoveryRates>,
) -> Result<CostRecovery, CostRecoveryError> {
    todo!()
}
