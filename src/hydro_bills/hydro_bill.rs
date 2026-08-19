use std::path::Path;

use jiff::civil::Date;

#[derive(Debug)]
/// Contents of a Toronto Hydro bill. Values with the same label that appear more than once in
/// the original bill are added together and shown as a single value in this data structure.
pub struct HydroBill {
    pub period_end_date: Date,
    pub statement_date: Date,

    // time_of_use_consumption_kwh
    pub on_peak_kwh: f64,
    pub mid_peak_kwh: f64,
    pub off_peak_kwh: f64,

    // time_of_use_cost
    pub on_peak_cost: f64,
    pub mid_peak_cost: f64,
    pub off_peak_cost: f64,

    // delivery
    pub delivery_customer_charges: f64,
    pub distribution_charges: f64,
    pub transmission_connection_charge: f64,
    pub transmission_network_charge: f64,

    // regulatory_charges
    pub standard_supply_admin_charge: f64,
    pub wholesale_market_svc_charge: f64,

    pub total_electricity_charges: f64,

    pub hst: f64,
    pub ontario_electricity_rebate: f64,

    pub bill_total_amount: f64,

    // your_electricity_usage
    pub meter_reading_period_from: Date,
    pub meter_reading_period_to: Date,
    pub number_of_days: u8,
    pub kwh_used: f64,
    pub loss_factor_adjustment: f64,
    pub adjusted_kwh_used: f64,
    pub peak_kw: f64,
    pub adj_peak_kw: f64,
    pub demand_kw: f64,
    pub demand_kva: f64,
    pub metering_adj: f64,
    pub adj_kw: f64,
    pub adj_kva: f64,
}

impl HydroBill {
    /// Reads a Toronto Hydro bill PDF file and returns a [`HydroBill`]. Values with the same label
    /// that appear more than once in the PDF are added together and shown as a single value in
    /// the data structure.
    pub fn from_pdf(path: &Path) -> HydroBill {
        todo!()
    }
}
