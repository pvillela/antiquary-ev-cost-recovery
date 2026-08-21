//! Toronto Hydro bills: the charges themselves, as distinct from the metered consumption that
//! `green_button` reads and the charging sessions that `sessions` reads.
//!
//! The two older modules answer *how much* — kilowatt-hours, kilowatts, kilovolt-amperes, and
//! which quarter-hour the site peaked in. Neither answers *what it cost*, and the project exists
//! to work out how much of a bill EV charging is responsible for. That last step needs the bill:
//! the rate schedule, the delivery and regulatory lines, the loss factor, and the way a demand
//! charge is levied on a monthly peak rather than on consumption. [`HydroBill`] is that bill, and
//! [`hydro_bill_from_pdf`] reads one straight out of the PDF Toronto Hydro issues.
//!
//! Behind those two names the source is a stack, each file knowing less than the one above it:
//! `hydro_bill.rs` is the figures alone, `bill_pdf.rs` is everything that knows what a Toronto
//! Hydro bill looks like, and [`pdf_text`] is positioned text out of any PDF at all. Only the
//! last is a module of its own here, because reading a PDF is a job in its own right and its
//! `Line` and `Fragment` read better with it named.
//!
//! Plural because a run reconciles a series of them — a billing period at a time. What a billing
//! period is, `billing_period.rs` defines: [`BILL_END_DAY`], [`BillingPeriod`] and
//! [`billing_period_dates`]. It is here because the period is the bill's, and the rest of the crate
//! divides its data that way only because the bill does.

mod bill_pdf;
pub use bill_pdf::*;

mod billing_period;
pub use billing_period::*;

mod hydro_bill;
pub use hydro_bill::*;

pub mod pdf_text;
