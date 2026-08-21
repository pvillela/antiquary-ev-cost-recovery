//! What a front-end asks the library, stated in the terms a front-end has.
//!
//! The rest of the crate is organised by the source of data it reads. A caller checking an invoice
//! has none of those in hand — it has a billing period, a meter export and the charging network's
//! monthly reports — so this module is where those become the calls the other modules understand.
//!
//! # How it is arranged
//!
//! [`pure`] computes and [`io`] reads. Every `io` function turns paths into values and delegates,
//! so that the reasoning a figure rests on can be exercised without a filesystem, and so that one
//! file answers what the library touches on disk.
//!
//! Types live with the function that produces them, errors included:
//! [`PowerEstimates`](pure::peak_power::PowerEstimates) and
//! [`PeakPowerError`](pure::peak_power::PeakPowerError) with `peak_power`,
//! [`SessionReportCoverage`](pure::session_reports::SessionReportCoverage) and
//! [`CoverageError`](pure::session_reports::CoverageError) with `session_reports`,
//! [`ReadError`](io::ReadError) with `io`.
//!
//! [`ApiError`] is the exception, and has a module of its own: it is the union every call collapses
//! into for a front-end that would rather render one type, so it depends on both halves while
//! neither depends on it.
//!
//! # Importing from here
//!
//! The unit of import is the operation, not the type: `use ev_cost_recovery::io::peak_power;` gives
//! both the function and the module, so the call and the types its signature names come from one
//! import. Each module re-exports what its own signatures mention and does not define, so calling a
//! function never requires knowing which module it delegates to.
//!
//! That stops at the first level. Probing *into* a returned type may well lead elsewhere in the
//! crate — the two halves of [`PowerEstimates`](pure::peak_power::PowerEstimates) are
//! [`sessions::IntervalEstimates`](crate::sessions::IntervalEstimates), and reading about them
//! means going there. Re-exporting transitively would put the whole crate in every module.
//!
//! There is deliberately no roster of every type here, and no module of shared ones. Either would
//! gather a result and an error per operation into one flat namespace — the arrangement being
//! avoided, spelled `pub use` instead of `pub struct`.

mod error;
pub use error::ApiError;

pub mod io;
pub mod pure;
