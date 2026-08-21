//! Reads a Toronto Hydro bill PDF and debug-prints the parsed
//! [`HydroBill`](ev_cost_recovery::hydro_bill::HydroBill).

use ev_cost_recovery::hydro_bill::pdf_text;
use ev_cost_recovery::hydro_bill::{BillError, hydro_bill_from_pdf};
use std::io;
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
        let pages = pdf_text::read_pages(input)?;
        return match pdf_text::write_pages(&pages, &mut io::stdout().lock()) {
            // Whatever stdout is connected to can stop reading before the output ends, and the
            // write then fails. That is its choice to make, not a fault, so this still succeeds.
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            result => Ok(result?),
        };
    }
    let bill = hydro_bill_from_pdf(input).map_err(with_advice)?;
    println!("{}", bill.print());
    Ok(())
}

/// A parse failure, with what to do about it where there is something to do.
///
/// [`BillError::is_layout`] is the one distinction that changes the advice. A layout failure means
/// the PDF gave up its text and the text is not what this expects, so there is something to go and
/// look at. Anything else means there is no text to look at, and pointing at `--lines` would only
/// send the reader somewhere that fails the same way.
fn with_advice(e: BillError) -> String {
    if !e.is_layout() {
        return e.to_string();
    }
    format!(
        "{e}\n\
         This is not a bill layout this recognises. To see what the layout actually is:\n\
         \x20   hydro_bill_dump --lines {}",
        e.path().display()
    )
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_layout_failure_is_told_where_to_look() {
        let advice = with_advice(BillError::Missing {
            path: PathBuf::from("data/hydro_bills/TH_2025_07_28.pdf"),
            what: "line labelled \"Your Electricity Charges\"".to_owned(),
        });
        assert!(
            advice
                .starts_with("data/hydro_bills/TH_2025_07_28.pdf: the bill has no line labelled ")
        );
        assert!(advice.ends_with("hydro_bill_dump --lines data/hydro_bills/TH_2025_07_28.pdf"));
    }

    #[test]
    fn a_file_that_never_read_is_not_sent_to_lines() {
        let advice = with_advice(BillError::Unreadable {
            path: PathBuf::from("README.md"),
            source: "README.md: couldn't parse input: invalid file header".into(),
        });
        assert_eq!(
            advice,
            "README.md: couldn't parse input: invalid file header"
        );
    }
}
