//! EV delivery cost for one billing period, from a Toronto Hydro bill, a Green Button export and
//! the Evolute session reports covering the period's two ends.

use ev_cost_recovery::io::peak_power_cost;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
peak_power_cost_cli -- the delivery cost attributable to EV charging in one billing period.

Prices the EV share of each demand-priced delivery line at the bill's own rate for that line. Each
line is levied on one demand figure, and each demand figure is a maximum over one interval: the
hour the site's kVA peaked in, the hour its kW peaked in, and the hour its kW peaked in within
07:00-19:00. Every rate and proportion comes off the bill; no tariff is assumed.

Only the demand-priced lines are attributed. Consumption is billed by the kilowatt-hour and the
customer charge is fixed, so neither turns on which interval the site peaked in. For the
consumption side, see energy_cost_cli.

No closing date is asked for. The bill states which period it covers.

Two session reports are asked for because a billing period straddles two calendar months, and an
Evolute session report covers one. Give the one covering the start of the period first and the one
covering its end second.

The report is written to stdout as markdown that also reads as plain text.

Usage:
    peak_power_cost_cli <BILL.pdf> <GREEN_BUTTON.XML> <SESSIONS_1.csv> <SESSIONS_2.csv>
    peak_power_cost_cli --help

Example:
    peak_power_cost_cli data/bills/TH_2026_06_29.pdf data/TH_Electric_Usage.XML \\
        data/May.csv data/June.csv
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let [bill_pdf, gb_xml, session_csv1, session_csv2] = args.as_slice() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };

    match run(
        Path::new(bill_pdf),
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
    bill_pdf: &Path,
    gb_xml: &Path,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<(), Box<dyn Error>> {
    print!(
        "{}",
        peak_power_cost(bill_pdf, gb_xml, session_csv1, session_csv2)?
    );
    Ok(())
}
