use crate::{
    hydro_bill::{BILL_END_DAY, BillingPeriod, HydroBill},
    session::{RSession, TouKwh, tou_kwh},
    time::Interval,
};

pub enum EnergyError {}

/// Energy consumption by TOU attributable EV charging sessions.
pub fn energy(hydro_bill: &HydroBill, sessions: &[RSession]) -> Result<TouKwh, EnergyError> {
    // Happy path.
    let period_end = hydro_bill.period_end_date();
    let period = BillingPeriod::ending_on(period_end, BILL_END_DAY);
    let time_range = Interval::from_start_end(period.start, period.end);
    let tou_kwh = tou_kwh(time_range, sessions);
    Ok(tou_kwh)
}
