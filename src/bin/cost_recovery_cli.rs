//! Cost recovery for one billing period, at the EV cost-recovery rates given, from the Evolute
//! session reports covering the period's two ends.

use ev_cost_recovery::io::{CostRecoveryRates, cost_recovery};
use jiff::civil::Date;
use std::error::Error;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
cost_recovery_cli -- EV cost recovery for one billing period.

Applies the EV cost-recovery rates given to the energy the chargers drew in each time-of-use band,
and reports what that recovers. A billing period is named by the date it closes on.

The rates are yours rather than Toronto Hydro's, so no bill is read and no tax is added: the report
is the rate times the kilowatt-hours it was charged on.

A billing period straddles two calendar months, so two session reports are asked for and an Evolute
session report covers one month. Give the one covering the start of the period first and the one
covering its end second.

A rate schedule is written EFFECTIVE_DATE:ON_PEAK,MID_PEAK,OFF_PEAK. Give one schedule for a period
the rates held through, and two when they changed during it -- the first being the rates in effect
on the period's first day, whose effective date may be well before the period, and the second the
rates it changed to. A change takes effect at local midnight starting its date.

The report is written to stdout as markdown that also reads as plain text.

Usage:
    cost_recovery_cli <YYYY-MM-DD> <RATES> [RATES] <SESSIONS_1.csv> <SESSIONS_2.csv>
    cost_recovery_cli --help

Examples:
    cost_recovery_cli 2026-06-23 2026-05-01:0.1100,0.0900,0.0700 \\
        data/May.csv data/June.csv

    cost_recovery_cli 2026-06-23 2026-05-01:0.1100,0.0900,0.0700 \\
        2026-06-01:0.1200,0.1000,0.0800 data/May.csv data/June.csv
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    // The optional schedule sits in the middle, so the two shapes are told apart by how many
    // arguments there are rather than by looking at any one of them. Both paths are last, which is
    // what keeps that unambiguous.
    let (ending, rates1, rates2, session_csv1, session_csv2) = match args.as_slice() {
        [ending, rates1, csv1, csv2] => (ending, rates1, None, csv1, csv2),
        [ending, rates1, rates2, csv1, csv2] => (ending, rates1, Some(rates2), csv1, csv2),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(
        ending,
        rates1,
        rates2.map(String::as_str),
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
    rates1: &str,
    rates2: Option<&str>,
    session_csv1: &Path,
    session_csv2: &Path,
) -> Result<(), Box<dyn Error>> {
    // The closing date and the rates are read before anything else, so a typo in either is reported
    // as such rather than as a missing file. Every argument is positional, so a shifted argument
    // list is otherwise hard to tell from a typo.
    let billing_period_ending: Date = ending.parse().map_err(|e| {
        format!("cannot read \"{ending}\" as the billing period's closing date, YYYY-MM-DD: {e}")
    })?;
    let recovery_rates_at_start = parse_rates(rates1)?;
    let recovery_rates_at_end = rates2.map(parse_rates).transpose()?;

    let recovery = cost_recovery(
        billing_period_ending,
        session_csv1,
        session_csv2,
        recovery_rates_at_start,
        recovery_rates_at_end,
    )?;

    print!("{recovery}");
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
