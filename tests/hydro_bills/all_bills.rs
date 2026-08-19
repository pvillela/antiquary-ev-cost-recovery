//! Slow-tier check against every real bill in `data/hydro_bills`.
//!
//! Ignored by default: the bills are not in the repository, and reading two dozen 3 MB PDFs is not
//! something to put on every `cargo test`. Run explicitly with:
//!
//! ```text
//! cargo test --test integration -- hydro_bills::all_bills --ignored --nocapture
//! ```
//!
//! Parsing without error is the weaker half of what this checks. The stronger half is that the
//! figures agree with each other: a label matched to the wrong column, or a charge line quietly
//! dropped because a rate change printed it twice, still parses -- it just gives a total that no
//! longer equals its parts.

use ev_cost_recovery::hydro_bills::hydro_bill::HydroBill;
use ev_cost_recovery::hydro_bills::pdf_text::{self, Line};
use std::path::PathBuf;

/// Money is stated to the cent, and the sums here are of a handful of terms.
const CENT: f64 = 0.005;

/// Consumption is stated to a thousandth of a kilowatt-hour, and the sums below are over four
/// separately rounded figures, so they can differ in the last couple of those thousandths.
const KWH_ROUNDING: f64 = 0.01;

fn bills_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/hydro_bills")
}

#[test]
#[ignore = "reads every bill PDF in data/hydro_bills"]
fn every_bill_parses_and_its_figures_agree_with_each_other() {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(bills_dir())
        .expect(
            "the sample bills are not in the repository: put the Toronto Hydro PDFs in \
             data/hydro_bills before running this",
        )
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no PDFs in {:?}", bills_dir());

    for path in &paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bill = match HydroBill::from_pdf(path) {
            Ok(bill) => bill,
            Err(e) => panic!("{name}: {e}"),
        };
        println!("{name}: {bill:?}");

        let charges = bill.on_peak_cost
            + bill.mid_peak_cost
            + bill.off_peak_cost
            + bill.delivery_customer_charges
            + bill.distribution_charges
            + bill.transmission_connection_charge
            + bill.transmission_network_charge
            + bill.standard_supply_admin_charge
            + bill.wholesale_market_svc_charge;
        close(
            &name,
            "charge lines vs. total electricity charges",
            charges,
            bill.total_electricity_charges,
            CENT,
        );

        // `bill_total_amount` is this period's own total, so it comes up short of the bill's
        // printed `Amount Due` by exactly whatever was still owed from the bill before. On all but
        // one of these bills that balance is zero and the two figures are the same.
        let lines: Vec<Line> = pdf_text::read_pages(path)
            .expect("already read once")
            .into_iter()
            .flatten()
            .collect();
        close(
            &name,
            "this period's total plus the balance forward vs. the printed Amount Due",
            bill.bill_total_amount + printed(&lines, "Balance Forward"),
            printed(&lines, "Amount Due"),
            CENT,
        );

        // Time-of-Use is billed on the loss-adjusted kilowatt-hours, not the metered ones, so this
        // also confirms that the two consumption figures did not get swapped.
        close(
            &name,
            "Time-of-Use kWh vs. adjusted kWh used",
            bill.on_peak_kwh + bill.mid_peak_kwh + bill.off_peak_kwh,
            bill.adjusted_kwh_used,
            KWH_ROUNDING,
        );
        close(
            &name,
            "kWh used times loss factor vs. adjusted kWh used",
            bill.kwh_used * bill.loss_factor_adjustment,
            bill.adjusted_kwh_used,
            KWH_ROUNDING,
        );

        assert_eq!(
            bill.meter_reading_period_from
                .until(bill.meter_reading_period_to)
                .unwrap()
                .get_days(),
            i32::from(bill.number_of_days),
            "{name}: meter reading period length vs. number of days"
        );
        assert_eq!(
            bill.period_end_date, bill.meter_reading_period_to,
            "{name}: period end date"
        );
        assert!(
            bill.statement_date >= bill.meter_reading_period_to,
            "{name}: statement dated {} before the period ended on {}",
            bill.statement_date,
            bill.meter_reading_period_to
        );
        // A bill is issued monthly, so the period is a month give or take a few days.
        assert!(
            (28..=31).contains(&bill.number_of_days),
            "{name}: {} days in the period",
            bill.number_of_days
        );
        // Every figure on these bills is positive; a sign that flips means a column was misread.
        for (what, value) in [
            ("peak kW", bill.peak_kw),
            ("adjusted peak kW", bill.adj_peak_kw),
            ("demand kW", bill.demand_kw),
            ("demand kVA", bill.demand_kva),
            ("metering adjustment", bill.metering_adj),
            ("adjusted kW", bill.adj_kw),
            ("adjusted kVA", bill.adj_kva),
            ("H.S.T.", bill.hst),
            (
                "Ontario Electricity Rebate",
                bill.ontario_electricity_rebate,
            ),
        ] {
            assert!(value > 0.0, "{name}: {what} is {value}");
        }
        // Apparent power is never less than real power.
        assert!(
            bill.demand_kva >= bill.demand_kw,
            "{name}: {} kVA is below {} kW",
            bill.demand_kva,
            bill.demand_kw
        );
    }

    println!("{} bills read", paths.len());
}

/// The figure printed beside `label` on the bill, read from the PDF rather than from the parse.
fn printed(lines: &[Line], label: &str) -> f64 {
    let text = lines
        .iter()
        .find_map(|line| {
            let at = line.fragments.iter().position(|f| f.text.trim() == label)?;
            Some(line.fragments.get(at + 1)?.text.trim())
        })
        .unwrap_or_else(|| panic!("no figure beside {label:?}"));
    text.trim_start_matches('$')
        .replace(',', "")
        .parse()
        .unwrap_or_else(|_| panic!("{label}: not a number: {text:?}"))
}

fn close(name: &str, what: &str, left: f64, right: f64, tolerance: f64) {
    assert!(
        (left - right).abs() <= tolerance,
        "{name}: {what}: {left} vs. {right}"
    );
}
