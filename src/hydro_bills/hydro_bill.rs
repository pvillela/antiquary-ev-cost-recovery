//! Reading a Toronto Hydro bill PDF into [`HydroBill`].
//!
//! The bill is a form, not a table: every figure is found by the words next to it. Two habits of
//! the generator shape the parsing.
//!
//! First, a label and its value are separate runs of text sharing a baseline, so a value is read
//! as "the run to the right of these words" rather than by any column index. Second, the same
//! label can appear more than once on one bill, and the values must then be added. That happens
//! for two independent reasons: a billing period that straddles the Summer/Winter boundary
//! carries a Time-of-Use block for each season, and a period that straddles a rate change carries
//! every delivery and regulatory line twice, once at the old rate and once at the new. Toronto
//! Hydro prints "Seeing double? You're not being charged twice" beside the second set.
//!
//! Which page a section lands on moves with the length of the charges, so the whole document is
//! read as one sequence of lines rather than page by page.

use std::error::Error;
use std::path::Path;

use jiff::civil::Date;

use crate::hydro_bills::pdf_text::{self, Fragment, Line};

/// Where the charges column ends, in PDF points from the left edge of the page.
///
/// The bills print promotional text beside the charges, and its lines land between the charge
/// lines rather than beside them -- a "Seeing double?" notice sits between a Transmission
/// Connection Charge and the rate line that carries its amount. Cutting the page at this vertical
/// line removes that column, which is what lets a rate line be found as the next line down. The
/// rightmost charge amount on the bills read so far starts at 327; the promotional column starts
/// at 394.
const CHARGE_COLUMN_RIGHT: f64 = 360.0;

/// Month names as the bill writes them, in either case: `Jan 28 2026`, `JUN 23 2025`.
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

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
    /// The rebate as a positive amount taken off the bill, though the bill prints it as a credit.
    pub ontario_electricity_rebate: f64,

    /// What this billing period cost: `total_electricity_charges + hst - ontario_electricity_rebate`.
    ///
    /// Not the bill's `Amount Due`, which the bill states only after adding whatever was still
    /// owed from last time. The two agree except when a bill goes out before the one before it was
    /// paid, and then `Amount Due` is two periods of charges added together -- a figure that
    /// belongs to neither period on its own.
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
    ///
    /// # Errors
    ///
    /// Returns an error naming what was missing or unreadable. A charge line the parser does not
    /// recognise is an error rather than a figure quietly left out of the totals: these numbers
    /// get reconciled against metered consumption, and a charge that silently reads as zero is
    /// worse than no answer.
    pub fn from_pdf(path: &Path) -> Result<HydroBill, Box<dyn Error>> {
        let lines: Vec<Line> = pdf_text::read_pages(path)?.into_iter().flatten().collect();
        HydroBill::from_lines(&lines).map_err(|e| format!("{}: {e}", path.display()).into())
    }

    /// The parse proper, over the lines of the whole document in reading order.
    fn from_lines(lines: &[Line]) -> Result<HydroBill, Box<dyn Error>> {
        let usage = Usage::read(lines)?;
        let charges = Charges::read(lines)?;
        let hst = money(value_after_prefix(lines, "H.S.T.")?)?;
        // Printed as a credit, with `CR` in the column to its right, and held here as the positive
        // amount taken off the bill.
        let rebate = money(value_after(lines, "Ontario Electricity Rebate")?)?;

        Ok(HydroBill {
            // The bill states no period end of its own. The meter reading period is that period:
            // it runs from the 23rd of one month to the 23rd of the next, which is the same
            // division, and the same label, that `green_button::BillingPeriod` uses.
            period_end_date: usage.reading_period_to,
            statement_date: date(value_after(lines, "Statement Date")?)?,

            on_peak_kwh: charges.on_peak_kwh,
            mid_peak_kwh: charges.mid_peak_kwh,
            off_peak_kwh: charges.off_peak_kwh,

            on_peak_cost: charges.on_peak_cost,
            mid_peak_cost: charges.mid_peak_cost,
            off_peak_cost: charges.off_peak_cost,

            delivery_customer_charges: charges.customer_charges,
            distribution_charges: charges.distribution_charges,
            transmission_connection_charge: charges.transmission_connection,
            transmission_network_charge: charges.transmission_network,

            standard_supply_admin_charge: charges.supply_admin,
            wholesale_market_svc_charge: charges.wholesale_market,

            total_electricity_charges: charges.total,

            hst,
            ontario_electricity_rebate: rebate,

            // Worked out rather than read off the bill. `Amount Due` is this sum plus any balance
            // still owed from the bill before, and there is no line stating the period's own
            // total once a balance forward has been folded into it.
            bill_total_amount: charges.total + hst - rebate,

            meter_reading_period_from: usage.reading_period_from,
            meter_reading_period_to: usage.reading_period_to,
            number_of_days: usage.number_of_days,
            kwh_used: usage.kwh_used,
            loss_factor_adjustment: usage.loss_factor_adjustment,
            adjusted_kwh_used: usage.adjusted_kwh_used,
            peak_kw: usage.peak_kw,
            adj_peak_kw: usage.adj_peak_kw,
            demand_kw: usage.demand_kw,
            demand_kva: usage.demand_kva,
            metering_adj: usage.metering_adj,
            adj_kw: usage.adj_kw,
            adj_kva: usage.adj_kva,
        })
    }
}

/// Which charge a rate line belongs to.
///
/// Four of the charges print their amount on the line below their name, alongside the rate it was
/// worked out from -- `140.640 kW at $3.1008 per kW per 30 Days`. The name alone carries no
/// figure, so it only records what the next rate line will be adding to.
#[derive(Clone, Copy)]
enum RateLineFor {
    TransmissionConnection,
    TransmissionNetwork,
    SupplyAdmin,
    WholesaleMarket,
}

/// Everything between `Your Electricity Charges` and `Your Total Electricity Charges`.
#[derive(Default)]
struct Charges {
    on_peak_kwh: f64,
    mid_peak_kwh: f64,
    off_peak_kwh: f64,
    on_peak_cost: f64,
    mid_peak_cost: f64,
    off_peak_cost: f64,
    customer_charges: f64,
    distribution_charges: f64,
    transmission_connection: f64,
    transmission_network: f64,
    supply_admin: f64,
    wholesale_market: f64,
    total: f64,
}

impl Charges {
    fn read(lines: &[Line]) -> Result<Charges, Box<dyn Error>> {
        let rows: Vec<Vec<&Fragment>> = lines
            .iter()
            .map(|line| line.left_of(CHARGE_COLUMN_RIGHT))
            .filter(|row| !row.is_empty())
            .collect();
        let start = row_labelled(&rows, "Your Electricity Charges")?;
        let end = row_labelled(&rows, "Your Total Electricity Charges")?;
        if end <= start {
            return Err("the total comes before the charges it totals".into());
        }

        let mut charges = Charges {
            total: money(amount(&rows[end]).ok_or("no total electricity charges amount")?)?,
            ..Charges::default()
        };
        let mut rate_line_for: Option<RateLineFor> = None;

        for row in &rows[start + 1..end] {
            let label = row[0].text.trim();
            let amount = amount(row);

            // `13,240.523 kWh On-peak @ $0.158 / kWh`. The Wholesale Market rate line also reads
            // as "<number> kWh <something>", so the season word has to be one of the three.
            if let Some((used, rest)) = label.split_once(" kWh ")
                && let Some(period) = rest.split(" @ ").next()
                && matches!(period, "On-peak" | "Mid-peak" | "Off-peak")
            {
                let used = money(used)?;
                let cost = money(amount.ok_or_else(|| missing_amount(row))?)?;
                match period {
                    "On-peak" => {
                        charges.on_peak_kwh += used;
                        charges.on_peak_cost += cost;
                    }
                    "Mid-peak" => {
                        charges.mid_peak_kwh += used;
                        charges.mid_peak_cost += cost;
                    }
                    _ => {
                        charges.off_peak_kwh += used;
                        charges.off_peak_cost += cost;
                    }
                }
                continue;
            }

            match label {
                "Customer Charges" => {
                    charges.customer_charges += money(amount.ok_or_else(|| missing_amount(row))?)?;
                }
                "Distribution Charges" => {
                    charges.distribution_charges +=
                        money(amount.ok_or_else(|| missing_amount(row))?)?;
                }
                "Transmission Connection Charge" => {
                    rate_line_for = Some(RateLineFor::TransmissionConnection);
                }
                "Transmission Network Charge" => {
                    rate_line_for = Some(RateLineFor::TransmissionNetwork);
                }
                "Standard Supply Service Administrative Charge" => {
                    rate_line_for = Some(RateLineFor::SupplyAdmin);
                }
                "Wholesale Market Service Charge" => {
                    rate_line_for = Some(RateLineFor::WholesaleMarket);
                }
                // Section headings, and the line naming the distributor.
                "Electricity" | "Delivery" | "Regulatory Charges" => {}
                _ if label.starts_with("Time of use") => {}
                _ if label.starts_with("Electricity distributed by") => {}
                // `140.640 kW at $3.1008 per kW per 30 Days`, or `at $0.25 per 30 Days` where the
                // charge is a flat one. Time-of-Use lines write `@ $`, so they cannot land here.
                _ if label.starts_with("at $") || label.contains(" at $") => {
                    let target = rate_line_for.take().ok_or_else(|| {
                        format!("rate line with no charge above it: {}", row_text(row))
                    })?;
                    let value = money(amount.ok_or_else(|| missing_amount(row))?)?;
                    match target {
                        RateLineFor::TransmissionConnection => {
                            charges.transmission_connection += value;
                        }
                        RateLineFor::TransmissionNetwork => charges.transmission_network += value,
                        RateLineFor::SupplyAdmin => charges.supply_admin += value,
                        RateLineFor::WholesaleMarket => charges.wholesale_market += value,
                    }
                }
                // Rules and other decoration. A line with no figure on it cannot be a charge.
                _ if amount.is_none() => {}
                _ => return Err(format!("unrecognised charge line: {}", row_text(row)).into()),
            }
        }
        Ok(charges)
    }
}

/// The two rows of the `Your Electricity Usage` table.
struct Usage {
    reading_period_from: Date,
    reading_period_to: Date,
    number_of_days: u8,
    kwh_used: f64,
    loss_factor_adjustment: f64,
    adjusted_kwh_used: f64,
    peak_kw: f64,
    adj_peak_kw: f64,
    demand_kw: f64,
    demand_kva: f64,
    metering_adj: f64,
    adj_kw: f64,
    adj_kva: f64,
}

impl Usage {
    fn read(lines: &[Line]) -> Result<Usage, Box<dyn Error>> {
        let meter = lines
            .iter()
            .find(|line| {
                line.fragments
                    .iter()
                    .any(|f| reading_period(&f.text).is_some())
            })
            .ok_or("no meter reading period on any page")?;
        // Meter Number, Meter Reading Period, Number of Days, Unit Self-Contained Number, kWh
        // Used, Loss Factor Adjustment, Adjusted kWh Used. The meter number and the unit count
        // are read past: the struct carries neither.
        let [_, period, days, _, used, loss_factor, adjusted] = meter.fragments.as_slice() else {
            return Err(format!(
                "meter reading row has {} values, expected 7: {}",
                meter.fragments.len(),
                meter.text()
            )
            .into());
        };
        let (from, to) = reading_period(&period.text).expect("just matched");

        // Peak kW 7-7, Adj. Peak kW 7-7, Demand kW, Demand kVA, Metering Adj., Adj. kW, Adj. kVA.
        // The heading spans three lines of its own; the figures are the first row of seven
        // numbers below it.
        let header = lines
            .iter()
            .position(|line| line.fragments.iter().any(|f| f.text.trim() == "Peak kW"))
            .ok_or("no demand table heading")?;
        let demand = lines[header..]
            .iter()
            .find(|line| {
                line.fragments.len() == 7 && line.fragments.iter().all(|f| money(&f.text).is_ok())
            })
            .ok_or("no row of demand figures below the demand table heading")?;
        let [
            peak_kw,
            adj_peak_kw,
            demand_kw,
            demand_kva,
            metering_adj,
            adj_kw,
            adj_kva,
        ] = demand.fragments.as_slice()
        else {
            unreachable!("just matched a row of seven")
        };

        Ok(Usage {
            reading_period_from: from,
            reading_period_to: to,
            number_of_days: days
                .text
                .trim()
                .parse()
                .map_err(|_| format!("number of days is not a whole number: {}", days.text))?,
            kwh_used: money(&used.text)?,
            loss_factor_adjustment: money(&loss_factor.text)?,
            adjusted_kwh_used: money(&adjusted.text)?,
            peak_kw: money(&peak_kw.text)?,
            adj_peak_kw: money(&adj_peak_kw.text)?,
            demand_kw: money(&demand_kw.text)?,
            demand_kva: money(&demand_kva.text)?,
            metering_adj: money(&metering_adj.text)?,
            adj_kw: money(&adj_kw.text)?,
            adj_kva: money(&adj_kva.text)?,
        })
    }
}

/// The index of the row whose leftmost text is `label`.
fn row_labelled(rows: &[Vec<&Fragment>], label: &str) -> Result<usize, Box<dyn Error>> {
    rows.iter()
        .position(|row| row[0].text.trim() == label)
        .ok_or_else(|| format!("no line labelled {label:?}").into())
}

/// The rightmost text on a row, when the row holds more than the label alone.
fn amount<'a>(row: &[&'a Fragment]) -> Option<&'a str> {
    (row.len() >= 2).then(|| row[row.len() - 1].text.trim())
}

fn missing_amount(row: &[&Fragment]) -> String {
    format!("no amount on charge line: {}", row_text(row))
}

fn row_text(row: &[&Fragment]) -> String {
    row.iter()
        .map(|f| f.text.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text immediately to the right of the run reading exactly `label`.
fn value_after<'a>(lines: &'a [Line], label: &str) -> Result<&'a str, Box<dyn Error>> {
    value_matching(lines, label, |text| text == label)
}

/// The text immediately to the right of the run starting with `prefix`.
///
/// The H.S.T. line carries the registration number inside the label, so it cannot be matched whole
/// without pinning the parse to one account.
fn value_after_prefix<'a>(lines: &'a [Line], prefix: &str) -> Result<&'a str, Box<dyn Error>> {
    value_matching(lines, prefix, |text| text.starts_with(prefix))
}

fn value_matching<'a>(
    lines: &'a [Line],
    what: &str,
    matches: impl Fn(&str) -> bool,
) -> Result<&'a str, Box<dyn Error>> {
    lines
        .iter()
        .find_map(|line| {
            let at = line.fragments.iter().position(|f| matches(f.text.trim()))?;
            Some(line.fragments.get(at + 1)?.text.trim())
        })
        .ok_or_else(|| format!("no value beside {what:?}").into())
}

/// A number as the bill writes it: thousands separated by commas, sometimes led by a dollar sign.
fn money(text: &str) -> Result<f64, Box<dyn Error>> {
    let text = text.trim();
    text.trim_start_matches('$')
        .replace(',', "")
        .parse()
        .map_err(|_| format!("not a number: {text:?}").into())
}

/// `Jan 28 2026`, or `JUN 23 2025` as the usage table writes it.
fn date(text: &str) -> Result<Date, Box<dyn Error>> {
    let [month, day, year] = text.split_whitespace().collect::<Vec<_>>()[..] else {
        return Err(format!("not a date: {text:?}").into());
    };
    let month = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(month))
        .ok_or_else(|| format!("not a month name: {month:?}"))?;
    let day: i8 = day.parse().map_err(|_| format!("not a day: {day:?}"))?;
    let year: i16 = year.parse().map_err(|_| format!("not a year: {year:?}"))?;
    Ok(Date::new(year, month as i8 + 1, day)?)
}

/// The two dates of a `JUN 23 2025 TO JUL 23 2025` meter reading period.
///
/// Returns `None` for anything else, which is how the usage table's row is picked out of the
/// document without depending on which page it landed on.
fn reading_period(text: &str) -> Option<(Date, Date)> {
    let (from, to) = text.trim().split_once(" TO ")?;
    Some((date(from).ok()?, date(to).ok()?))
}

#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date as civil_date;

    #[test]
    fn amounts_shed_their_dollar_signs_and_thousands_separators() {
        assert_eq!(money("10,403.82").unwrap(), 10_403.82);
        assert_eq!(money(" $10,393.42 ").unwrap(), 10_393.42);
        assert_eq!(money("1").unwrap(), 1.0);
        assert!(money("CR").is_err());
    }

    #[test]
    fn dates_read_in_either_case_the_bill_uses() {
        assert_eq!(date("Jan 28 2026").unwrap(), civil_date(2026, 1, 28));
        assert_eq!(date("JUN 23 2025").unwrap(), civil_date(2025, 6, 23));
        assert!(date("Smarch 1 2026").is_err());
        assert!(date("Jan 2026").is_err());
    }

    #[test]
    fn a_reading_period_yields_both_of_its_dates() {
        assert_eq!(
            reading_period("JUN 23 2025 TO JUL 23 2025").unwrap(),
            (civil_date(2025, 6, 23), civil_date(2025, 7, 23))
        );
    }

    #[test]
    fn anything_that_is_not_a_reading_period_is_passed_over() {
        assert!(reading_period("Meter Reading Period").is_none());
        assert!(reading_period("40004253").is_none());
        assert!(reading_period("").is_none());
    }
}
