use ev_cost_recovery::sessions::{Session, session_list};
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
Lists the charging sessions in a converted session report workbook.

Usage: sessions <SESSION_REPORT.xlsx>...

One line per session: id, connection start (UTC), energy use in kWh, average power in kW.

A session with zero Active_Charge_Time has no finite average power and is listed separately, under
SPIKES; those are worth reviewing for their effect on the building's demand charge. A session whose
reported start, end and duration contradict each other is listed under EXCLUDED and takes no part
in any estimate. See README.md.";

/// Printed above the spike listing, because the figure in that listing is the one thing about a
/// spike that cannot be read at face value.
///
/// A spike delivered its energy in no time at all, so it has no average power to report. The
/// workbook says as much — its `avg_kw` cell shows `#DIV/0!` — but this listing has to print a
/// number in that column, and the number it prints is the substitute the estimating logic uses, not
/// a measurement. Saying so here is cheaper than leaving a reader to wonder why a session with zero
/// charge time has a perfectly ordinary-looking kW figure.
const SPIKE_RECAP: &str = "  These sessions report zero Active_Charge_Time, so their energy
  arrived in no time at all and they have no measurable average power.
  The workbook shows #DIV/0! in the avg_kw cell.

  The kW figure below is a SUBSTITUTE, not a measurement: the breaker
  rating where energy was delivered, and zero where none was. Without it
  an infinite average power would swamp any segment the session touched.
  The energy and the times are as reported.

  Worth reviewing one at a time. A real delivery of energy with a broken
  duration still moved the building's demand, and the substitute is a
  floor on what it contributed, not an estimate of it. See README.md,
  \"Other\".
";

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
                    println!("{SPIKE_RECAP}");
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

/// A spike has no finite energy-over-time figure; [`Session::avg_kw`] substitutes the one the
/// estimating logic uses, which is the figure worth listing beside the energy it came from.
fn line(session: &Session) -> String {
    format!(
        "{:<14} {}  {:>8.3}  {:>9.3} kW",
        session.id,
        session.conn_start,
        session.energy_use,
        session.avg_kw()
    )
}
