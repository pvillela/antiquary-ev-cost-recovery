//! Peak power estimates for one billing period, from a Green Button export and the Evolute session
//! reports covering the period's two ends.

use ev_cost_recovery::io::{PowerEstimates, peak_power};
use jiff::civil::Date;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
peak_power_cli -- peak power estimates for one billing period.

Reports the intervals of interest that maximize kW and kVA in the billing period, one report for
each. A billing period is named by the date it closes on.

Two session reports are asked for because a billing period straddles two calendar months, and an
Evolute session report covers one. Give the one covering the start of the period first and the one
covering its end second.

The reports are written to stdout as markdown that also reads as plain text.

Usage:
    peak_power_cli <YYYY-MM-DD> <GREEN_BUTTON.XML> <SESSIONS_1.csv> <SESSIONS_2.csv>
    peak_power_cli --help

Example:
    peak_power_cli 2026-06-22 data/TH_Electric_Usage.XML data/May.csv data/June.csv
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let [ending, gb_xml, session_csv1, session_csv2] = args.as_slice() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };

    match run(
        ending,
        Path::new(gb_xml),
        Path::new(session_csv1),
        Path::new(session_csv2),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    ending: &str,
    gb_xml: &Path,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<(), Box<dyn Error>> {
    // The closing date is read before anything else, so omitting it is reported as a date that
    // cannot be read rather than as a missing file. All four arguments are positional and three of
    // them are paths, so a shifted argument list is otherwise hard to tell from a typo.
    let billing_period_ending: Date = ending.parse().map_err(|e| {
        format!("cannot read \"{ending}\" as the billing period's closing date, YYYY-MM-DD: {e}")
    })?;

    let PowerEstimates {
        kw_estimates,
        kva_estimates,
    } = peak_power(billing_period_ending, gb_xml, session_csv1, session_csv2)?;

    // Each report carries its own heading, so the label above it says only which of the two it is.
    println!("Billing period ending {billing_period_ending} -- interval maximizing kW\n");
    print!("{kw_estimates}");
    println!("\nBilling period ending {billing_period_ending} -- interval maximizing kVA\n");
    print!("{kva_estimates}");
    Ok(())
}
