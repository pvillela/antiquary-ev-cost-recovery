//! EV energy for one billing period, from the Evolute session reports covering the period's two
//! ends.

use ev_cost_recovery::io::energy;
use jiff::civil::Date;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
energy_cli -- EV energy for one billing period, split by time-of-use band.

Reports the kilowatt-hours drawn by EV charging within the period, in each of the three price
bands. A billing period is named by the date it closes on.

No meter export and no bill are asked for. Consumption is billed by the kilowatt-hour, so neither
the hour the site peaked in nor any rate on the bill bears on this figure. For what the energy
cost, see energy_cost_cli.

Two session reports are asked for because a billing period straddles two calendar months, and an
Evolute session report covers one. Give the one covering the start of the period first and the one
covering its end second.

The report is written to stdout as markdown that also reads as plain text.

Usage:
    energy_cli <YYYY-MM-DD> <SESSIONS_1.csv> <SESSIONS_2.csv>
    energy_cli --help

Example:
    energy_cli 2026-06-23 data/May.csv data/June.csv
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let [ending, session_csv1, session_csv2] = args.as_slice() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };

    match run(ending, Path::new(session_csv1), Path::new(session_csv2)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(ending: &str, session_csv1: &Path, session_csv2: &Path) -> Result<(), Box<dyn Error>> {
    // The closing date is read before anything else, so omitting it is reported as a date that
    // cannot be read rather than as a missing file. All three arguments are positional and two of
    // them are paths, so a shifted argument list is otherwise hard to tell from a typo.
    let billing_period_ending: Date = ending.parse().map_err(|e| {
        format!("cannot read \"{ending}\" as the billing period's closing date, YYYY-MM-DD: {e}")
    })?;

    print!(
        "{}",
        energy(billing_period_ending, session_csv1, session_csv2)?
    );
    Ok(())
}
