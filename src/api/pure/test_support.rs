//! Fixtures shared by the unit tests of more than one module in this directory.
//!
//! One billing period, one bill for it, one meter export and one set of sessions, so that a figure
//! computed by two operations can be compared between them. A second definition of any of these
//! would be a second place for the two to drift apart, which is exactly what
//! [`cost_recovery_surplus`](super::recovery::cost_recovery_surplus) exists to subtract.
//!
//! What stays with a module's own tests is anything that reads a result rather than builds an
//! input. Those are about one operation and belong beside it.

use crate::green_button::{Peak, PeriodValues};
use crate::hydro_bill::{BILL_END_DAY, BillingPeriod, HydroBill};
use crate::session::{RSession, test_support::session};
use crate::time::Tou;
use jiff::Timestamp;
use jiff::civil::{Date, date};
use std::collections::BTreeMap;

/// The period every fixture here belongs to: 24 May to 23 June 2026.
const PERIOD_ENDING: (i16, i8, i8) = (2026, 6, 23);

/// The hour the building peaked in kW. 16:00 EDT on 10 June.
pub(crate) const KW_PEAK_HOUR: &str = "2026-06-10T20:00:00Z";
/// The hour it peaked in kVA — a different hour, as it usually is. 19:00 EDT on 11 June.
pub(crate) const KVA_PEAK_HOUR: &str = "2026-06-11T23:00:00Z";
/// The hour it peaked in kW within the 07:00-19:00 demand window. 08:00 EDT on 1 June, a third
/// hour again, so a figure read off the wrong one is visible.
pub(crate) const NOP_PEAK_HOUR: &str = "2026-06-01T12:00:00Z";

pub(crate) fn ts(s: &str) -> Timestamp {
    s.parse().expect("an RFC 3339 timestamp")
}

pub(crate) fn period_ending_date() -> Date {
    date(PERIOD_ENDING.0, PERIOD_ENDING.1, PERIOD_ENDING.2)
}

/// A period whose maxima fall in the hours named, with the figures the estimate does not read left
/// at zero.
///
/// `peak_power` uses a [`Peak`] for one thing only — when it happened — so the magnitudes are
/// deliberately not stated. A fixture carrying invented kW values would suggest the estimate
/// compares itself against them, and it does not: the building's own demand is the bill's, and what
/// is estimated is the EV share of the same hour.
///
/// `nop_at` is the hour the 7-7 maximum fell in, which only the cost reads.
pub(crate) fn period_values_with_nop(
    kw_at: Option<&str>,
    kva_at: Option<&str>,
    nop_at: Option<&str>,
) -> PeriodValues {
    let peak = |at: &str, tou| Peak {
        value: 0,
        at: ts(at),
        companion: None,
        tou,
    };
    PeriodValues {
        period: BillingPeriod::ending_on(period_ending_date(), BILL_END_DAY),
        interval_count: 744,
        kwh_total: 0,
        max_kw: kw_at.map(|at| peak(at, Tou::OnPeak)),
        max_kw_nop: nop_at.map(|at| peak(at, Tou::MidPeak)),
        max_kva: kva_at.map(|at| peak(at, Tou::MidPeak)),
        max_kva_nop: None,
        anomaly_counts: BTreeMap::new(),
    }
}

/// A bill for the fixture period, with figures chosen so that every rate a cost derives comes out
/// whole and can be checked by eye.
///
/// 31 days, so the `Adj.` proration is 31/30 and not the identity: `120 * 31/30 = 124`,
/// `150 * 31/30 = 155`, `90 * 31/30 = 93`. The three delivery lines then divide into blended rates
/// of 10, 3 and 5, and HST and the rebate into 13% and 10% of the total charges.
///
/// The lines a cost does not read carry figures too, so a test cannot pass by reading one of them:
/// nothing here is zero.
pub(crate) fn bill() -> HydroBill {
    HydroBill {
        statement_date: date(2026, 6, 28),
        on_peak_kwh: 13000.0,
        mid_peak_kwh: 12000.0,
        off_peak_kwh: 45000.0,
        on_peak_cost: 2000.0,
        mid_peak_cost: 1500.0,
        off_peak_cost: 3400.0,
        delivery_customer_charges: 62.0,
        distribution_charges: 1550.0,
        transmission_connection_charge: 372.0,
        transmission_network_charge: 465.0,
        standard_supply_admin_charge: 0.25,
        wholesale_market_svc_charge: 420.0,
        total_electricity_charges: 10000.0,
        hst: 1300.0,
        ontario_electricity_rebate: 1000.0,
        meter_reading_period_from: date(2026, 5, 23),
        meter_reading_period_to: period_ending_date(),
        number_of_days: 31,
        kwh_used: 70000.0,
        loss_factor_adjustment: 1.0295,
        adjusted_kwh_used: 72065.0,
        peak_7_7_kw: 90.0,
        adj_peak_7_7_kw: 93.0,
        demand_kw: 120.0,
        demand_kva: 150.0,
        metering_adj: 1.0,
        adj_kw: 124.0,
        adj_kva: 155.0,
    }
}

/// Sessions as the two monthly reports covering this period would yield them.
///
/// Laid out against the kW peak hour, whose four segments start at 20:00, 20:15, 20:30 and 20:45.
/// `WHOLE` runs the length of the hour; `MID_A` and `MID_B` are 14 minutes from 20:15, which with
/// the one-minute padding on the adjusted end is exactly the 20:15 segment and no other. That
/// segment therefore holds three sessions and every other holds one, which is what makes the
/// maximum predictable without depending on any electrical constant.
///
/// `EVENING` sits in the kVA peak hour instead, and `ELSEWHERE` in neither, so the two estimates
/// cannot accidentally agree.
pub(crate) fn two_reports() -> Vec<RSession> {
    vec![
        session("May.csv", 2, "MAY_ONLY", "2026-05-26T18:00:00Z", 30, 2.0),
        session("June.csv", 2, "WHOLE", KW_PEAK_HOUR, 60, 6.0),
        session("June.csv", 3, "MID_A", "2026-06-10T20:15:00Z", 14, 1.0),
        session("June.csv", 4, "MID_B", "2026-06-10T20:15:00Z", 14, 1.0),
        session("June.csv", 5, "EVENING", "2026-06-11T23:45:00Z", 14, 1.0),
        session("June.csv", 6, "ELSEWHERE", "2026-06-01T12:00:00Z", 30, 2.0),
    ]
}

/// Money, to the cent.
pub(crate) fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 0.005
}
