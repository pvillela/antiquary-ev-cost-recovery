use ev_peak_contrib::{Session, session_list};
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
Lists the charging sessions in a converted session report workbook.

Usage: sessions <SESSION_REPORT.xlsx>...

One line per session: id, connection start (UTC), energy use in kWh, average power in kW.

A session with zero Active_Charge_Time has no finite average power and is listed separately, under
SPIKES; those are worth reviewing for their effect on the building's demand charge. A session whose
reported start, end and duration contradict each other is listed under EXCLUDED and takes no part
in any estimate. See README.md.";

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
        match session_list(path) {
            Ok(report) => {
                println!("{}", path.display());
                for session in &report.sessions {
                    println!("{}", line(session));
                }
                if !report.spikes.is_empty() {
                    println!("\nSPIKES (zero Active_Charge_Time):");
                    for spike in &report.spikes {
                        println!("{}", line(spike));
                    }
                }
                if !report.excluded.is_empty() {
                    println!("\nEXCLUDED (inconsistent start, end and duration):");
                    for session in &report.excluded {
                        println!("{}", line(session));
                    }
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

/// A spike's `avg_power` is infinite, which `{:>9.3}` renders as `inf`.
fn line(session: &Session) -> String {
    format!(
        "{:<14} {}  {:>8.3}  {:>9.3} kW",
        session.id, session.conn_start, session.energy_use, session.avg_kw
    )
}
