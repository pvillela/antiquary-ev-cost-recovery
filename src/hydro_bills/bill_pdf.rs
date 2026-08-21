//! Reading a Toronto Hydro bill PDF into a [`HydroBill`].
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
//!
//! [`super::pdf_text`] supplies the positioned text and knows nothing of bills; everything that
//! knows what a Toronto Hydro bill looks like is here.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use jiff::civil::Date;

use crate::hydro_bills::hydro_bill::HydroBill;
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
/// Reads a Toronto Hydro bill PDF file and returns a [`HydroBill`]. Values with the same label
/// that appear more than once in the PDF are added together and shown as a single value in the
/// data structure.
///
/// # Errors
///
/// Returns a [`BillError`] naming the file and what went wrong with it. The variants separate the
/// two failures that call for different things: the file never gave up its text, or it did and the
/// bill is not laid out the way this reads one. [`BillError::is_layout`] asks that question
/// directly.
///
/// A charge line the parser does not recognise is one of the errors, rather than a figure quietly
/// left out of the totals: these numbers get reconciled against metered consumption, and a charge
/// that silently reads as zero is worse than no answer.
pub fn hydro_bill_from_pdf(path: &Path) -> Result<HydroBill, BillError> {
    let pages = pdf_text::read_pages(path).map_err(|source| BillError::Unreadable {
        path: path.to_path_buf(),
        source,
    })?;
    let lines: Vec<Line> = pages.into_iter().flatten().collect();
    from_lines(&lines).map_err(|problem| problem.at(path))
}

/// The parse proper, over the lines of the whole document in reading order.
fn from_lines(lines: &[Line]) -> Result<HydroBill, Problem> {
    let usage = Usage::read(lines)?;
    let charges = Charges::read(lines)?;
    let hst = money(value_after_prefix(lines, "H.S.T.")?)?;
    // Printed as a credit, with `CR` in the column to its right, and held here as the positive
    // amount taken off the bill.
    let rebate = money(value_after(lines, "Ontario Electricity Rebate")?)?;

    Ok(HydroBill {
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

        meter_reading_period_from: usage.reading_period_from,
        meter_reading_period_to: usage.reading_period_to,
        number_of_days: usage.number_of_days,
        kwh_used: usage.kwh_used,
        loss_factor_adjustment: usage.loss_factor_adjustment,
        adjusted_kwh_used: usage.adjusted_kwh_used,
        peak_7_7_kw: usage.peak_kw,
        adj_peak_7_7_kw: usage.adj_peak_kw,
        demand_kw: usage.demand_kw,
        demand_kva: usage.demand_kva,
        metering_adj: usage.metering_adj,
        adj_kw: usage.adj_kw,
        adj_kva: usage.adj_kva,
    })
}

/// Why a bill PDF did not read into a [`HydroBill`].
///
/// The variants are apart because they ask different things of whoever hit them.
/// [`BillError::Unreadable`] is about the file: the wrong one was named, or the download is
/// damaged. Every other variant means the file read perfectly well and the bill is not laid out
/// the way the bills this was written against are -- something to look at with
/// `hydro_bill_dump --lines`, and most likely something to change here. [`BillError::is_layout`]
/// is that distinction as a single question.
///
/// Every variant names the file, so an error printed on its own says which bill it came from.
#[derive(Debug)]
pub enum BillError {
    /// The document's text could not be read at all: the file is missing, is not a PDF, or its
    /// fonts carry no `ToUnicode` CMap to decode their glyph codes with. Nothing about the bill
    /// was reached, so this says nothing either way about whether the bill is one this
    /// understands.
    Unreadable {
        path: PathBuf,
        source: Box<dyn Error>,
    },

    /// A section or figure that every bill carries was not on the page. `what` names it as a noun
    /// phrase: `line labelled "Your Electricity Charges"`.
    Missing { path: PathBuf, what: String },

    /// The section is on the page, but not in the shape it is read in -- a table row of the wrong
    /// width, or a total printed above the charges it totals.
    Shape { path: PathBuf, what: String },

    /// A charge line turned up that there is no rule for, given whole in `line`.
    UnknownCharge { path: PathBuf, line: String },

    /// Text that should be a number or a date is neither. `what` is what it should have been:
    /// `a number`, `a date`, `a month name`.
    Malformed {
        path: PathBuf,
        what: String,
        text: String,
    },
}

impl BillError {
    /// The bill this came from.
    pub fn path(&self) -> &Path {
        match self {
            BillError::Unreadable { path, .. }
            | BillError::Missing { path, .. }
            | BillError::Shape { path, .. }
            | BillError::UnknownCharge { path, .. }
            | BillError::Malformed { path, .. } => path,
        }
    }

    /// Whether the bill's layout is what failed, as opposed to the file itself.
    ///
    /// True is the actionable case: the PDF gave up its text, so the text is there to look at, and
    /// what it says is not what this expects. False means there is no text to look at and the
    /// thing to check is the file.
    pub fn is_layout(&self) -> bool {
        !matches!(self, BillError::Unreadable { .. })
    }
}

impl fmt::Display for BillError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // `pdf_text::read_pages` already names the file, and often the page as well.
            BillError::Unreadable { source, .. } => write!(f, "{source}"),
            BillError::Missing { path, what } => {
                write!(f, "{}: the bill has no {what}", path.display())
            }
            BillError::Shape { path, what } => write!(f, "{}: {what}", path.display()),
            BillError::UnknownCharge { path, line } => {
                write!(f, "{}: unrecognised charge line: {line}", path.display())
            }
            BillError::Malformed { path, what, text } => {
                write!(f, "{}: not {what}: {text:?}", path.display())
            }
        }
    }
}

impl Error for BillError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            BillError::Unreadable { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

/// A [`BillError`] before it knows which file it came from.
///
/// The parse works over lines rather than over a file, and so has no path to name. `from_pdf`
/// attaches one on the way out. Keeping the two types apart is what stops the parse from having to
/// thread a `&Path` through every helper for the sake of an error that is usually not raised.
#[derive(Debug)]
enum Problem {
    Missing(String),
    Shape(String),
    UnknownCharge(String),
    Malformed { what: String, text: String },
}

impl Problem {
    fn at(self, path: &Path) -> BillError {
        let path = path.to_path_buf();
        match self {
            Problem::Missing(what) => BillError::Missing { path, what },
            Problem::Shape(what) => BillError::Shape { path, what },
            Problem::UnknownCharge(line) => BillError::UnknownCharge { path, line },
            Problem::Malformed { what, text } => BillError::Malformed { path, what, text },
        }
    }
}

/// Something every bill carries was not found. `what` is a noun phrase: the message reads
/// "the bill has no {what}".
fn missing(what: impl Into<String>) -> Problem {
    Problem::Missing(what.into())
}

/// It was found, but not in the shape it is read in. `what` is a whole sentence.
fn shape(what: impl Into<String>) -> Problem {
    Problem::Shape(what.into())
}

/// `text` is not the `what` it should be: the message reads "not {what}: {text:?}".
fn malformed(what: &str, text: &str) -> Problem {
    Problem::Malformed {
        what: what.to_owned(),
        text: text.trim().to_owned(),
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
    fn read(lines: &[Line]) -> Result<Charges, Problem> {
        let rows: Vec<Vec<&Fragment>> = lines
            .iter()
            .map(|line| line.left_of(CHARGE_COLUMN_RIGHT))
            .filter(|row| !row.is_empty())
            .collect();
        let start = row_labelled(&rows, "Your Electricity Charges")?;
        let end = row_labelled(&rows, "Your Total Electricity Charges")?;
        if end <= start {
            return Err(shape("the total comes before the charges it totals"));
        }

        let mut charges = Charges {
            total: money(
                amount(&rows[end])
                    .ok_or_else(|| missing("amount on the total electricity charges line"))?,
            )?,
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
                        shape(format!(
                            "rate line with no charge above it: {}",
                            row_text(row)
                        ))
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
                _ => return Err(Problem::UnknownCharge(row_text(row))),
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
    fn read(lines: &[Line]) -> Result<Usage, Problem> {
        let meter = lines
            .iter()
            .find(|line| {
                line.fragments
                    .iter()
                    .any(|f| reading_period(&f.text).is_some())
            })
            .ok_or_else(|| missing("meter reading period on any page"))?;
        // Meter Number, Meter Reading Period, Number of Days, Unit Self-Contained Number, kWh
        // Used, Loss Factor Adjustment, Adjusted kWh Used. The meter number and the unit count
        // are read past: the struct carries neither.
        let [_, period, days, _, used, loss_factor, adjusted] = meter.fragments.as_slice() else {
            return Err(shape(format!(
                "meter reading row has {} values, expected 7: {}",
                meter.fragments.len(),
                meter.text()
            )));
        };
        let (from, to) = reading_period(&period.text).expect("just matched");

        // Peak kW 7-7, Adj. Peak kW 7-7, Demand kW, Demand kVA, Metering Adj., Adj. kW, Adj. kVA.
        // The heading spans three lines of its own; the figures are the first row of seven
        // numbers below it.
        let header = lines
            .iter()
            .position(|line| line.fragments.iter().any(|f| f.text.trim() == "Peak kW"))
            .ok_or_else(|| missing("demand table heading"))?;
        let demand = lines[header..]
            .iter()
            .find(|line| {
                line.fragments.len() == 7 && line.fragments.iter().all(|f| money(&f.text).is_ok())
            })
            .ok_or_else(|| missing("row of demand figures below the demand table heading"))?;
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
                .map_err(|_| malformed("a whole number of days", &days.text))?,
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
fn row_labelled(rows: &[Vec<&Fragment>], label: &str) -> Result<usize, Problem> {
    rows.iter()
        .position(|row| row[0].text.trim() == label)
        .ok_or_else(|| missing(format!("line labelled {label:?}")))
}

/// The rightmost text on a row, when the row holds more than the label alone.
fn amount<'a>(row: &[&'a Fragment]) -> Option<&'a str> {
    (row.len() >= 2).then(|| row[row.len() - 1].text.trim())
}

fn missing_amount(row: &[&Fragment]) -> Problem {
    missing(format!("amount on charge line: {}", row_text(row)))
}

fn row_text(row: &[&Fragment]) -> String {
    row.iter()
        .map(|f| f.text.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text immediately to the right of the run reading exactly `label`.
fn value_after<'a>(lines: &'a [Line], label: &str) -> Result<&'a str, Problem> {
    value_matching(lines, label, |text| text == label)
}

/// The text immediately to the right of the run starting with `prefix`.
///
/// The H.S.T. line carries the registration number inside the label, so it cannot be matched whole
/// without pinning the parse to one account.
fn value_after_prefix<'a>(lines: &'a [Line], prefix: &str) -> Result<&'a str, Problem> {
    value_matching(lines, prefix, |text| text.starts_with(prefix))
}

fn value_matching<'a>(
    lines: &'a [Line],
    what: &str,
    matches: impl Fn(&str) -> bool,
) -> Result<&'a str, Problem> {
    lines
        .iter()
        .find_map(|line| {
            let at = line.fragments.iter().position(|f| matches(f.text.trim()))?;
            Some(line.fragments.get(at + 1)?.text.trim())
        })
        .ok_or_else(|| missing(format!("value beside {what:?}")))
}

/// A number as the bill writes it: thousands separated by commas, sometimes led by a dollar sign.
fn money(text: &str) -> Result<f64, Problem> {
    let text = text.trim();
    text.trim_start_matches('$')
        .replace(',', "")
        .parse()
        .map_err(|_| malformed("a number", text))
}

/// `Jan 28 2026`, or `JUN 23 2025` as the usage table writes it.
fn date(text: &str) -> Result<Date, Problem> {
    let [month, day, year] = text.split_whitespace().collect::<Vec<_>>()[..] else {
        return Err(malformed("a date", text));
    };
    let month = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(month))
        .ok_or_else(|| malformed("a month name", month))?;
    let day: i8 = day.parse().map_err(|_| malformed("a day", day))?;
    let year: i16 = year.parse().map_err(|_| malformed("a year", year))?;
    // A day that does not exist in that month lands here -- `FEB 30 2025` reads as a date in every
    // part and is still not one.
    Date::new(year, month as i8 + 1, day).map_err(|_| malformed("a date", text))
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

    #[test]
    fn a_day_that_month_does_not_have_is_not_a_date() {
        assert!(date("FEB 30 2025").is_err());
        assert_eq!(date("FEB 29 2024").unwrap(), civil_date(2024, 2, 29));
    }

    /// Every error the parse can raise names the bill it came from, and reads as a sentence about
    /// that bill rather than as a fragment needing the caller to prefix it.
    #[test]
    fn a_problem_becomes_an_error_naming_the_bill() {
        let path = Path::new("data/hydro_bills/TH_2025_07_28.pdf");
        for (problem, message) in [
            (
                missing("line labelled \"Your Electricity Charges\""),
                "data/hydro_bills/TH_2025_07_28.pdf: the bill has no line labelled \
                 \"Your Electricity Charges\"",
            ),
            (
                shape("the total comes before the charges it totals"),
                "data/hydro_bills/TH_2025_07_28.pdf: the total comes before the charges it totals",
            ),
            (
                Problem::UnknownCharge("Rate Rider for Deferral $12.34".to_owned()),
                "data/hydro_bills/TH_2025_07_28.pdf: unrecognised charge line: \
                 Rate Rider for Deferral $12.34",
            ),
            (
                malformed("a number", " CR "),
                "data/hydro_bills/TH_2025_07_28.pdf: not a number: \"CR\"",
            ),
        ] {
            let error = problem.at(path);
            assert_eq!(error.to_string(), message);
            assert_eq!(error.path(), path);
        }
    }

    /// The question the caller actually asks: is there text to go and look at?
    #[test]
    fn everything_but_an_unreadable_file_is_a_layout_failure() {
        let path = Path::new("bill.pdf");
        assert!(missing("statement date").at(path).is_layout());
        assert!(malformed("a date", "Smarch").at(path).is_layout());
        assert!(
            !BillError::Unreadable {
                path: path.to_path_buf(),
                source: "bill.pdf: not a PDF".into(),
            }
            .is_layout()
        );
    }

    /// `pdf_text::read_pages` names the file itself, so the wrapper must not name it twice.
    #[test]
    fn an_unreadable_file_is_reported_as_the_reader_reported_it() {
        let error = BillError::Unreadable {
            path: PathBuf::from("bill.pdf"),
            source: "bill.pdf: page 2: font /F1: no ToUnicode CMap".into(),
        };
        assert_eq!(
            error.to_string(),
            "bill.pdf: page 2: font /F1: no ToUnicode CMap"
        );
        assert!(error.source().is_some());
    }
}
