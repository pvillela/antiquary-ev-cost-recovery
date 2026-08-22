//! The half of the API that computes.
//!
//! Everything here is a function of its arguments alone: no file is opened, no clock is read,
//! nothing is written. [`io`](super::io) is the other half, and it is deliberately thin — it turns
//! paths into values and hands them here, so that the reasoning a figure rests on can be exercised
//! without a filesystem in the way.
//!
//! Taking a `&Path` is not I/O.
//! [`session_report`](session_report::report_coverage) reads a *name*, which is a string that
//! happens to be spelled as a path; it never asks whether the file exists.
//!
//! The submodules are by subject, not by call. The API layer's other axis — reading versus
//! computing — is already spent on the `io`/`pure` division, and spending it twice would leave
//! every subject scattered. `peak_power` is one API operation today; `session_report` is what it
//! and every later operation are built from.
//!
//! What a billing period *is* is not here. It is a fact about the bill, so it lives in
//! [`hydro_bill::billing_period`](crate::hydro_bill) with [`BILL_END_DAY`](crate::hydro_bill::BILL_END_DAY)
//! and [`BillingPeriod`](crate::hydro_bill::BillingPeriod), and this module reads it from there.

pub mod energy;
pub mod peak_power;
pub mod session_report;

pub use energy::{energy, energy_cost};
pub use peak_power::{peak_power, peak_power_cost};
pub use session_report::{check_reports_cover_period, report_coverage, reports_cover};

use crate::hydro_bill::billing_period_dates;
use crate::markdown::field;
use jiff::Unit;
use jiff::civil::Date;

/// The `Period` line every report in this module is headed by.
///
/// The span rather than the closing date alone. A period is named by the day it closes on, which is
/// what the argument carries, but a reader checking a figure against a bill needs the dates the
/// bill states — and those run from the 24th of the month before.
///
/// Both ends count, so 24 May to 23 June is 31 days. That is the count the bill prorates its demand
/// charges by, so a report that said 30 here would contradict the `Adj.` columns it is checked
/// against.
pub(crate) fn period_line(billing_period_ending: Date) -> String {
    let span = billing_period_dates(billing_period_ending)
        .ok()
        .and_then(|(start, end)| Some((start, end, start.until((Unit::Day, end)).ok()?)));

    let value = match span {
        Some((start, end, days)) => format!("{start} - {end}  ({} days)", days.get_days() + 1),
        // Not reachable through any function here: each validates the closing date before building
        // the value this heads. Degraded rather than panicked on all the same, because a `Display`
        // that can bring a process down is worse than one that says less.
        None => format!("ending {billing_period_ending}"),
    };
    field("Period", &value)
}
