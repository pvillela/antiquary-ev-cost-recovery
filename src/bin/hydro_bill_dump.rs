//! Reads a Toronto Hydro bill PDF and debug-prints the parsed [`HydroBill`].

use ev_cost_recovery::hydro_bills::hydro_bill::HydroBill;
use ev_cost_recovery::hydro_bills::pdf_text;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
hydro_bill_dump -- what a Toronto Hydro bill PDF parses to.

Reads a Toronto Hydro bill PDF and writes the resulting HydroBill to stdout in Rust's debug form,
one field per line. Nothing is written to disk.

A figure that appears more than once on the bill is added up before it is printed. That happens
when a billing period straddles the Summer/Winter boundary, which gives it two Time-of-Use blocks,
and when it straddles a rate change, which prints every delivery and regulatory line twice.

A bill whose layout this does not recognise is an error naming the line it stopped on, rather than
a structure with a zero where a charge should be. --lines is how to see what the layout actually
is: it prints the text of the PDF as positioned lines, page by page, and parses nothing.

Usage:
    hydro_bill_dump <PDF>
    hydro_bill_dump --lines <PDF>
    hydro_bill_dump --help

Example:
    hydro_bill_dump data/hydro_bills/TH_5728140000_2025_07_28.pdf
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let (lines_only, input) = match args.as_slice() {
        [input] => (false, input),
        [flag, input] if flag == "--lines" => (true, input),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match run(Path::new(input), lines_only) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &Path, lines_only: bool) -> Result<(), Box<dyn std::error::Error>> {
    if lines_only {
        for (number, page) in pdf_text::read_pages(input)?.iter().enumerate() {
            println!("===== page {} =====", number + 1);
            for line in page {
                let runs: Vec<String> = line
                    .fragments
                    .iter()
                    .map(|f| format!("{:.0}:{}", f.x, f.text.trim()))
                    .collect();
                println!("[y={:7.1}] {}", line.y, runs.join("  |  "));
            }
        }
        return Ok(());
    }
    println!("{:#?}", HydroBill::from_pdf(input)?);
    Ok(())
}
