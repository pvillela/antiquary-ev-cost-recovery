//! Facts about Toronto Hydro's billing that the rest of the crate needs, held where the bills are
//! rather than where they happen to be used.

/// The day of the month a Toronto Hydro billing period ends on.
///
/// The bill states its own period as `MAY 23 2026 TO JUN 23 2026`, so this is read off every
/// invoice rather than chosen. It is here because it is a fact about the bill, and
/// `green_button` needs it only because the meter data has to be cut the same way the bill cuts
/// it -- a period there runs from the start of the day after this one to the end of this one, in
/// local time, and is labelled by the date it ends on.
///
/// Changing it moves every billing period boundary in the crate. Nothing suggests Toronto Hydro
/// will, but the number was in three places before it was here, and three places is how a change
/// like that goes half-applied.
pub const BILL_END_DAY: i8 = 23;

/// The day of the month a Toronto Hydro billing period starts on: the day after the one the
/// period before it ended on.
pub const BILL_START_DAY: i8 = BILL_END_DAY + 1;
