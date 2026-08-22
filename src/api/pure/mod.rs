//! The half of the API that computes.
//!
//! Everything here is a function of its arguments alone: no file is opened, no clock is read,
//! nothing is written. [`io`](super::io) is the other half, and it is deliberately thin — it turns
//! paths into values and hands them here, so that the reasoning a figure rests on can be exercised
//! without a filesystem in the way.
//!
//! Taking a `&Path` is not I/O.
//! [`coverage`](coverage::check_reports_cover_period) reads a *name*, which is a string that
//! happens to be spelled as a path; it never asks whether the file exists.
//!
//! The submodules are by subject, not by call. The API layer's other axis — reading versus
//! computing — is already spent on the `io`/`pure` division, and spending it twice would leave
//! every subject scattered. `peak_power` is one API operation today; `coverage` is what it and
//! every later operation are built from.
//!
//! What a billing period *is* is not here. It is a fact about the bill, so it lives in
//! [`hydro_bill::billing_period`](crate::hydro_bill) with [`BILL_END_DAY`](crate::hydro_bill::BILL_END_DAY)
//! and [`BillingPeriod`](crate::hydro_bill::BillingPeriod), and this module reads it from there.

pub mod coverage;
pub mod energy;
pub mod peak_power;

pub use coverage::check_reports_cover_period;
pub use energy::{energy, energy_cost};
pub use peak_power::{peak_power, peak_power_cost};
