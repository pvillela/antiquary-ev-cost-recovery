//! Toronto Hydro bills: the charges themselves, as distinct from the metered consumption that
//! `green_button` reads and the charging sessions that `sessions` reads.
//!
//! Empty so far. The two existing modules answer *how much* — kilowatt-hours, kilowatts, kilovolt-
//! amperes, and which quarter-hour the site peaked in. Neither answers *what it cost*, and the
//! project exists to work out how much of a bill EV charging is responsible for. That last step
//! needs the bill: the rate schedule, the delivery and regulatory lines, the loss factor, and the
//! way a demand charge is levied on a monthly peak rather than on consumption.
//!
//! Plural because a run reconciles a series of them — a billing period at a time, the way
//! `green_button::BillingPeriod` already divides the meter data.

pub mod hydro_bill;
