//! The EV share of the two hours a billing period peaked in.

use crate::api::pure::billing_period::{NotABillingPeriodEnding, billing_period_dates};
use crate::green_button::{METER_INTERVAL, Peak};
use crate::sessions::{
    Bracket, EstimateSet, IntervalEstimates, SessionReport, estimates_from_report,
};
use crate::time::Interval;

// Re-exported because `peak_power` and `peak_power_cost` take them. `IntervalEstimates` is
// deliberately not: it is inside `PowerEstimates` rather than named by the signature, and a reader
// who probes that far can go to `sessions` for it.
pub use crate::green_button::PeriodValues;
pub use crate::hydro_bills::HydroBill;
pub use crate::sessions::RSession;
use jiff::civil::Date;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// Peak power estimates for a billing period.
pub struct PowerEstimates {
    pub kw_estimates: IntervalEstimates,
    pub kva_estimates: IntervalEstimates,
}

/// Breakdown of delivery cost attributable to EV sessions in a billing period.
///
/// Every field is stated rather than left to be recomputed, because the point of the breakdown is
/// to be checked against the bill line by line.
pub struct DeliveryCost {
    /// `'Distribution Charges' / 'Adj. kVA'` from bill.
    pub blended_distribution_rate: f64,
    /// `'Transmission Connection Charge' / 'Adj. kW'` from bill.
    pub blended_transmission_connection_rate: f64,
    /// `'Transmission Network Charge' / 'Adj. Peak kW 7-7'` from bill.
    pub blended_transmission_network_rate: f64,

    /// Mid-point of energy-based bracket of EV kVA from sessions
    /// for Demand kVA interval of interest.
    pub demand_kva: f64,
    /// Mid-point of energy-based bracket of EV kW from sessions
    /// for Demand kW interval of interest.
    pub demand_kw: f64,
    /// Mid-point of energy-based bracket of EV kW from sessions
    /// for Peak 7-7 kW interval of interest.
    pub peak_7_7_kw: f64,

    /// Days in billing period, as the bill counts them.
    pub days_in_period: u8,
    /// `days_in_period / 30`
    pub days_adj_factor: f64,

    /// Distribution charges attributable to EV sessions.
    pub distribution_charges: f64,
    /// Transmission Connection Charge attributable to EV sessions.
    pub transmission_connection_charge: f64,
    /// Transmission Network Charge attributable to EV sessions.
    pub transmission_network_charge: f64,

    /// HST on delivery charges attributable to EV sessions, before OER.
    pub hst: f64,
    /// Onario Electricity Rebate
    pub ontario_electricity_rebate: f64,

    /// Total delivery cost attributable to EV sessions, net of HST and OER.
    pub delivery_cost: f64,
}

/// Why a billing period's figures cannot be turned into peak power estimates, or into the delivery
/// cost drawn from them.
///
/// No variant names a file. Producing this is a computation, and a computation cannot fail to read
/// something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeakPowerError {
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// The billing period carries no reading in one of the power series, so it has no maximum to
    /// estimate against. The feed is expected to carry hourly kW *and* kVA.
    NoPeak {
        period_ending: Date,
        unit: &'static str,
    },
}

impl From<NotABillingPeriodEnding> for PeakPowerError {
    fn from(e: NotABillingPeriodEnding) -> Self {
        Self::NotABillingPeriodEnding(e)
    }
}

impl fmt::Display for PeakPowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABillingPeriodEnding(e) => e.fmt(f),
            Self::NoPeak {
                period_ending,
                unit,
            } => write!(
                f,
                "the billing period ending {period_ending} carries no {unit} reading, so it has no \
                 {unit} maximum to estimate against"
            ),
        }
    }
}

impl Error for PeakPowerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotABillingPeriodEnding(e) => Some(e),
            _ => None,
        }
    }
}

/// The month the delivery lines are priced against: they are levied "per kW per 30 Days", which is
/// where the bill's `Adj.` proration of the demand figures comes from.
const BILLED_DAYS_PER_MONTH: f64 = 30.0;

/// Estimates the net delivery cost attributable to EV charging sessions during a billing period.
///
/// Pure throughout, as [`fn@peak_power`] is: this is everything the call does once the meter
/// export, the session reports and the bill have been read. There is no `io` counterpart yet, so a
/// caller reads the bill with [`hydro_bill_from_pdf`](crate::hydro_bills::hydro_bill_from_pdf) and
/// the rest as [`io::peak_power`](crate::io::peak_power) does.
///
/// # How the figure is arrived at
///
/// Only the three demand-priced delivery lines can be attributed at all. Each is levied on one
/// demand figure, and each demand figure is a maximum over one interval:
///
/// | Bill line | Levied on | Interval of interest |
/// |---|---|---|
/// | Distribution Charges | `Adj. kVA` | the hour the site's kVA peaked in |
/// | Transmission Connection Charge | `Adj. kW` | the hour its kW peaked in |
/// | Transmission Network Charge | `Adj. Peak kW 7-7` | the hour its kW peaked in within 07:00-19:00 |
///
/// For each, the EV share of that same interval is estimated the way [`fn@peak_power`] estimates
/// it, prorated to a 30-day month as the bill prorates its own figure, and priced at the bill's
/// blended rate — the line divided by the adjusted demand it was charged on. "Blended" because a
/// period straddling a rate change carries every delivery line twice, at the old rate and the new,
/// and [`HydroBill`] holds the two added together; the quotient is what was actually charged per
/// kW, whatever the schedule said.
///
/// HST and the Ontario Electricity Rebate are then applied in the bill's own proportions, taken
/// against `total_electricity_charges` rather than from a statutory rate, so a bill that rounds or
/// prorates either of them carries that through to the EV share.
///
/// Everything else on the bill is left out. Consumption is billed by the kilowatt-hour and the
/// customer charge is fixed, so neither turns on which interval the site peaked in and neither
/// belongs in a figure derived from one.
///
/// Note what falls out of pricing at the blended rate: since the rate is a quotient against the
/// *adjusted* demand, and the EV figure is adjusted by the same day count, the proration cancels.
/// Each line comes to the bill's own line times the EV share of the demand it was charged on.
/// [`DeliveryCost::days_adj_factor`] is reported all the same — it is what makes the two adjusted
/// figures comparable, and a reader checking the arithmetic against the bill needs to see it.
///
/// # Arguments
///
/// - `bill` - the Toronto Hydro bill for the period, which supplies every rate and every day count
///   used. Nothing is assumed about the tariff.
/// - `gb_period_values` - that period's figures, read from the meter export.
/// - `sessions` - every session from every report covering the period, as [`fn@peak_power`] takes
///   them.
///
/// The period is not a parameter. `bill` states which one it covers, so passing it alongside would
/// let a caller name two different periods in one call; [`HydroBill::period_end_date`] is the one
/// answer, and every figure here is a proportion of a line on that same bill.
///
/// The session handling is [`fn@peak_power`]'s exactly, duplicates and all: both calls build one
/// [`SessionReport`] from the same records by the same rules, so a cost and the estimates it rests
/// on can never disagree about which sessions there were.
///
/// # Errors
///
/// [`PeakPowerError::NotABillingPeriodEnding`] if the bill's meter reading period does not close on
/// [`BILL_END_DAY`](crate::hydro_bills::BILL_END_DAY), and [`PeakPowerError::NoPeak`] if the period
/// carries no reading in one of the three series.
pub fn peak_power_cost(
    bill: &HydroBill,
    gb_period_values: PeriodValues,
    sessions: &[RSession],
) -> Result<DeliveryCost, PeakPowerError> {
    // An off-cycle bill -- one whose meter reading period does not close a billing period -- is
    // refused rather than estimated from. Its demand figures are levied over a window this does not
    // model, so the proration below would be arithmetic on two different bases.
    let billing_period_ending = bill.period_end_date();
    billing_period_dates(billing_period_ending)?;

    let (sessions_report, sources) = one_report(sessions);
    let estimates = |peak, unit| {
        let ioi = peak_interval(peak, unit, billing_period_ending)?;
        Ok::<_, PeakPowerError>(estimates_from_report(
            ioi,
            sources.clone(),
            &sessions_report,
        ))
    };

    // Each maximum is taken over the interval its own bill line is charged on. Reading all three
    // off one interval would price two of the lines against an hour they were never charged for.
    let demand_kva = energy_based(&estimates(gb_period_values.max_kva, "kVA")?, |e| {
        e.energy_based_kva
    });
    let demand_kw = energy_based(&estimates(gb_period_values.max_kw, "kW")?, |e| {
        e.energy_based_kw
    });
    let peak_7_7_kw = energy_based(&estimates(gb_period_values.max_kw_nop, "kW 7-7")?, |e| {
        e.energy_based_kw
    });

    let days_in_period = bill.number_of_days;
    let days_adj_factor = f64::from(days_in_period) / BILLED_DAYS_PER_MONTH;

    // Each rate is the line as billed over the demand it was billed on, so it carries whatever the
    // bill actually did -- two rate schedules added together, a corrected figure, a rounding.
    let blended_distribution_rate = bill.distribution_charges / bill.adj_kva;
    let blended_transmission_connection_rate = bill.transmission_connection_charge / bill.adj_kw;
    let blended_transmission_network_rate = bill.transmission_network_charge / bill.adj_peak_7_7_kw;

    // The EV demand is prorated before pricing because the rate is per adjusted kW or kVA, which is
    // the prorated figure. Pricing the raw figure at that rate would mix the two bases.
    let distribution_charges = blended_distribution_rate * demand_kva * days_adj_factor;
    let transmission_connection_charge =
        blended_transmission_connection_rate * demand_kw * days_adj_factor;
    let transmission_network_charge =
        blended_transmission_network_rate * peak_7_7_kw * days_adj_factor;
    let charges =
        distribution_charges + transmission_connection_charge + transmission_network_charge;

    // Both as fractions of the bill's own charges rather than as rates of their own. The rebate in
    // particular is a policy percentage that has been changed more than once, and reading it off
    // the bill means a change needs no code.
    let hst = charges * bill.hst / bill.total_electricity_charges;
    let ontario_electricity_rebate =
        charges * bill.ontario_electricity_rebate / bill.total_electricity_charges;

    Ok(DeliveryCost {
        blended_distribution_rate,
        blended_transmission_connection_rate,
        blended_transmission_network_rate,
        demand_kva,
        demand_kw,
        peak_7_7_kw,
        days_in_period,
        days_adj_factor,
        distribution_charges,
        transmission_connection_charge,
        transmission_network_charge,
        hst,
        ontario_electricity_rebate,
        // The bill states its own total the same way -- see `HydroBill::bill_total_amount`. The
        // rebate is held as a positive amount and subtracted, though the bill prints it as a
        // credit.
        delivery_cost: charges + hst - ontario_electricity_rebate,
    })
}

/// One figure off the segment that maximises an interval's energy-based estimate.
///
/// The mid-point of the bracket, because the reported session times are stated only to the minute
/// and the overlap they imply is a range. A bill needs one number, and the mid-point is the only
/// choice that does not systematically over- or under-state the share.
///
/// Both units are read off the one segment [`IntervalEstimates`] already chose, which is the
/// selector's reason for existing: the choice is made on energy-based kW, and the load model is
/// monotone in it — a segment drawing more real power draws more apparent power — so the segment
/// maximising kVA is the same one. Re-choosing per unit could only introduce a disagreement.
fn energy_based(
    estimates: &IntervalEstimates,
    which: impl Fn(&EstimateSet) -> Bracket<f64>,
) -> f64 {
    which(&estimates.energy_based_seg_estimate.1).mid()
}

/// Returns peak power estimates for the intervals of interest that maximize kW and kVA in the
/// specified billing period.
///
/// The reading half of the same call is [`io::peak_power`](crate::io::peak_power), which is where
/// these arguments come from. This is everything that call does once the meter export and the
/// session reports have been read.
///
/// The two intervals are the hours the *building* peaked in, taken from `gb_period_values`, and
/// each estimate says how much of that hour's demand the chargers can account for. They are usually
/// different hours, and occasionally the same one.
///
/// Each interval is a whole metering hour, because that is the resolution the Green Button feed
/// states demand at. The estimate within it is still a 15-minute figure: an
/// [`IntervalEstimates`](crate::sessions::IntervalEstimates) reports the highest of the hour's four segments,
/// which is the basis the demand charge is billed on. See docs/sessions/README.md, "Interval of
/// interest boundaries".
///
/// The maxima used are the period's unrestricted ones — what an invoice bills as `Demand kW` and
/// `Demand kVA` — not the 07:00-19:00 figures it reports as `Peak kW 7-7`.
///
/// # Arguments
/// - `billing_period_ending` - the billing period, named by the date it closes on. Must be
///   [`BILL_END_DAY`](crate::hydro_bills::BILL_END_DAY) of its month.
/// - `gb_period_values` - that period's figures, read from the meter export.
/// - `sessions` - every session from every report covering the period, as read, in the order the
///   reports were read. Not merged: which records describe one session is decided here, since that
///   is a question about the records rather than about the files they came from.
///
/// # Sessions the reports share
///
/// Monthly reports overlap at the month boundary, so a session near it appears in both. A record
/// the two state identically is one session and is counted once; counting it twice would inflate
/// every figure drawn from it.
///
/// A `Charge_Session_ID` carried by two records that are *not* identical does not collapse. Such
/// records are kept and estimated from, both of them, and each is flagged
/// [`AnomalyKind::DuplicateId`](crate::sessions::AnomalyKind::DuplicateId) in the returned
/// [`IntervalEstimates::session_anomalies`](crate::sessions::IntervalEstimates::session_anomalies) — subject
/// to the same scoping as every other anomaly there, so one is listed only if its session reaches
/// that estimate's interval.
///
/// A flag rather than an error, because a shared id is not necessarily a fault.
/// `Charge_Session_ID` is not unique in Evolute's reports — the June 2026 report carries `S37487`
/// on two sessions a week apart, within the one file — so refusing would make that month
/// unestimatable. The flag also cannot tell a reused id from two reports genuinely disagreeing
/// about one session, which is why both records are kept and the judgement is left to a reader who
/// can go back to the source rows.
///
/// # Errors
///
/// [`PeakPowerError::NotABillingPeriodEnding`] if `billing_period_ending` does not label a period,
/// and [`PeakPowerError::NoPeak`] if the period carries no reading in one of the two series. There
/// is no variant naming a file: nothing here reads one.
pub fn peak_power(
    billing_period_ending: Date,
    gb_period_values: PeriodValues,
    sessions: &[RSession],
) -> Result<PowerEstimates, PeakPowerError> {
    // Re-checked rather than assumed. This is an entry point in its own right, and a caller
    // reaching it directly has not been through `io::peak_power`'s validation.
    billing_period_dates(billing_period_ending)?;

    let kw_ioi = peak_interval(gb_period_values.max_kw, "kW", billing_period_ending)?;
    let kva_ioi = peak_interval(gb_period_values.max_kva, "kVA", billing_period_ending)?;

    // Both estimates come off the one report, so the two figures cannot be drawn from different
    // session data.
    let (sessions_report, sources) = one_report(sessions);
    Ok(PowerEstimates {
        kw_estimates: estimates_from_report(kw_ioi, sources.clone(), &sessions_report),
        kva_estimates: estimates_from_report(kva_ioi, sources, &sessions_report),
    })
}

/// The files' sessions as one report, with the files they came from.
///
/// They become one report here rather than arriving as one. Deciding which records are the same
/// session, and what each surviving session is fit for, are both functions of the sessions alone,
/// which is why they sit on this side of the split.
///
/// One list rather than one per file: `merge_sessions` compares each session against everything
/// already kept and never looks at list boundaries, so concatenating the files first gives the same
/// answer as handing them over separately.
///
/// No log paths, because nothing here writes a log. That field records where a *reader* put one.
///
/// Shared by both entry points so that a [`DeliveryCost`] and the [`PowerEstimates`] for the same
/// period cannot be built from different readings of the same records.
fn one_report(sessions: &[RSession]) -> (SessionReport, Vec<PathBuf>) {
    (
        SessionReport::from_session_lists(vec![sessions.to_vec()], Vec::new()),
        sources_of(sessions),
    )
}

/// The files `sessions` were read from, each named once, in the order they first appear.
///
/// Taken from the sessions rather than from a list of paths passed alongside them, so that what a
/// report says it was built from cannot disagree with what it was actually built from.
///
/// A file that contributed no session is therefore not named. That is the intended reading of
/// [`IntervalEstimates::sources`](crate::sessions::IntervalEstimates::sources) — the files the sessions were
/// read from — and a month in which nobody charged contributed nothing to any figure.
fn sources_of(sessions: &[RSession]) -> Vec<PathBuf> {
    let mut sources: Vec<PathBuf> = Vec::new();
    for session in sessions {
        if !sources.iter().any(|p| p == session.path.as_ref()) {
            sources.push(session.path.as_ref().clone());
        }
    }
    sources
}

/// The metering hour a peak occurred in, as an interval of interest.
///
/// # Errors
///
/// [`PeakPowerError::NoPeak`] when the period carries no reading in that series at all.
fn peak_interval(
    peak: Option<Peak>,
    unit: &'static str,
    period_ending: Date,
) -> Result<Interval, PeakPowerError> {
    let peak = peak.ok_or(PeakPowerError::NoPeak {
        period_ending,
        unit,
    })?;
    // Only an interval that starts on the hour can be a peak — see `green_button::peaks` — so this
    // is always a legal interval of interest and needs no further checking.
    Ok(Interval::new(peak.at, METER_INTERVAL))
}

// cargo test --lib -- api::pure::peak_power::test
#[cfg(test)]
mod test {
    use super::*;
    use crate::green_button::BillingPeriod;
    use crate::hydro_bills::BILL_END_DAY;
    use crate::sessions::{AnomalyKind, IntervalEstimates, test_support::session};
    use crate::time::Tou;
    use jiff::Timestamp;
    use jiff::civil::date;
    use std::collections::BTreeMap;

    /// The period every fixture here belongs to: 24 May to 23 June 2026.
    const PERIOD_ENDING: (i16, i8, i8) = (2026, 6, 23);

    /// The hour the building peaked in kW. 16:00 EDT on 10 June.
    const KW_PEAK_HOUR: &str = "2026-06-10T20:00:00Z";
    /// The hour it peaked in kVA — a different hour, as it usually is. 19:00 EDT on 11 June.
    const KVA_PEAK_HOUR: &str = "2026-06-11T23:00:00Z";
    /// The hour it peaked in kW within the 07:00-19:00 demand window. 08:00 EDT on 1 June, a third
    /// hour again, so a figure read off the wrong one is visible.
    const NOP_PEAK_HOUR: &str = "2026-06-01T12:00:00Z";

    fn ts(s: &str) -> Timestamp {
        s.parse().expect("an RFC 3339 timestamp")
    }

    fn period_ending() -> Date {
        date(PERIOD_ENDING.0, PERIOD_ENDING.1, PERIOD_ENDING.2)
    }

    /// A period whose two maxima fall in the hours named, with the figures the estimate does not
    /// read left at zero.
    ///
    /// `peak_power` uses a [`Peak`] for one thing only — when it happened — so the magnitudes are
    /// deliberately not stated. A fixture carrying invented kW values would suggest the estimate
    /// compares itself against them, and it does not: the building's own demand is the bill's, and
    /// what is estimated here is the EV share of the same hour.
    fn period_values(kw_at: Option<&str>, kva_at: Option<&str>) -> PeriodValues {
        period_values_with_nop(kw_at, kva_at, None)
    }

    /// As above, and also stating the hour the 7-7 maximum fell in, which only the cost reads.
    fn period_values_with_nop(
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
            period: BillingPeriod::ending_on(period_ending(), BILL_END_DAY),
            interval_count: 744,
            kwh_total: 0,
            max_kw: kw_at.map(|at| peak(at, Tou::OnPeak)),
            max_kw_nop: nop_at.map(|at| peak(at, Tou::MidPeak)),
            max_kva: kva_at.map(|at| peak(at, Tou::MidPeak)),
            max_kva_nop: None,
            anomaly_counts: BTreeMap::new(),
        }
    }

    /// A bill for the fixture period, with figures chosen so that every rate the cost derives comes
    /// out whole and can be checked by eye.
    ///
    /// 31 days, so the `Adj.` proration is 31/30 and not the identity: `120 * 31/30 = 124`,
    /// `150 * 31/30 = 155`, `90 * 31/30 = 93`. The three delivery lines then divide into blended
    /// rates of 10, 3 and 5, and HST and the rebate into 13% and 10% of the total charges.
    ///
    /// The lines the cost does not read carry figures too, so a test cannot pass by reading one of
    /// them: nothing here is zero.
    fn bill() -> HydroBill {
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
            meter_reading_period_to: period_ending(),
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

    /// The cost for the fixture bill, sessions and meter figures.
    fn cost() -> DeliveryCost {
        peak_power_cost(
            &bill(),
            period_values_with_nop(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR), Some(NOP_PEAK_HOUR)),
            &two_reports(),
        )
        .expect("the bill closes a billing period and it has all three maxima")
    }

    /// Money, to the cent.
    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 0.005
    }

    /// Sessions as the two monthly reports covering this period would yield them.
    ///
    /// Laid out against the kW peak hour, whose four segments start at 20:00, 20:15, 20:30 and
    /// 20:45. `WHOLE` runs the length of the hour; `MID_A` and `MID_B` are 14 minutes from 20:15,
    /// which with the one-minute padding on the adjusted end is exactly the 20:15 segment and no
    /// other. That segment therefore holds three sessions and every other holds one, which is what
    /// makes the maximum predictable without depending on any electrical constant.
    ///
    /// `EVENING` sits in the kVA peak hour instead, and `ELSEWHERE` in neither, so the two
    /// estimates cannot accidentally agree.
    fn two_reports() -> Vec<RSession> {
        vec![
            session("May.csv", 2, "MAY_ONLY", "2026-05-26T18:00:00Z", 30, 2.0),
            session("June.csv", 2, "WHOLE", KW_PEAK_HOUR, 60, 6.0),
            session("June.csv", 3, "MID_A", "2026-06-10T20:15:00Z", 14, 1.0),
            session("June.csv", 4, "MID_B", "2026-06-10T20:15:00Z", 14, 1.0),
            session("June.csv", 5, "EVENING", "2026-06-11T23:45:00Z", 14, 1.0),
            session("June.csv", 6, "ELSEWHERE", "2026-06-01T12:00:00Z", 30, 2.0),
        ]
    }

    /// The ids a test expects in one segment.
    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    /// The ids in each segment of an estimate, in segment order.
    fn membership(estimates: &IntervalEstimates) -> Vec<Vec<String>> {
        estimates
            .seg_estimates
            .iter()
            .map(|(seg, _)| seg.sessions.iter().map(|s| s.id.clone()).collect())
            .collect()
    }

    /// Each estimate covers the metering hour its own maximum fell in, and reports the highest of
    /// that hour's four segments.
    #[test]
    fn each_estimate_covers_the_hour_its_own_maximum_fell_in() {
        let estimates = peak_power(
            period_ending(),
            period_values(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR)),
            &two_reports(),
        )
        .expect("the period has both maxima");

        assert_eq!(estimates.kw_estimates.interval.start, ts(KW_PEAK_HOUR));
        assert_eq!(estimates.kw_estimates.interval.duration, METER_INTERVAL);
        assert_eq!(estimates.kva_estimates.interval.start, ts(KVA_PEAK_HOUR));

        // The hour is tiled by four 15-minute segments, and only sessions reaching the hour appear.
        assert_eq!(
            membership(&estimates.kw_estimates),
            vec![
                ids(&["WHOLE"]),
                ids(&["WHOLE", "MID_A", "MID_B"]),
                ids(&["WHOLE"]),
                ids(&["WHOLE"]),
            ]
        );
        // The busiest segment is the maximum on both derivations.
        let (energy_seg, _) = &estimates.kw_estimates.energy_based_seg_estimate;
        let (count_seg, _) = &estimates.kw_estimates.count_based_seg_estimate;
        assert_eq!(energy_seg.start(), ts("2026-06-10T20:15:00Z"));
        assert_eq!(count_seg.start(), ts("2026-06-10T20:15:00Z"));

        // The other hour is a different hour, holding a different session.
        assert_eq!(
            membership(&estimates.kva_estimates),
            vec![ids(&[]), ids(&[]), ids(&[]), ids(&["EVENING"])]
        );
    }

    /// The files the sessions came from, named once each, in the order they were read.
    #[test]
    fn the_report_names_the_files_its_sessions_came_from() {
        let estimates = peak_power(
            period_ending(),
            period_values(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR)),
            &two_reports(),
        )
        .unwrap();
        assert_eq!(
            estimates.kw_estimates.sources,
            [PathBuf::from("May.csv"), PathBuf::from("June.csv")]
        );
        // Both estimates come off the same sessions, so both say the same thing about their source.
        assert_eq!(
            estimates.kw_estimates.sources,
            estimates.kva_estimates.sources
        );
    }

    /// The overlap case: a session at the end of May appears in both months' reports, stated
    /// identically. It is one session, and counting it twice would inflate every figure it enters.
    #[test]
    fn a_session_both_reports_state_identically_is_counted_once() {
        let one_copy = two_reports();
        let mut two_copies = one_copy.clone();
        // The same session as June's `WHOLE`, as May's report states it: its own row in its own
        // file, and every figure the same.
        two_copies.insert(1, session("May.csv", 9, "WHOLE", KW_PEAK_HOUR, 60, 6.0));

        let estimate = |sessions: &[RSession]| {
            peak_power(
                period_ending(),
                period_values(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR)),
                sessions,
            )
            .unwrap()
        };

        assert_eq!(
            membership(&estimate(&two_copies).kw_estimates),
            membership(&estimate(&one_copy).kw_estimates),
            "the duplicate should leave the segments as they were"
        );
        assert!(
            estimate(&two_copies)
                .kw_estimates
                .session_anomalies
                .is_empty(),
            "one session reported twice is not an anomaly"
        );
    }

    /// The other case: two records share an id but describe different sessions. Evolute reuses ids,
    /// so both count, and both are flagged for a reader rather than one being discarded.
    #[test]
    fn a_reused_id_keeps_both_sessions_and_flags_them() {
        let mut sessions = two_reports();
        // Same id as `MID_A`, a different session entirely — a week earlier, in another file.
        sessions.push(session(
            "May.csv",
            7,
            "MID_A",
            "2026-06-10T20:15:00Z",
            30,
            2.0,
        ));

        let estimates = peak_power(
            period_ending(),
            period_values(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR)),
            &sessions,
        )
        .unwrap();

        // Both are in the 20:15 segment, so the id appears there twice.
        let ids = &membership(&estimates.kw_estimates)[1];
        assert_eq!(ids.iter().filter(|id| *id == "MID_A").count(), 2);

        let flagged: Vec<_> = estimates
            .kw_estimates
            .session_anomalies
            .iter()
            .filter(|a| a.kind == AnomalyKind::DuplicateId)
            .map(|a| (a.session.id.as_str(), a.session.row))
            .collect();
        // Symmetric: the earlier record is flagged as well as the later one.
        assert_eq!(flagged, [("MID_A", 3), ("MID_A", 7)]);
    }

    /// A period with no reading in one of the two series has no maximum to estimate against, and
    /// says which series is missing rather than returning a figure of zero.
    #[test]
    fn a_period_missing_a_series_has_no_estimate_for_it() {
        let err = peak_power(
            period_ending(),
            period_values(Some(KW_PEAK_HOUR), None),
            &two_reports(),
        )
        .err()
        .expect("the period carries no kVA reading");
        assert!(
            matches!(err, PeakPowerError::NoPeak { unit: "kVA", .. }),
            "{err}"
        );

        let err = peak_power(
            period_ending(),
            period_values(None, Some(KVA_PEAK_HOUR)),
            &two_reports(),
        )
        .err()
        .expect("the period carries no kW reading");
        assert!(
            matches!(err, PeakPowerError::NoPeak { unit: "kW", .. }),
            "{err}"
        );
    }

    /// Each demand figure is read off the hour its own bill line was charged on, not off a single
    /// hour used for all three.
    ///
    /// The three fixture hours hold different sessions, so a figure taken from the wrong one shows
    /// as a wrong number rather than as a coincidence.
    #[test]
    fn each_bill_line_is_priced_on_the_hour_it_was_charged_for() {
        let cost = cost();
        let estimates = peak_power(
            period_ending(),
            period_values(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR)),
            &two_reports(),
        )
        .unwrap();

        // The two unrestricted figures are the ones `peak_power` reports for the same period, so
        // the cost and the estimates behind it cannot state different demands.
        assert_eq!(
            cost.demand_kw,
            estimates
                .kw_estimates
                .energy_based_seg_estimate
                .1
                .energy_based_kw
                .mid()
        );
        assert_eq!(
            cost.demand_kva,
            estimates
                .kva_estimates
                .energy_based_seg_estimate
                .1
                .energy_based_kva
                .mid()
        );

        // The 7-7 hour is a third hour holding only `ELSEWHERE`, which is a smaller load than the
        // three sessions in the kW peak hour. All three figures are distinct and none is zero.
        assert!(cost.peak_7_7_kw > 0.0);
        assert!(
            cost.peak_7_7_kw < cost.demand_kw,
            "7-7 {} should be under the unrestricted {}",
            cost.peak_7_7_kw,
            cost.demand_kw
        );
        assert_ne!(cost.demand_kw, cost.demand_kva);
    }

    /// The rates are the bill's lines over the demand each was charged on, and the day proration is
    /// the bill's own.
    #[test]
    fn the_rates_are_the_bills_own_lines_over_the_demand_they_were_charged_on() {
        let cost = cost();
        assert!(close(cost.blended_distribution_rate, 10.0));
        assert!(close(cost.blended_transmission_connection_rate, 3.0));
        assert!(close(cost.blended_transmission_network_rate, 5.0));
        assert_eq!(cost.days_in_period, 31);
        assert!(close(cost.days_adj_factor, 31.0 / 30.0));
    }

    /// Each delivery line comes to the bill's line times the EV share of the demand it was charged
    /// on — the day proration cancels between the rate and the EV figure.
    ///
    /// Derived independently of the code under test: it never divides by a bill line, so this would
    /// fail if the proration were applied on one side only, or omitted from the rate, or applied
    /// twice.
    #[test]
    fn each_delivery_line_is_the_bills_line_times_the_ev_share_of_that_demand() {
        let (cost, bill) = (cost(), bill());
        assert!(close(
            cost.distribution_charges,
            bill.distribution_charges * cost.demand_kva / bill.demand_kva
        ));
        assert!(close(
            cost.transmission_connection_charge,
            bill.transmission_connection_charge * cost.demand_kw / bill.demand_kw
        ));
        assert!(close(
            cost.transmission_network_charge,
            bill.transmission_network_charge * cost.peak_7_7_kw / bill.peak_7_7_kw
        ));
    }

    /// HST and the rebate are the bill's own proportions of the EV charges, and the total is the
    /// charges plus the one less the other — the shape `HydroBill::bill_total_amount` uses.
    #[test]
    fn tax_and_rebate_follow_the_bills_own_proportions() {
        let cost = cost();
        let charges = cost.distribution_charges
            + cost.transmission_connection_charge
            + cost.transmission_network_charge;
        // 1300 / 10000 and 1000 / 10000 on the fixture bill.
        assert!(close(cost.hst, charges * 0.13));
        assert!(close(cost.ontario_electricity_rebate, charges * 0.10));
        assert!(close(
            cost.delivery_cost,
            charges + cost.hst - cost.ontario_electricity_rebate
        ));
        // The rebate is smaller than the tax on these proportions, so the total exceeds the charges.
        assert!(cost.delivery_cost > charges);
    }

    /// The 7-7 maximum is a series of its own. A period carrying none has no network charge to
    /// apportion, and says so rather than pricing that line at zero.
    #[test]
    fn a_period_missing_the_7_7_maximum_has_no_cost() {
        let err = peak_power_cost(
            &bill(),
            period_values(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR)),
            &two_reports(),
        )
        .err()
        .expect("the period carries no 7-7 maximum");
        assert!(
            matches!(err, PeakPowerError::NoPeak { unit: "kW 7-7", .. }),
            "{err}"
        );
    }

    /// Every figure is a proportion of a bill line, so a bill from another month would give a
    /// plausible number resting on nothing.
    #[test]
    fn the_cost_is_for_the_period_the_bill_covers() {
        // The same sessions and the same meter figures, against a bill for the month before. The
        // period is not a parameter, so this is a different call and not a mismatch: it prices May's
        // rates over the hours June peaked in, and says so.
        let mut may = bill();
        may.meter_reading_period_from = date(2026, 4, 23);
        may.meter_reading_period_to = date(2026, 5, 23);
        may.number_of_days = 30;
        may.distribution_charges = 775.0;

        let june = cost();
        let cost = peak_power_cost(
            &may,
            period_values_with_nop(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR), Some(NOP_PEAK_HOUR)),
            &two_reports(),
        )
        .expect("May closes a billing period too");

        assert_eq!(cost.days_in_period, 30);
        assert!(close(cost.days_adj_factor, 1.0));
        // The EV demand is the same -- it comes off the meter and the sessions, which did not
        // change -- and only the money moved, so the bill is the only thing that period selects.
        assert_eq!(cost.demand_kva, june.demand_kva);
        assert!(cost.distribution_charges < june.distribution_charges);
    }

    /// Reached directly rather than through `io::peak_power`, this still checks its own arguments.
    #[test]
    fn a_date_that_does_not_close_a_billing_period_is_refused() {
        let err = peak_power(
            date(2026, 6, 30),
            period_values(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR)),
            &two_reports(),
        )
        .err()
        .expect("30 June does not label a billing period");
        assert!(
            matches!(err, PeakPowerError::NotABillingPeriodEnding(_)),
            "{err}"
        );

        // The cost takes no date, so the same refusal reaches it through the bill: an off-cycle
        // bill, whose meter reading period does not close on the 23rd, is not something to prorate
        // to a 30-day month.
        let mut off_cycle = bill();
        off_cycle.meter_reading_period_to = date(2026, 6, 30);
        let err = peak_power_cost(
            &off_cycle,
            period_values_with_nop(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR), Some(NOP_PEAK_HOUR)),
            &two_reports(),
        )
        .err()
        .expect("30 June does not label a billing period");
        assert!(
            matches!(err, PeakPowerError::NotABillingPeriodEnding(_)),
            "{err}"
        );
    }
}
