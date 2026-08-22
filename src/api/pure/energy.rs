//! The EV share of a billing period's energy, split by time-of-use period, and what that share
//! cost.
//!
//! [`fn@energy`] answers the first: how many kilowatt-hours the chargers drew in each of the three
//! price bands. [`energy_cost`] prices those against the bill, at the rate the bill itself charged
//! for each band.
//!
//! The demand side of the bill is [`peak_power`](mod@super::peak_power): what the chargers
//! contributed to the hour the site peaked in. This is the other side, consumption, which is billed
//! by the kilowatt-hour at a rate that depends on when it was drawn.

use crate::{
    hydro_bill::{
        BILL_END_DAY, BillingPeriod, NotABillingPeriodEnding, ZeroDenominator,
        billing_period_dates, billing_period_span,
    },
    markdown::{Left, Right, amounts, field, h1, h2, rounding_note, table},
    session::{Sessions, tou_kwh},
    time::Interval,
};

use jiff::civil::Date;
use std::{error::Error, fmt};

// Re-exported because the two functions here take these and return those, and a caller should not
// have to know which module they come from in order to spell the call. `Tou` for the same reason
// one level down: it is what `EnergyError::NoRate` carries, and matching on that names it.
pub use crate::hydro_bill::HydroBill;
pub use crate::session::{RSession, TouKwh};
pub use crate::time::Tou;

/// EV energy over a billing period, split by time-of-use band.
///
/// The period is carried rather than left to the caller because the three figures mean nothing
/// without it: they are what was drawn *within* it, and a session outside contributes none of it.
#[derive(Debug)]
pub struct Energy {
    /// The billing period these figures are for, named by the date it closes on.
    pub billing_period_ending: Date,
    /// EV energy used by TOU.
    pub kwh: TouKwh,
}

/// Breakdown of energy costs attributable to EV sessions in a billing period.
#[derive(Debug)]
pub struct EnergyCost {
    /// The billing period these figures are for, named by the date it closes on. The bill's own,
    /// since every figure below is a proportion of one of its lines.
    pub billing_period_ending: Date,
    /// EV energy used by TOU.
    pub kwh: TouKwh,
    /// Loss factor adjustment from bill.
    pub loss_factor_adjustment: f64,
    /// EV energy used by TOU, multiplied by loss factor adjustment.
    pub adjusted_kwh: TouKwh,

    /// Toronto Hydro blended nominal on-peak rate.
    pub th_on_peak_rate: f64,
    /// Toronto Hydro blended nominal mid-peak rate.
    pub th_mid_peak_rate: f64,
    /// Toronto Hydro blended nominal off-peak rate.
    pub th_off_peak_rate: f64,

    /// On-peak energy cost attributable to EV sessions:
    /// `adjusted_kwh.on_peak * th_on_peak_rate`.
    pub on_peak_cost: f64,
    /// Mid-peak energy cost attributable to EV sessions:
    /// `adjusted_kwh.mid_peak * th_mid_peak_rate`.
    pub mid_peak_cost: f64,
    /// Off-peak energy cost attributable to EV sessions:
    /// `adjusted_kwh.off_peak * th_off_peak_rate`.
    pub off_peak_cost: f64,

    /// HST on energy charges attributable to EV sessions, before OER.
    pub hst: f64,
    /// Ontario Electricity Rebate
    pub ontario_electricity_rebate: f64,

    /// Total energy cost attributable to EV sessions, net of HST and OER.
    pub energy_cost: f64,
}

/// Why a billing period's sessions cannot be turned into an energy attribution, or into the cost
/// drawn from it.
///
/// No variant names a file. Producing this is a computation, and a computation cannot fail to read
/// something. [`NotABillingPeriodEnding`] is embedded rather than restated, which is what
/// [`CoverageError`](super::coverage::CoverageError) and
/// [`PeakPowerError`](super::peak_power::PeakPowerError) do with the same failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnergyError {
    /// The date given does not close a billing period: it is not [`BILL_END_DAY`] of its month.
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// The bill reports no consumption in one of the three time-of-use bands, so it states no rate
    /// for that band and the EV share of it cannot be priced.
    ///
    /// Refused rather than treated as a rate of zero. The quotient is undefined, not small, and a
    /// band the site drew nothing in is a band the chargers cannot have drawn anything in either --
    /// so a bill saying otherwise disagrees with the reports, and neither figure can be trusted
    /// over the other.
    NoRate { period_ending: Date, tou: Tou },

    /// A bill figure the cost divides by is zero. See [`ZeroDenominator`].
    ///
    /// Distinct from [`Self::NoRate`], which is the same arithmetic on a time-of-use band and
    /// carries the band it happened in.
    ZeroDenominator(ZeroDenominator),
}

impl From<NotABillingPeriodEnding> for EnergyError {
    fn from(e: NotABillingPeriodEnding) -> Self {
        Self::NotABillingPeriodEnding(e)
    }
}

impl From<ZeroDenominator> for EnergyError {
    fn from(e: ZeroDenominator) -> Self {
        Self::ZeroDenominator(e)
    }
}

impl fmt::Display for EnergyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABillingPeriodEnding(e) => e.fmt(f),
            Self::NoRate { period_ending, tou } => {
                let band = band_name(*tou);
                write!(
                    f,
                    "the bill for the billing period ending {period_ending} reports no {band} \
                     consumption, so it states no {band} rate to price the EV share at"
                )
            }
            Self::ZeroDenominator(e) => e.fmt(f),
        }
    }
}

impl Error for EnergyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotABillingPeriodEnding(e) => Some(e),
            Self::ZeroDenominator(e) => Some(e),
            Self::NoRate { .. } => None,
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
pub fn energy(billing_period_ending: Date, sessions: &[RSession]) -> Result<Energy, EnergyError> {
    billing_period_dates(billing_period_ending)?;

    let period = BillingPeriod::ending_on(billing_period_ending, BILL_END_DAY);
    let time_range = Interval::from_start_end(period.start, period.end);

    Ok(Energy {
        billing_period_ending,
        kwh: tou_kwh(time_range, &countable(sessions)),
    })
}

/// The sessions whose energy may be placed on a timeline, from a list that may state one of them
/// more than once.
///
/// Two things at once, and both are needed by every figure drawn from sessions.
///
/// Deduplicated, because a caller supplying overlapping reports states a session twice and summing
/// as given would count its energy twice.
///
/// Two of the three report buckets. `excluded` is left out rather than overlooked: those records'
/// start, end and duration contradict each other, and an inverted one panics in `adj_duration`
/// before any figure comes of it.
pub(super) fn countable(sessions: &[RSession]) -> Vec<RSession> {
    let report = Sessions::from_session_lists(vec![sessions.to_vec()], Vec::new());
    report
        .sessions
        .iter()
        .chain(&report.spikes)
        .cloned()
        .collect()
}

/// Estimates the net energy cost attributable to EV charging sessions during a billing period.
///
/// # How the figure is arrived at
///
/// The EV kilowatt-hours in each of the three time-of-use bands are taken as [`fn@energy`] takes
/// them, raised by the bill's loss factor, and priced at the bill's own rate for that band — the
/// band's cost line divided by the consumption it was charged on. "Blended" because a period
/// straddling a rate change carries each band's line twice, at the old rate and the new, and
/// [`HydroBill`] holds the two added together; the quotient is what was actually charged per kWh,
/// whatever the schedule said.
///
/// The loss factor is applied because the bill's own rates are per *adjusted* kilowatt-hour: its
/// three time-of-use consumption lines sum to `adjusted_kwh_used`, not to `kwh_used`. Pricing an
/// unadjusted EV figure at such a rate would understate the share by the loss factor.
///
/// HST and the Ontario Electricity Rebate are then applied in the bill's own proportions, taken
/// against `total_electricity_charges` rather than from a statutory rate, so a bill that rounds or
/// prorates either of them carries that through to the EV share.
///
/// Only the three time-of-use lines are attributed. The three demand-priced delivery lines are
/// [`peak_power_cost`](super::peak_power::peak_power_cost)'s, since they turn on which interval the
/// site peaked in rather than on how much was drawn. The customer charge and the standard supply
/// administration charge are fixed. The wholesale market service charge is levied per kilowatt-hour
/// and so is attributable in principle, but is not among the figures returned.
///
/// # Arguments
///
/// - `bill` - the Toronto Hydro bill for the period, which supplies the period, the loss factor and
///   every rate used. Nothing is assumed about the tariff.
/// - `sessions` - every session from every report covering the period, as [`fn@energy`] takes them,
///   with the same obligation to supply all of them and the same treatment of duplicates and of
///   records that contradict themselves.
///
/// The period is not a parameter. `bill` states which one it covers, so passing it alongside would
/// let a caller name two different periods in one call, and every figure here is a proportion of a
/// line on that same bill.
///
/// # Errors
///
/// [`EnergyError::NotABillingPeriodEnding`] if the bill's meter reading period does not close on
/// [`BILL_END_DAY`], and [`EnergyError::NoRate`] if the bill reports no consumption in one of the
/// three bands.
pub fn energy_cost(bill: &HydroBill, sessions: &[RSession]) -> Result<EnergyCost, EnergyError> {
    // An off-cycle bill -- one whose meter reading period does not close a billing period -- is
    // refused rather than estimated from, because `energy` sums over a period this does not model.
    // The check is `energy`'s own; there is no second one here.
    let billing_period_ending = bill.period_end_date();
    let kwh = energy(billing_period_ending, sessions)?.kwh;

    let loss_factor_adjustment = bill.loss_factor_adjustment;
    let adjusted_kwh = TouKwh {
        on_peak: kwh.on_peak * loss_factor_adjustment,
        mid_peak: kwh.mid_peak * loss_factor_adjustment,
        off_peak: kwh.off_peak * loss_factor_adjustment,
    };

    let rate = |cost, band_kwh, tou| blended_rate(cost, band_kwh, tou, billing_period_ending);
    let th_on_peak_rate = rate(bill.on_peak_cost, bill.on_peak_kwh, Tou::OnPeak)?;
    let th_mid_peak_rate = rate(bill.mid_peak_cost, bill.mid_peak_kwh, Tou::MidPeak)?;
    let th_off_peak_rate = rate(bill.off_peak_cost, bill.off_peak_kwh, Tou::OffPeak)?;

    let on_peak_cost = adjusted_kwh.on_peak * th_on_peak_rate;
    let mid_peak_cost = adjusted_kwh.mid_peak * th_mid_peak_rate;
    let off_peak_cost = adjusted_kwh.off_peak * th_off_peak_rate;
    let charges = on_peak_cost + mid_peak_cost + off_peak_cost;

    // Both as fractions of the bill's own charges rather than as rates of their own. The rebate in
    // particular is a policy percentage that has been changed more than once, and reading it off
    // the bill means a change needs no code.
    let total_charges =
        bill.divisor("Total Electricity Charges", bill.total_electricity_charges)?;
    let hst = charges * bill.hst / total_charges;
    let ontario_electricity_rebate = charges * bill.ontario_electricity_rebate / total_charges;

    Ok(EnergyCost {
        billing_period_ending,
        kwh,
        loss_factor_adjustment,
        adjusted_kwh,
        th_on_peak_rate,
        th_mid_peak_rate,
        th_off_peak_rate,
        on_peak_cost,
        mid_peak_cost,
        off_peak_cost,
        hst,
        ontario_electricity_rebate,
        // The bill states its own total the same way -- see `HydroBill::bill_total_amount`. The
        // rebate is held as a positive amount and subtracted, though the bill prints it as a
        // credit.
        energy_cost: charges + hst - ontario_electricity_rebate,
    })
}

/// What the bill charged per adjusted kilowatt-hour in one time-of-use band: its cost line over the
/// consumption that line was levied on.
///
/// # Errors
///
/// [`EnergyError::NoRate`] when `band_kwh` is zero, which leaves the quotient undefined.
fn blended_rate(
    cost: f64,
    band_kwh: f64,
    tou: Tou,
    period_ending: Date,
) -> Result<f64, EnergyError> {
    if band_kwh == 0.0 {
        return Err(EnergyError::NoRate { period_ending, tou });
    }
    Ok(cost / band_kwh)
}

impl fmt::Display for Energy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kwh = &self.kwh;
        let rows = vec![
            band_row("On-peak", kwh.on_peak),
            band_row("Mid-peak", kwh.mid_peak),
            band_row("Off-peak", kwh.off_peak),
            band_row("Total", kwh.total_kwh()),
        ];
        writeln!(f, "{}\n", h1("EV Energy"))?;
        writeln!(
            f,
            "{}\n",
            field("Period", &billing_period_span(self.billing_period_ending))
        )?;
        writeln!(f, "{}\n", table(&["TOU", "kWh"], &rows, &[Left, Right]))?;
        writeln!(f, "{}", rounding_note())
    }
}

impl fmt::Display for EnergyCost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let band = |name: &str, kwh: f64, adj: f64, rate: f64, cost: f64| {
            vec![
                name.to_owned(),
                format!("{kwh:.3}"),
                format!("{adj:.3}"),
                format!("{rate:.5}"),
                format!("{cost:.2}"),
            ]
        };
        let charges = self.on_peak_cost + self.mid_peak_cost + self.off_peak_cost;
        let rows = vec![
            band(
                "On-peak",
                self.kwh.on_peak,
                self.adjusted_kwh.on_peak,
                self.th_on_peak_rate,
                self.on_peak_cost,
            ),
            band(
                "Mid-peak",
                self.kwh.mid_peak,
                self.adjusted_kwh.mid_peak,
                self.th_mid_peak_rate,
                self.mid_peak_cost,
            ),
            band(
                "Off-peak",
                self.kwh.off_peak,
                self.adjusted_kwh.off_peak,
                self.th_off_peak_rate,
                self.off_peak_cost,
            ),
            // No rate on the total line: the three differ, so one there would be read as a fourth
            // rate rather than as the absence of one.
            vec![
                "Total before HST & OER".to_owned(),
                format!("{:.3}", self.kwh.total_kwh()),
                format!("{:.3}", self.adjusted_kwh.total_kwh()),
                String::new(),
                format!("{charges:.2}"),
            ],
        ];

        writeln!(f, "{}\n", h1("EV Energy Cost"))?;
        writeln!(
            f,
            "{}",
            field("Period", &billing_period_span(self.billing_period_ending))
        )?;
        writeln!(
            f,
            "{}\n",
            field(
                "Loss factor",
                &format!("{:.4}", self.loss_factor_adjustment)
            )
        )?;
        writeln!(
            f,
            "{}\n",
            table(
                &["TOU", "kWh", "Adj. kWh", "TH blended rate", "Cost"],
                &rows,
                &[Left, Right, Right, Right, Right],
            )
        )?;

        writeln!(f, "{}\n", h2("EV Energy Cost (after HST & OER)"))?;
        writeln!(
            f,
            "{}",
            amounts(&[
                ("Energy charges", charges),
                ("HST", self.hst),
                // Negative because it is a credit. The bill prints it as one, and a column that is
                // alternately added and subtracted cannot be checked by eye.
                (
                    "Ontario Electricity Rebate",
                    -self.ontario_electricity_rebate
                ),
                ("Energy cost", self.energy_cost),
            ])
        )?;
        writeln!(f, "\n{}", rounding_note())
    }
}

/// One `| TOU | kWh |` row.
fn band_row(name: &str, kwh: f64) -> Vec<String> {
    vec![name.to_owned(), format!("{kwh:.3}")]
}

/// A band's name as a bill and a reader spell it.
///
/// Not [`Tou::as_str`], which is documented as a wire format for workbook column names and spells
/// these `OnPeak`. This is prose, and goes into a sentence.
fn band_name(tou: Tou) -> &'static str {
    match tou {
        Tou::OnPeak => "on-peak",
        Tou::MidPeak => "mid-peak",
        Tou::OffPeak => "off-peak",
    }
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
            .kwh
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

    /// A bill for the fixture period, with figures chosen so that every rate the cost derives comes
    /// out whole and can be checked by eye.
    ///
    /// The three time-of-use consumption lines sum to `adjusted_kwh_used` and not to `kwh_used`,
    /// which is how a real bill states them, so a rate derived from them is per adjusted kWh. They
    /// divide into rates of 0.20, 0.15 and 0.10, and HST and the rebate into 13% and 10% of the
    /// total charges.
    ///
    /// The loss factor is 1.05 rather than 1, so a figure priced without it is a different number.
    ///
    /// The lines the cost does not read carry figures too, so a test cannot pass by reading one of
    /// them: nothing here is zero.
    /// The energy cost divides by the bill's total charges to take HST and the rebate in the bill's
    /// own proportions. Zero is refused rather than divided by.
    #[test]
    fn a_bill_stating_no_total_charges_is_refused() {
        let mut b = bill();
        b.total_electricity_charges = 0.0;
        let err = energy_cost(&b, &sessions()).expect_err("a zero divisor");
        let EnergyError::ZeroDenominator(z) = &err else {
            panic!("expected a ZeroDenominator, got {err}");
        };
        assert_eq!(z.figure, "Total Electricity Charges", "{err}");

        // A band stating no consumption keeps its own kind, which carries the band rather than the
        // bill figure. The two are the same arithmetic read two ways and stay apart.
        let mut b = bill();
        b.on_peak_kwh = 0.0;
        let err = energy_cost(&b, &sessions()).expect_err("no on-peak rate");
        assert!(matches!(err, EnergyError::NoRate { .. }), "{err}");
    }

    fn bill() -> HydroBill {
        HydroBill {
            statement_date: date(2026, 6, 28),
            on_peak_kwh: 10000.0,
            mid_peak_kwh: 10000.0,
            off_peak_kwh: 30000.0,
            on_peak_cost: 2000.0,
            mid_peak_cost: 1500.0,
            off_peak_cost: 3000.0,
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
            kwh_used: 47619.048,
            loss_factor_adjustment: 1.05,
            adjusted_kwh_used: 50000.0,
            peak_7_7_kw: 90.0,
            adj_peak_7_7_kw: 93.0,
            demand_kw: 120.0,
            demand_kva: 150.0,
            metering_adj: 1.0,
            adj_kw: 124.0,
            adj_kva: 155.0,
        }
    }

    /// One session in each band, on Wednesday 10 June: 02:00 EDT is off-peak, 08:00 is mid-peak and
    /// 12:00 is on-peak in the summer schedule. Three different amounts of energy, so a figure
    /// taken from the wrong band shows as a wrong number.
    fn sessions() -> Vec<RSession> {
        vec![
            session("June.csv", 2, "OFF", "2026-06-10T06:00:00Z", 60, 7.0),
            session("June.csv", 3, "MID", "2026-06-10T12:00:00Z", 60, 3.0),
            session("June.csv", 4, "ON", "2026-06-10T16:00:00Z", 60, 5.0),
        ]
    }

    fn cost() -> EnergyCost {
        energy_cost(&bill(), &sessions()).expect("the bill closes a billing period")
    }

    /// Money, to the cent.
    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 0.005
    }

    /// The kilowatt-hours priced are the ones [`fn@energy`] reports for the same period, so a cost
    /// and the attribution behind it can never state different energy.
    #[test]
    fn the_energy_priced_is_the_energy_attributed() {
        let cost = cost();
        let kwh = energy(period_ending_date(), &sessions()).unwrap().kwh;
        assert_eq!(cost.kwh.on_peak, kwh.on_peak);
        assert_eq!(cost.kwh.mid_peak, kwh.mid_peak);
        assert_eq!(cost.kwh.off_peak, kwh.off_peak);
        // The report heads itself with the period, taken from the bill rather than passed in.
        assert_eq!(cost.billing_period_ending, period_ending_date());
        // Each fixture session lands whole in one band, so all three are non-zero and distinct --
        // otherwise the assertions below could pass on zeros.
        assert!(close(cost.kwh.on_peak, 5.0));
        assert!(close(cost.kwh.mid_peak, 3.0));
        assert!(close(cost.kwh.off_peak, 7.0));
    }

    /// The rates are the bill's own cost lines over the consumption each was charged on.
    #[test]
    fn the_rates_are_the_bills_own_lines_over_the_consumption_they_were_charged_on() {
        let cost = cost();
        assert!(close(cost.th_on_peak_rate, 0.20));
        assert!(close(cost.th_mid_peak_rate, 0.15));
        assert!(close(cost.th_off_peak_rate, 0.10));
    }

    /// The bill's rates are per adjusted kilowatt-hour, so the EV figure is raised by the loss
    /// factor before it is priced.
    ///
    /// Stated as the bill's line times the EV share of that band's consumption, which never divides
    /// by a rate: the same figure arrived at from the other direction. It would fail if the loss
    /// factor were left out, or applied twice.
    #[test]
    fn each_band_is_priced_on_the_loss_adjusted_energy() {
        let (cost, bill) = (cost(), bill());
        assert_eq!(cost.loss_factor_adjustment, 1.05);
        assert!(close(cost.adjusted_kwh.on_peak, 5.0 * 1.05));

        assert!(close(
            cost.on_peak_cost,
            bill.on_peak_cost * cost.adjusted_kwh.on_peak / bill.on_peak_kwh
        ));
        assert!(close(
            cost.mid_peak_cost,
            bill.mid_peak_cost * cost.adjusted_kwh.mid_peak / bill.mid_peak_kwh
        ));
        assert!(close(
            cost.off_peak_cost,
            bill.off_peak_cost * cost.adjusted_kwh.off_peak / bill.off_peak_kwh
        ));

        // 5 * 1.05 * 0.20, 3 * 1.05 * 0.15 and 7 * 1.05 * 0.10.
        assert!(close(cost.on_peak_cost, 1.05));
        assert!(close(cost.mid_peak_cost, 0.4725));
        assert!(close(cost.off_peak_cost, 0.735));
    }

    /// HST and the rebate are the bill's own proportions of the EV charges, and the total is the
    /// charges plus the one less the other -- the shape `HydroBill::bill_total_amount` uses.
    #[test]
    fn tax_and_rebate_follow_the_bills_own_proportions() {
        let cost = cost();
        let charges = cost.on_peak_cost + cost.mid_peak_cost + cost.off_peak_cost;
        // 1300 / 10000 and 1000 / 10000 on the fixture bill.
        assert!(close(cost.hst, charges * 0.13));
        assert!(close(cost.ontario_electricity_rebate, charges * 0.10));
        assert!(close(
            cost.energy_cost,
            charges + cost.hst - cost.ontario_electricity_rebate
        ));
        // The rebate is smaller than the tax on these proportions, so the total exceeds the charges.
        assert!(cost.energy_cost > charges);
    }

    /// A band the bill reports no consumption in states no rate, and the quotient would be infinite
    /// or NaN. Refused, naming the band, rather than carried into a dollar figure.
    #[test]
    fn a_band_the_bill_reports_no_consumption_in_has_no_rate() {
        let (mut on, mut mid, mut off) = (bill(), bill(), bill());
        on.on_peak_kwh = 0.0;
        mid.mid_peak_kwh = 0.0;
        off.off_peak_kwh = 0.0;

        for (band, bill) in [(Tou::OnPeak, on), (Tou::MidPeak, mid), (Tou::OffPeak, off)] {
            let err = energy_cost(&bill, &sessions())
                .err()
                .unwrap_or_else(|| panic!("the bill reports no {band} consumption"));
            assert!(
                matches!(err, EnergyError::NoRate { tou, .. } if tou == band),
                "{err}"
            );
            // The message names the band in prose, not as the workbook token.
            assert!(err.to_string().contains(band_name(band)), "{err}");
        }
    }

    /// The report heads itself with the period's full span, which is what a bill states, rather
    /// than with the closing date it is named by.
    #[test]
    fn the_reports_head_themselves_with_the_period() {
        let period = "Period       2026-05-24 - 2026-06-23  (31 days)";
        let energy = energy(period_ending_date(), &sessions())
            .unwrap()
            .to_string();
        assert!(energy.starts_with("EV Energy\n=========\n"), "{energy}");
        assert!(energy.contains(period), "{energy}");

        let cost = cost().to_string();
        assert!(cost.starts_with("EV Energy Cost\n"), "{cost}");
        assert!(cost.contains(period), "{cost}");
    }

    /// Rates are labelled as Toronto Hydro's. [`CostRecoveryRates`] is what the EV owners are
    /// charged, and a report that said only "rate" would leave the two indistinguishable.
    #[test]
    fn the_cost_report_says_whose_rates_it_is_pricing_at() {
        let text = cost().to_string();
        assert!(text.contains("TH blended rate"), "{text}");
        // The loss factor is stated, because it is what separates the two kWh columns.
        assert!(text.contains("Loss factor  1.0500"), "{text}");
        // The rebate is shown as the credit it is, not as a positive to be subtracted by the
        // reader. Read off the line rather than matched against a figure, so the test says what it
        // is checking and not merely that the arithmetic has not moved.
        let rebate = text
            .lines()
            .find(|l| l.contains("Ontario Electricity Rebate"))
            .expect("the totals table lists the rebate");
        assert!(rebate.contains("-0."), "{rebate}");
    }

    /// Every figure is a proportion of a bill line, so an off-cycle bill -- one whose meter reading
    /// period does not close a billing period -- is refused rather than priced.
    #[test]
    fn an_off_cycle_bill_is_refused() {
        let mut off_cycle = bill();
        off_cycle.meter_reading_period_to = date(2026, 6, 30);

        let err = energy_cost(&off_cycle, &sessions())
            .expect_err("30 June does not close a billing period");
        assert!(
            matches!(err, EnergyError::NotABillingPeriodEnding(_)),
            "{err}"
        );
    }
}
