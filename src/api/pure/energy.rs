//! The EV share of a billing period's energy, split by time-of-use period.
//!
//! The demand side of the bill is [`peak_power`](mod@super::peak_power): what the chargers contributed
//! to the hour the site peaked in. This is the other side, consumption, which is billed by the
//! kilowatt-hour at a rate that depends on when it was drawn.

use crate::{
    hydro_bill::{BILL_END_DAY, BillingPeriod, NotABillingPeriodEnding, billing_period_dates},
    session::{RSession, SessionReport, TouKwh, tou_kwh},
    time::Interval,
};
use jiff::civil::Date;
use std::error::Error;
use std::fmt;

/// Why a billing period's sessions cannot be turned into an energy attribution.
///
/// An enum with one variant rather than the bare struct, because the operation is expected to grow
/// failures of its own -- the loss factor and the rate schedule are both on the bill and neither is
/// read yet. Embedding [`NotABillingPeriodEnding`] rather than restating it is what
/// [`CoverageError`](super::session_report::CoverageError) and
/// [`PeakPowerError`](super::peak_power::PeakPowerError) do with the same failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnergyError {
    /// The date given does not close a billing period: it is not [`BILL_END_DAY`] of its month.
    NotABillingPeriodEnding(NotABillingPeriodEnding),
}

impl From<NotABillingPeriodEnding> for EnergyError {
    fn from(e: NotABillingPeriodEnding) -> Self {
        Self::NotABillingPeriodEnding(e)
    }
}

impl fmt::Display for EnergyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABillingPeriodEnding(e) => e.fmt(f),
        }
    }
}

impl Error for EnergyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotABillingPeriodEnding(e) => Some(e),
        }
    }
}

/// Energy consumption by time-of-use period attributable to EV charging sessions, over a billing
/// period.
///
/// Each session's energy is spread evenly over the time it was connected, then cut at the period's
/// boundaries and at every price-period boundary. A session straddling either contributes only the
/// part falling inside, so the three figures returned sum to the energy drawn within the period and
/// not to the energy of the sessions given.
///
/// # Arguments
///
/// - `billing_period_ending` - the billing period, named by the date it closes on. Must be
///   [`BILL_END_DAY`] of its month.
/// - `sessions` - the sessions to attribute, in any order.
///
/// `sessions` need not be exactly the period's. A session outside it contributes nothing, so
/// passing more than the period needs is harmless; a session stated identically more than once is
/// counted once, so overlapping sources may be concatenated. What the caller must supply is *every*
/// session touching the period, since a missing one is indistinguishable from one that drew nothing.
///
/// # Records that are not ordinary
///
/// A record whose reported start, end and duration contradict each other is left out entirely. Its
/// energy cannot be placed on a timeline, which is the only thing this function does with it.
///
/// A record reporting real energy against zero `Active_Charge_Time` is included. That makes its
/// *power* meaningless, and power is not what is summed here; its energy is spread over the time it
/// was connected, which is sound.
///
/// # Errors
///
/// [`EnergyError::NotABillingPeriodEnding`] if `billing_period_ending` is not [`BILL_END_DAY`] of its
/// month.
pub fn energy(billing_period_ending: Date, sessions: &[RSession]) -> Result<TouKwh, EnergyError> {
    billing_period_dates(billing_period_ending)?;

    let period = BillingPeriod::ending_on(billing_period_ending, BILL_END_DAY);
    let time_range = Interval::from_start_end(period.start, period.end);

    // Not summed as given: `sessions` may state the same session more than once, and this is what
    // counts it once.
    let report = SessionReport::from_session_lists(vec![sessions.to_vec()], Vec::new());
    // Two of the three report buckets. `excluded` is left out rather than overlooked: those
    // records' start, end and duration contradict each other, and an inverted one panics in
    // `adj_duration` before any figure comes of it.
    let counted: Vec<RSession> = report
        .sessions
        .iter()
        .chain(&report.spikes)
        .cloned()
        .collect();

    Ok(tou_kwh(time_range, &counted))
}

// cargo test --lib -- api::pure::energy::test
#[cfg(test)]
mod test {
    use super::*;
    use crate::session::test_support::{inverted_session, session, spike_session};
    use jiff::civil::{Date, date};

    /// The period every fixture here belongs to: 24 May to 23 June 2026.
    fn period_ending_date() -> Date {
        date(2026, 6, 23)
    }

    fn kwh(sessions: &[RSession]) -> f64 {
        energy(period_ending_date(), sessions)
            .expect("23 June closes a billing period")
            .total_kwh()
    }

    /// A session inside the period contributes all its energy, and one outside contributes none.
    #[test]
    fn only_energy_drawn_inside_the_period_counts() {
        // 02:00 EDT on 10 June, an hour long, wholly inside the period and inside one TOU block.
        let inside = session("June.csv", 2, "IN", "2026-06-10T06:00:00Z", 60, 7.0);
        // Mid-April, two months before the period opens.
        let outside = session("April.csv", 2, "OUT", "2026-04-15T06:00:00Z", 60, 7.0);

        assert!((kwh(std::slice::from_ref(&inside)) - 7.0).abs() < 1e-9);
        assert_eq!(kwh(std::slice::from_ref(&outside)), 0.0);
        assert!((kwh(&[inside, outside]) - 7.0).abs() < 1e-9);
    }

    /// The overlap case. Monthly reports overlap at the month boundary, so a session near it is in
    /// both files; counting it twice would inflate the energy by exactly its own kilowatt-hours.
    #[test]
    fn a_session_both_reports_state_identically_is_counted_once() {
        let once = session("June.csv", 2, "BOUNDARY", "2026-06-01T06:00:00Z", 60, 7.0);
        // The same session as May's report states it: its own row in its own file, every figure
        // the same.
        let again = session("May.csv", 9, "BOUNDARY", "2026-06-01T06:00:00Z", 60, 7.0);

        assert!((kwh(std::slice::from_ref(&once)) - 7.0).abs() < 1e-9);
        assert!((kwh(&[once, again]) - 7.0).abs() < 1e-9);
    }

    /// A record whose end precedes its start is flagged `InconsistentDuration` and takes no part.
    ///
    /// Not merely a wrong figure if it did: `Session::adj_duration` panics on an inverted span, so
    /// summing the list as given would bring the whole call down.
    #[test]
    fn a_record_that_contradicts_itself_is_left_out_rather_than_summed() {
        let sound = session("June.csv", 2, "SOUND", "2026-06-10T06:00:00Z", 60, 7.0);
        // Its reported end is an hour before its reported start.
        let inverted = inverted_session("June.csv", 3, "INVERTED", "2026-06-10T06:00:00Z", 60, 5.0);

        let total = kwh(&[sound, inverted]);
        assert!(
            (total - 7.0).abs() < 1e-9,
            "the inverted record should contribute nothing, got {total}"
        );
    }

    /// A spike -- zero `Active_Charge_Time` beside real energy -- counts. Its *power* is
    /// meaningless, and power is not what is summed here.
    #[test]
    fn a_spike_contributes_its_energy() {
        // The peak estimates hold spikes apart because `energy / charge_time` is infinite. The
        // energy split divides by connection time instead, which is a whole hour here.
        let spike = spike_session("June.csv", 2, "SPIKE", "2026-06-10T06:00:00Z", 60, 7.0);
        assert!((kwh(&[spike]) - 7.0).abs() < 1e-9);
    }

    /// A date that is not a closing date is the caller's mistake, and is reported as such rather
    /// than reaching the panic in `BillingPeriod::ending_on`.
    #[test]
    fn a_date_that_does_not_close_a_billing_period_is_refused() {
        let err =
            energy(date(2026, 6, 30), &[]).expect_err("30 June does not label a billing period");
        assert!(
            matches!(err, EnergyError::NotABillingPeriodEnding(_)),
            "{err}"
        );
        assert!(err.to_string().contains("2026-06-30"), "{err}");
    }
}
