//! The contents of a Toronto Hydro bill, as a data structure.
//!
//! Where the figures come from is [`super::bill_pdf`]; this is only what they are once read. The
//! two are apart because the bill and the reading of it change for different reasons: a rate that
//! Toronto Hydro starts charging adds a field here, while a redesigned statement changes only how
//! the existing fields are found.

use std::fmt;

use jiff::civil::Date;

#[derive(Debug)]
/// Contents of a Toronto Hydro bill. Values with the same label that appear more than once in
/// the original bill are added together and shown as a single value in this data structure.
pub struct HydroBill {
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
    /// The rebate as a positive amount taken off the bill, though the bill prints it as a credit.
    pub ontario_electricity_rebate: f64,

    // your_electricity_usage
    pub meter_reading_period_from: Date,
    pub meter_reading_period_to: Date,
    pub number_of_days: u8,
    pub kwh_used: f64,
    pub loss_factor_adjustment: f64,
    pub adjusted_kwh_used: f64,
    pub peak_7_7_kw: f64,
    pub adj_peak_7_7_kw: f64,
    pub demand_kw: f64,
    pub demand_kva: f64,
    pub metering_adj: f64,
    pub adj_kw: f64,
    pub adj_kva: f64,
}

impl HydroBill {
    /// The end of the period this bill covers.
    ///
    /// The bill states no period end of its own. The meter reading period is that period: it runs
    /// from the 23rd of one month to the 23rd of the next, which is the same division, and the
    /// same label, that `green_button::BillingPeriod` uses.
    pub fn period_end_date(&self) -> Date {
        self.meter_reading_period_to
    }

    /// What this billing period cost: `total_electricity_charges + hst - ontario_electricity_rebate`.
    ///
    /// Not the bill's `Amount Due`, which the bill states only after adding whatever was still
    /// owed from last time. The two agree except when a bill goes out before the one before it was
    /// paid, and then `Amount Due` is two periods of charges added together -- a figure that
    /// belongs to neither period on its own.
    pub fn bill_total_amount(&self) -> f64 {
        self.total_electricity_charges + self.hst - self.ontario_electricity_rebate
    }

    /// Outputs the bill to a formatted pretty-print string, including `period_end_date` and
    /// `bill_total_amount`.
    pub fn print(&self) -> String {
        format!("{:#?}", Pretty(self))
    }
}

/// The bill as `Debug` renders it, with the two worked-out values put back where the fields they
/// replaced used to sit.
///
/// `Debug` is derived and so can only show what is stored. This lists the fields by hand to fold
/// the two methods in among them, and does it through `debug_struct`, which is the same machinery
/// the derive uses -- so the layout stays identical rather than being reproduced by eye.
struct Pretty<'a>(&'a HydroBill);

impl fmt::Debug for Pretty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bill = self.0;
        f.debug_struct("HydroBill")
            .field("period_end_date", &bill.period_end_date())
            .field("statement_date", &bill.statement_date)
            .field("on_peak_kwh", &bill.on_peak_kwh)
            .field("mid_peak_kwh", &bill.mid_peak_kwh)
            .field("off_peak_kwh", &bill.off_peak_kwh)
            .field("on_peak_cost", &bill.on_peak_cost)
            .field("mid_peak_cost", &bill.mid_peak_cost)
            .field("off_peak_cost", &bill.off_peak_cost)
            .field("delivery_customer_charges", &bill.delivery_customer_charges)
            .field("distribution_charges", &bill.distribution_charges)
            .field(
                "transmission_connection_charge",
                &bill.transmission_connection_charge,
            )
            .field(
                "transmission_network_charge",
                &bill.transmission_network_charge,
            )
            .field(
                "standard_supply_admin_charge",
                &bill.standard_supply_admin_charge,
            )
            .field(
                "wholesale_market_svc_charge",
                &bill.wholesale_market_svc_charge,
            )
            .field("total_electricity_charges", &bill.total_electricity_charges)
            .field("hst", &bill.hst)
            .field(
                "ontario_electricity_rebate",
                &bill.ontario_electricity_rebate,
            )
            .field("bill_total_amount", &bill.bill_total_amount())
            .field("meter_reading_period_from", &bill.meter_reading_period_from)
            .field("meter_reading_period_to", &bill.meter_reading_period_to)
            .field("number_of_days", &bill.number_of_days)
            .field("kwh_used", &bill.kwh_used)
            .field("loss_factor_adjustment", &bill.loss_factor_adjustment)
            .field("adjusted_kwh_used", &bill.adjusted_kwh_used)
            .field("peak_kw", &bill.peak_7_7_kw)
            .field("adj_peak_kw", &bill.adj_peak_7_7_kw)
            .field("demand_kw", &bill.demand_kw)
            .field("demand_kva", &bill.demand_kva)
            .field("metering_adj", &bill.metering_adj)
            .field("adj_kw", &bill.adj_kw)
            .field("adj_kva", &bill.adj_kva)
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date as civil_date;

    /// Figures from `TH_5728140000_2025_07_28.pdf`.
    fn sample() -> HydroBill {
        HydroBill {
            statement_date: civil_date(2025, 7, 28),
            on_peak_kwh: 13240.523,
            mid_peak_kwh: 12412.311,
            off_peak_kwh: 45562.91,
            on_peak_cost: 2092.0,
            mid_peak_cost: 1514.3,
            off_peak_cost: 3462.78,
            delivery_customer_charges: 61.86,
            distribution_charges: 1761.07,
            transmission_connection_charge: 436.1,
            transmission_network_charge: 648.17,
            standard_supply_admin_charge: 0.25,
            wholesale_market_svc_charge: 427.29,
            total_electricity_charges: 10403.82,
            hst: 1352.5,
            ontario_electricity_rebate: 1362.9,
            meter_reading_period_from: civil_date(2025, 6, 23),
            meter_reading_period_to: civil_date(2025, 7, 23),
            number_of_days: 30,
            kwh_used: 69175.078,
            loss_factor_adjustment: 1.0295,
            adjusted_kwh_used: 71215.743,
            peak_7_7_kw: 140.639,
            adj_peak_7_7_kw: 140.639,
            demand_kw: 140.639,
            demand_kva: 170.879,
            metering_adj: 1.0,
            adj_kw: 140.639,
            adj_kva: 170.879,
        }
    }

    #[test]
    fn the_period_ends_when_the_meter_was_last_read() {
        assert_eq!(sample().period_end_date(), civil_date(2025, 7, 23));
    }

    #[test]
    fn the_total_is_the_charges_plus_tax_less_the_rebate() {
        assert!((sample().bill_total_amount() - 10_393.42).abs() < 0.005);
    }

    /// Guards the one hazard in listing the fields by hand: a field added to [`HydroBill`] that
    /// nobody remembers to add to [`Pretty`]. Strip the two worked-out lines from `print` and what
    /// is left must be the derived output exactly, so a missing field shows up here as a
    /// difference rather than as a quietly shorter dump.
    #[test]
    fn print_is_the_derived_output_plus_the_two_worked_out_values() {
        let bill = sample();
        let text = bill.print();
        let printed: Vec<&str> = text.lines().collect();
        let worked_out = ["    period_end_date: ", "    bill_total_amount: "];
        assert_eq!(
            printed
                .iter()
                .filter(|line| worked_out.iter().any(|prefix| line.starts_with(prefix)))
                .count(),
            2,
            "both worked-out values should appear once"
        );
        let stored: Vec<&str> = printed
            .into_iter()
            .filter(|line| !worked_out.iter().any(|prefix| line.starts_with(prefix)))
            .collect();
        assert_eq!(stored, format!("{bill:#?}").lines().collect::<Vec<_>>());
    }
}
