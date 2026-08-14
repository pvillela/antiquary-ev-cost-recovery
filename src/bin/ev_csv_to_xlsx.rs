use ev_cost_recovery::sessions::session_csv_to_xlsx;
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
Converts a charging session report from CSV to .xlsx.

Usage: ev_csv_to_xlsx <SESSION_REPORT.csv>...

Each workbook is written beside its input with the extension replaced. Rows needing a judgement
call — an ambiguous DST fold, a wall time in the DST gap, a session with no charge time, one whose
reported duration runs past its reported end — are reported on stderr and recorded in the
workbook's Anomalies column; they do not stop the conversion. Row numbers are workbook rows, so a
record duplicated to resolve a DST fold is reported twice, once per copy. See README.md.";

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return if args.is_empty() {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    let mut failed = false;
    for path in &args {
        match session_csv_to_xlsx(path) {
            Ok(report) => {
                println!("{}", report.output_path.display());
                for anomaly in &report.anomalies {
                    eprintln!("{}: {anomaly}", path.display());
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                failed = true;
            }
        }
    }

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
