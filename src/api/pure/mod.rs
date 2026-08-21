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
//! every subject scattered. `peak_power` is one API operation today; `billing_period` and
//! `session_report` are what it and every later operation are built from.

pub mod billing_period;
pub mod peak_power;
pub mod session_report;

pub use billing_period::billing_period_dates;
pub use peak_power::{peak_power, peak_power_cost};
pub use session_report::{check_reports_cover_period, report_coverage, reports_cover};
