//! Turns a Toronto Hydro Green Button export into the peak-values workbook.

use ev_cost_recovery::{
    green_button::{Feed, parse, write_workbook},
    hydro_bills::{BILL_END_DAY, bill_start_day},
    time::holidays,
    time::local_date,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The help text. A function rather than a `const` because it states the billing period boundary,
/// which is [`BILL_END_DAY`] rather than anything this file should be repeating.
fn usage() -> String {
    format!(
        "\
gb_peak_values -- billing-period peak values from a Green Button export.

Reads a Toronto Hydro Green Button (ESPI) XML export and writes an Excel workbook beside it with
two sheets. Peak_values carries one row per billing period: the energy used, the highest demand in
kW and kVA over the whole period and within the 7-7 demand window, when each of those occurred, and
the Time-of-Use period it fell in. Interval_values carries every hour of the export.

A billing period runs from the start of day {start_day} of one month to the end of day {BILL_END_DAY} of the next, in
Toronto local time, and is labelled by that closing date.

The output file is named after the input, with an .xlsx extension, and is written to the same
directory. An existing file is never overwritten: move or delete it first. That is deliberate --
figures in these workbooks get reconciled against real invoices, and a silent overwrite is how that
work gets lost.

An interval count that is not what a complete billing period should hold is highlighted in light
red, as is any cell reporting an anomaly.

Usage:
    gb_peak_values <XML>
    gb_peak_values --help

Example:
    gb_peak_values data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML
    -> data/TH_Electric_Usage_23-11-2024_to_24-06-2026.xlsx

The feed must carry hourly readings for all three of kWh, kW and kVA. Anything else is an error
naming what was missing, rather than a workbook with a hole in it.
",
        start_day = bill_start_day(BILL_END_DAY),
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    let [input] = args.as_slice() else {
        eprint!("{}", usage());
        return ExitCode::FAILURE;
    };

    match run(Path::new(input)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let output = output_path(input)?;
    if output.exists() {
        return Err(format!(
            "{} already exists. Move or delete it first -- this tool never overwrites its output.",
            output.display()
        )
        .into());
    }

    let xml = std::fs::read_to_string(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let feed = parse(&xml)?;

    report_holidays(&feed);

    let report = write_workbook(&output, &feed, BILL_END_DAY)?;
    println!("{}", output.display());
    eprintln!(
        "{} billing periods, {} intervals",
        report.period_rows, report.interval_rows
    );
    if report.incomplete_periods > 0 {
        eprintln!(
            "{} period(s) do not hold a full billing period's intervals; highlighted in the sheet",
            report.incomplete_periods
        );
    }
    for (kind, count) in &report.anomaly_counts {
        eprintln!("anomaly: {kind} x{count}");
    }
    Ok(())
}

/// The input's path with an `.xlsx` extension.
///
/// # Errors
///
/// Returns an error if that would name the input itself, which would mean reading and writing the
/// same file.
fn output_path(input: &Path) -> Result<PathBuf, String> {
    let output = input.with_extension("xlsx");
    if output == input {
        return Err(format!("{} is already an .xlsx file", input.display()));
    }
    Ok(output)
}

/// Prints the holiday calendar actually applied.
///
/// Worth the noise: which days count as holidays decides which hours are off-peak, and therefore
/// the demand figures the bill is built from. A wrong calendar is otherwise invisible -- it just
/// produces a slightly different number.
fn report_holidays(feed: &Feed) {
    let Some((first, _)) = feed.kwh.values.first_key_value() else {
        return;
    };
    let Some((last, _)) = feed.kwh.values.last_key_value() else {
        return;
    };
    let (from, to) = (local_date(*first), local_date(*last));

    eprintln!("Ontario TOU holidays applied ({from} to {to}):");
    for year in from.year()..=to.year() {
        for holiday in holidays::holidays(year) {
            if holiday.date < from || holiday.date > to {
                continue;
            }
            let note = match holiday.substitute_for {
                Some(base) => format!(" (observed, for {base})"),
                None => String::new(),
            };
            eprintln!("  {} {}{}", holiday.date, holiday.name, note);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_output_sits_beside_the_input_with_an_xlsx_extension() {
        assert_eq!(
            output_path(Path::new(
                "data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML"
            ))
            .unwrap(),
            PathBuf::from("data/TH_Electric_Usage_23-11-2024_to_24-06-2026.xlsx")
        );
    }

    #[test]
    fn an_xlsx_input_is_refused_rather_than_read_and_written_at_once() {
        assert!(output_path(Path::new("already.xlsx")).is_err());
    }
}
