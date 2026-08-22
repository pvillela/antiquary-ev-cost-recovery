//! EV energy cost for one billing period, from a Toronto Hydro bill and the Evolute session reports
//! covering the period's two ends.

use ev_cost_recovery::io::energy_cost;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
energy_cost_cli -- the energy cost attributable to EV charging in one billing period.

Prices the EV share of each time-of-use band at the bill's own rate for that band. Every rate and
proportion comes off the bill; no tariff is assumed.

No meter export is asked for. Consumption is billed by the kilowatt-hour, so the hour the site
peaked in does not bear on this figure. For the delivery lines, which do, see peak_power_cost_cli.

No closing date is asked for either. The bill states which period it covers.

Two session reports are asked for because a billing period straddles two calendar months, and an
Evolute session report covers one. Give the one covering the start of the period first and the one
covering its end second.

The report is written to stdout as markdown that also reads as plain text.

Usage:
    energy_cost_cli <BILL.pdf> <SESSIONS_1.csv> <SESSIONS_2.csv>
    energy_cost_cli --help

Example:
    energy_cost_cli data/bills/TH_2026_06_29.pdf data/May.csv data/June.csv
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let [bill_pdf, session_csv1, session_csv2] = args.as_slice() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };

    match run(
        Path::new(bill_pdf),
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

fn run(bill_pdf: &Path, session_csv1: &Path, session_csv2: &Path) -> Result<(), Box<dyn Error>> {
    print!("{}", energy_cost(bill_pdf, session_csv1, session_csv2)?);
    Ok(())
}
