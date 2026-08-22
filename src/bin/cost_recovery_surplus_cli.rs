//! What the EV cost-recovery rates recover for one billing period, less what the chargers' share of
//! the Toronto Hydro bill cost, from the bill, a Green Button export and the two session reports
//! covering the period's two ends.

use ev_cost_recovery::io::{CostRecoveryRates, cost_recovery_surplus};
use jiff::civil::Date;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
cost_recovery_surplus_cli -- EV cost recovery against EV cost, for one billing period.

Reports what the EV cost-recovery rates recover, what the chargers' share of the bill cost, and the
difference. A positive surplus means the rates covered that share; a negative one means they fell
short.

The three parts are printed in full beneath the summary, so every figure in the subtraction can be
checked against the report it came from.

No closing date is asked for: the bill states which period it covers, and everything else is
fetched for that period.

A billing period straddles two calendar months, and an Evolute session report covers one month, so
two are asked for. Give the one covering the start of the period first and the one covering its end
second.

A rate schedule is written EFFECTIVE_DATE:ON_PEAK,MID_PEAK,OFF_PEAK. Give one schedule for a period
the rates held through, and two when they changed during it -- the first being the rates in effect
on the period's first day, whose effective date may be well before the period, and the second the
rates it changed to. A change takes effect at local midnight starting its date.

Only the delivery and energy sides of the bill are counted as EV cost. The customer charge, the
standard supply administration charge and the wholesale market service charge are in neither, so a
surplus of zero is not the same as breaking even on the whole invoice.

The report is written to stdout as markdown that also reads as plain text.

Usage:
    cost_recovery_surplus_cli <BILL.pdf> <GREEN_BUTTON.XML> <RATES> [RATES] \\
        <SESSIONS_1.csv> <SESSIONS_2.csv>
    cost_recovery_surplus_cli --help

Examples:
    cost_recovery_surplus_cli data/June.pdf data/TH_Electric_Usage.XML \\
        2026-05-01:0.1100,0.0900,0.0700 data/May.csv data/June.csv

    cost_recovery_surplus_cli data/June.pdf data/TH_Electric_Usage.XML \\
        2026-05-01:0.1100,0.0900,0.0700 2026-06-01:0.1200,0.1000,0.0800 \\
        data/May.csv data/June.csv
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    // The optional schedule sits in the middle, so the two shapes are told apart by how many
    // arguments there are rather than by looking at any one of them. Both session reports are last,
    // which is what keeps that unambiguous.
    let (bill, gb_xml, rates1, rates2, csv1, csv2) = match args.as_slice() {
        [bill, gb, r1, c1, c2] => (bill, gb, r1, None, c1, c2),
        [bill, gb, r1, r2, c1, c2] => (bill, gb, r1, Some(r2), c1, c2),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(
        Path::new(bill),
        Path::new(gb_xml),
        rates1,
        rates2.map(String::as_str),
        Path::new(csv1),
        Path::new(csv2),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    bill_pdf: &Path,
    gb_xml: &Path,
    rates1: &str,
    rates2: Option<&str>,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<(), Box<dyn Error>> {
    // The rates are read before anything is opened, so a typo in one is reported as such rather
    // than after a bill and a year of meter readings have been parsed.
    let recovery_rates_at_start = parse_rates(rates1)?;
    let recovery_rates_at_end = rates2.map(parse_rates).transpose()?;

    let surplus = cost_recovery_surplus(
        bill_pdf,
        gb_xml,
        session_csv1,
        session_csv2,
        recovery_rates_at_start,
        recovery_rates_at_end,
    )?;

    print!("{surplus}");
    Ok(())
}

/// One rate schedule, written `EFFECTIVE_DATE:ON_PEAK,MID_PEAK,OFF_PEAK`.
///
/// One argument rather than four, so that the effective date cannot drift away from the rates it
/// belongs to when a second schedule is added to the command line.
fn parse_rates(spec: &str) -> Result<CostRecoveryRates, String> {
    const SHAPE: &str = "expected EFFECTIVE_DATE:ON_PEAK,MID_PEAK,OFF_PEAK, \
                         as in 2026-05-01:0.1100,0.0900,0.0700";

    let (date, rates) = spec
        .split_once(':')
        .ok_or_else(|| format!("cannot read \"{spec}\" as a rate schedule: {SHAPE}"))?;

    let effective_date: Date = date
        .parse()
        .map_err(|e| format!("cannot read \"{date}\" as an effective date, YYYY-MM-DD: {e}"))?;

    let [on_peak, mid_peak, off_peak] = rates.split(',').collect::<Vec<_>>()[..] else {
        return Err(format!("\"{rates}\" is not three rates: {SHAPE}"));
    };
    let rate = |s: &str, band: &str| -> Result<f64, String> {
        s.parse()
            .map_err(|e| format!("cannot read \"{s}\" as the {band} rate: {e}"))
    };

    Ok(CostRecoveryRates {
        effective_date,
        on_peak: rate(on_peak, "on-peak")?,
        mid_peak: rate(mid_peak, "mid-peak")?,
        off_peak: rate(off_peak, "off-peak")?,
    })
}
