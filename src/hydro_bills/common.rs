//! Facts about Toronto Hydro's billing that the rest of the crate needs, held where the bills are
//! rather than where they happen to be used.

/// The day of the month a Toronto Hydro billing period ends on.
///
/// The bill states its own period as `MAY 23 2026 TO JUN 23 2026`, so this is read off every
/// invoice rather than chosen. It is here because it is a fact about the bill, and
/// `green_button` needs it only because the meter data has to be cut the same way the bill cuts
/// it -- a period there runs from the start of the day after this one to the end of this one, in
/// **standard time**, and is labelled by the date it ends on. Standard time all year, not the
/// prevailing local clock: see `green_button::BillingPeriod` for why, and
/// `docs/hydro_bills/archive/dst-energy-anomaly-pre-fix.md` for the evidence.
///
/// Changing it moves every billing period boundary in the crate. Nothing suggests Toronto Hydro
/// will, but the number was in three places before it was here, and three places is how a change
/// like that goes half-applied.
pub const BILL_END_DAY: i8 = 23;

/// The day of the month a billing period starts on, given the day one ends on: the day after.
///
/// A function rather than a second constant, so this file states one fact and one relationship
/// rather than two facts that could drift apart. `const` so it still serves where a constant is
/// wanted. Call it as `bill_start_day(BILL_END_DAY)`.
pub const fn bill_start_day(end_day: i8) -> i8 {
    end_day + 1
}
