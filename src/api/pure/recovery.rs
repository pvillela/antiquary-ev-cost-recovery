//! What the EV drivers are charged for a billing period, at the rates we set.
//!
//! The other side of the ledger from [`energy_cost`](super::energy::energy_cost) and
//! [`peak_power_cost`](super::peak_power::peak_power_cost), which say what the chargers cost against
//! the bill. This says what is recovered from the people who drew that energy, and it is not derived
//! from the bill at all: the rates are ours, and setting them is a decision rather than a
//! calculation.
//!
//! So there is no loss factor here and no tax. A cost-recovery rate is charged on the kilowatt-hours
//! the chargers metered, which is what a driver can check against their own session history; what
//! the rate has to cover is a question for whoever sets it.

use crate::hydro_bill::{
    BILL_END_DAY, BillingPeriod, NotABillingPeriodEnding, billing_period_dates, billing_period_span,
};
use crate::markdown::{Left, Right, amounts, field, h1, h2, table, wrap};
use crate::session::tou_kwh;
use crate::time::{Interval, local_midnight};
use jiff::{Timestamp, civil::Date};
use std::{error::Error, fmt};

// Through `super`, not through `crate::io`. The two cost breakdowns are computed here in `pure`;
// reaching them by the path `io` re-exports them under would point this half of the API at the
// other, which is the one direction the split exists to prevent.
use super::energy::{EnergyCost, EnergyError, countable, energy_cost};
use super::peak_power::{DeliveryCost, PeakPowerError, peak_power_cost};

// Re-exported because the functions here take these and return those, and a caller should not have
// to know which module they come from in order to spell the call.
pub use crate::green_button::PeriodValues;
pub use crate::hydro_bill::HydroBill;
pub use crate::session::{RSession, TouKwh};

/// EV cost-recovery TOU rates. The rates are effective for at least one month.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CostRecoveryRates {
    /// Effective date of the rates. Normally, the first day of a month.
    pub effective_date: Date,
    /// On-peak EV cost-recovery rate.
    pub on_peak: f64,
    /// Mid-peak EV cost-recovery rate.
    pub mid_peak: f64,
    /// Off-peak EV cost-recovery rate.
    pub off_peak: f64,
}

/// One stretch of a billing period over which a single schedule of rates was in effect: the dates
/// it spans, the rates charged on it, the energy drawn within it, and what that recovers.
///
/// Self-contained, and the level at which every figure here has one rate behind it. The energy is
/// this stretch's own — only the sessions falling inside its dates, split by time-of-use band — and
/// each band's recovery is that band's kilowatt-hours times that band's rate from `rates`. Nothing
/// has to be looked up elsewhere to check a row of the report against it.
///
/// A billing period does not begin on the first of a month, so a rate change in the middle of one
/// leaves two of these rather than two months. The whole-period figures are on
/// [`CostRecovery`], which holds these.
#[derive(Debug, Clone, PartialEq)]
pub struct CostRecoveryStretch {
    /// The rates charged over this stretch.
    pub rates: CostRecoveryRates,
    /// First calendar date the rates were charged on within the billing period.
    ///
    /// Not [`CostRecoveryRates::effective_date`], which is when the rates began and may be well
    /// before the period. This is where they began to apply *here*.
    pub from: Date,
    /// Last calendar date the rates were charged on within the billing period, inclusive.
    pub to: Date,
    /// EV energy drawn over this stretch, split by time-of-use band.
    pub kwh: TouKwh,
    /// `kwh.on_peak * rates.on_peak`.
    pub on_peak_recovery: f64,
    /// `kwh.mid_peak * rates.mid_peak`.
    pub mid_peak_recovery: f64,
    /// `kwh.off_peak * rates.off_peak`.
    pub off_peak_recovery: f64,
}

impl CostRecoveryStretch {
    /// What this stretch recovers in total.
    pub fn recovery(&self) -> f64 {
        self.on_peak_recovery + self.mid_peak_recovery + self.off_peak_recovery
    }
}

/// Cost recovery allocated to a billing period.
#[derive(Debug, Clone, PartialEq)]
pub struct CostRecovery {
    /// The billing period these figures are for, named by the date it closes on.
    pub billing_period_ending: Date,

    /// The period broken into stretches, one per schedule of rates in effect, in date order.
    ///
    /// Not a list of rate schedules. Each entry carries its own dates, its own energy split by
    /// time-of-use band, and its own recovery per band — see [`CostRecoveryStretch`]. This is where
    /// the per-band detail lives, and the whole of what the report's tables are drawn from.
    ///
    /// One entry when the rates held all period, two when they changed during it. A `Vec` rather
    /// than a pair, because the stretches are what the report lists and what the totals below are
    /// summed from, and both read the same whichever it is.
    pub stretches: Vec<CostRecoveryStretch>,

    /// EV energy over the whole period, split by time-of-use band.
    ///
    /// The sum of the stretches above. The stretches partition the period exactly — each session's
    /// energy is cut at the rate change as it is cut at the period's own boundaries — so no
    /// kilowatt-hour is counted twice and none is lost between them.
    pub kwh: TouKwh,

    /// Total cost recovery allocated to the billing period.
    pub cost_recovery: f64,
}

/// The EV cost recovery for the billing period, the hydro delivery and energy costs attributable
/// to EV charging sessions during the period, and their net financial impact.
///
/// All three are for the same billing period, which is the bill's: it is what
/// [`cost_recovery_surplus`] takes a period from, and all three parts are built against it. There
/// is no separate `billing_period_ending` field because each part already carries one, and a fourth
/// copy could only agree with them.
pub struct CostRecoverySurplus {
    /// What the drivers are charged, at our own rates.
    pub recovery: CostRecovery,
    /// What the chargers' share of the three demand-priced delivery lines cost, after HST and the
    /// rebate.
    pub delivery: DeliveryCost,
    /// What the chargers' share of the three time-of-use consumption lines cost, after HST and the
    /// rebate.
    pub energy: EnergyCost,
    /// `recovery.cost_recovery - delivery.delivery_cost - energy.energy_cost`, each term rounded to
    /// the cent before the subtraction, and the result rounded again.
    ///
    /// Positive when the rates over-recover and negative when they fall short, so the sign is the
    /// answer the figure exists to give.
    ///
    /// Rounded, unlike every other figure in this module, because this one is an accounting total
    /// and the column above it has to add down. Subtracting the unrestrained values gave a surplus
    /// a cent away from what the three printed amounts come to, which in a report whose whole point
    /// is a subtraction reads as an arithmetic error. The three parts keep their own unrounded
    /// totals; only the difference taken here is to the cent.
    pub surplus: f64,
}

// No per-band recovery for the whole period. A band's kilowatt-hours were charged at one rate in
// each stretch and at a different rate in the next, so their sum is money recovered under two
// schedules at once -- a figure no table here shows and no invoice would state. The bands are
// reported per stretch, on [`CostRecoveryStretch`], where each has a single rate behind it.
//
// [`Self::kwh`] and [`Self::cost_recovery`] are summable in the same way and are kept, because
// both are figures the report itself states.

/// Why a billing period's sessions cannot be turned into a cost recovery.
///
/// No variant names a file. The rates are given as values and the period as a date, so nothing here
/// has a file to be about — which is why [`ApiError`](crate::error::ApiError) carries this one
/// without a `source`, unlike the errors of the two costing operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CostRecoveryError {
    /// The date given does not close a billing period: it is not [`BILL_END_DAY`] of its month.
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// The rates given as the period's opening rates take effect after the period starts, so the
    /// first days of it would be charged at no rate at all.
    ///
    /// Almost always one month's rates handed in for the period that straddles the month before.
    /// Refused rather than backdated: the rates that were actually in effect on those days exist,
    /// and inventing coverage for them would under-recover silently.
    RatesNotYetInEffect {
        period_start: Date,
        effective_date: Date,
    },

    /// The second schedule of rates does not take effect during the period.
    ///
    /// A change dated on or before the period's first day leaves the opening rates covering
    /// nothing, and one dated after its last day belongs to the next period. Either way the caller
    /// has named a change this period does not contain, and passing a single schedule is what they
    /// meant.
    RateChangeOutsidePeriod {
        period_start: Date,
        period_ending: Date,
        effective_date: Date,
    },
}

/// Why a billing period does not yield a cost-recovery surplus.
///
/// One variant per part the answer is built from, each carrying that part's own error. A surplus is
/// three calculations subtracted from each other, and any of the three can refuse; widening
/// [`CostRecoveryError`] instead would have given [`cost_recovery`] two variants it cannot raise.
///
/// No variant names a file, for the same reason [`CostRecoveryError`] names none.
#[derive(Debug, Clone, PartialEq)]
pub enum CostRecoverySurplusError {
    /// The recovery side refused. See [`CostRecoveryError`].
    Recovery(CostRecoveryError),
    /// The delivery cost refused. See [`PeakPowerError`].
    PeakPower(PeakPowerError),
    /// The energy cost refused. See [`EnergyError`].
    Energy(EnergyError),
}

impl From<CostRecoveryError> for CostRecoverySurplusError {
    fn from(e: CostRecoveryError) -> Self {
        Self::Recovery(e)
    }
}

impl From<PeakPowerError> for CostRecoverySurplusError {
    fn from(e: PeakPowerError) -> Self {
        Self::PeakPower(e)
    }
}

impl From<EnergyError> for CostRecoverySurplusError {
    fn from(e: EnergyError) -> Self {
        Self::Energy(e)
    }
}

impl fmt::Display for CostRecoverySurplusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recovery(e) => e.fmt(f),
            Self::PeakPower(e) => e.fmt(f),
            Self::Energy(e) => e.fmt(f),
        }
    }
}

impl Error for CostRecoverySurplusError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Recovery(e) => Some(e),
            Self::PeakPower(e) => Some(e),
            Self::Energy(e) => Some(e),
        }
    }
}

impl From<NotABillingPeriodEnding> for CostRecoveryError {
    fn from(e: NotABillingPeriodEnding) -> Self {
        Self::NotABillingPeriodEnding(e)
    }
}

impl fmt::Display for CostRecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotABillingPeriodEnding(e) => e.fmt(f),
            Self::RatesNotYetInEffect {
                period_start,
                effective_date,
            } => write!(
                f,
                "the cost-recovery rates given for the start of the period take effect \
                 {effective_date}, after it starts on {period_start}"
            ),
            Self::RateChangeOutsidePeriod {
                period_start,
                period_ending,
                effective_date,
            } => write!(
                f,
                "the second set of cost-recovery rates takes effect {effective_date}, which is not \
                 within the billing period {period_start} to {period_ending}"
            ),
        }
    }
}

impl Error for CostRecoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotABillingPeriodEnding(e) => Some(e),
            _ => None,
        }
    }
}

/// Returns the cost recovery allocated to the billing period. Applies the specified EV
/// cost-recovery TOU rates to the corresponding TOU energy use by EV charging sessions.
/// If the cost-recovery rates change during the billing period, a second set of cost-recovery
/// rates is specified.
///
/// # How the figure is arrived at
///
/// The EV kilowatt-hours in each of the three time-of-use bands are taken as
/// [`energy`](super::energy::energy) takes them, and multiplied by the rate for that band. Nothing
/// else: no loss factor, because the rate is charged on what the chargers metered rather than on
/// what the utility adjusted; and no HST or rebate, because those are the utility's and this is not
/// a utility bill.
///
/// A change of rates during the period splits it in two at prevailing local midnight starting
/// `recovery_rates_at_end.effective_date`, and each session's energy is cut at that instant the way
/// it is cut at the period's own boundaries. Local midnight rather than the standard-time midnight
/// the *period* turns on: the period boundary is on standard time because Toronto Hydro's is, while
/// when our own rates change is our own decision, and the date it is announced on means the day
/// people live in.
///
/// # Arguments
///
/// - `billing_period_ending` - the billing period, named by the date it closes on. Must be
///   [`BILL_END_DAY`] of its month.
/// - `sessions` - every session from every report covering the period, as
///   [`energy`](super::energy::energy) takes them, with the same obligation to supply all of them
///   and the same treatment of duplicates and of records that contradict themselves.
/// - `recovery_rates_at_start` - the rates in effect on the period's first day. Their
///   `effective_date` may be well before the period.
/// - `recovery_rates_at_end` - the rates the period changed to, or `None` if it did not.
///
/// # Errors
///
/// [`CostRecoveryError::NotABillingPeriodEnding`] if `billing_period_ending` is not
/// [`BILL_END_DAY`] of its month; [`CostRecoveryError::RatesNotYetInEffect`] if the opening rates
/// do not reach the period's first day; and [`CostRecoveryError::RateChangeOutsidePeriod`] if the
/// second schedule takes effect outside it.
pub fn cost_recovery(
    billing_period_ending: Date,
    sessions: &[RSession],
    recovery_rates_at_start: CostRecoveryRates,
    recovery_rates_at_end: Option<CostRecoveryRates>,
) -> Result<CostRecovery, CostRecoveryError> {
    let (period_start, period_ending) = billing_period_dates(billing_period_ending)?;

    if recovery_rates_at_start.effective_date > period_start {
        return Err(CostRecoveryError::RatesNotYetInEffect {
            period_start,
            effective_date: recovery_rates_at_start.effective_date,
        });
    }
    if let Some(rates) = &recovery_rates_at_end
        && !(period_start < rates.effective_date && rates.effective_date <= period_ending)
    {
        return Err(CostRecoveryError::RateChangeOutsidePeriod {
            period_start,
            period_ending,
            effective_date: rates.effective_date,
        });
    }

    let period = BillingPeriod::ending_on(billing_period_ending, BILL_END_DAY);
    let counted = countable(sessions);

    // The instant the rates change, and with it the two stretches. Checked above to fall strictly
    // inside the period, so neither stretch is empty and the two partition it exactly.
    let stretches = match recovery_rates_at_end {
        None => vec![stretch(
            recovery_rates_at_start,
            period_start,
            period_ending,
            period.start,
            period.end,
            &counted,
        )],
        Some(rates_at_end) => {
            let change = local_midnight(rates_at_end.effective_date);
            let last_of_first = rates_at_end
                .effective_date
                .yesterday()
                .expect("a date inside a billing period has a yesterday");
            vec![
                stretch(
                    recovery_rates_at_start,
                    period_start,
                    last_of_first,
                    period.start,
                    change,
                    &counted,
                ),
                stretch(
                    rates_at_end,
                    rates_at_end.effective_date,
                    period_ending,
                    change,
                    period.end,
                    &counted,
                ),
            ]
        }
    };

    let sum = |f: fn(&CostRecoveryStretch) -> f64| stretches.iter().map(f).sum::<f64>();

    Ok(CostRecovery {
        billing_period_ending,
        kwh: TouKwh {
            on_peak: sum(|a| a.kwh.on_peak),
            mid_peak: sum(|a| a.kwh.mid_peak),
            off_peak: sum(|a| a.kwh.off_peak),
        },
        cost_recovery: sum(CostRecoveryStretch::recovery),
        stretches,
    })
}

/// Returns the EV cost-recovery surplus for the billing period, i.e., the EV cost recovery for the
/// billing period minus the hydro delivery and energy costs attributable to EV charging sessions
/// during the period.
///
/// # How the figure is arrived at
///
/// The three parts are [`cost_recovery`], [`peak_power_cost`](super::peak_power::peak_power_cost)
/// and [`energy_cost`](super::energy::energy_cost), each computed exactly as it is on its own and
/// each returned whole. Nothing is recomputed here and no figure is adjusted to make the three
/// agree: this function subtracts, and the parts are kept so that the subtraction can be checked.
///
/// The two costs are net of HST and the Ontario Electricity Rebate, because that is what the
/// chargers actually cost. The recovery carries neither, because our rates are whatever we set them
/// to. Subtracting one from the other is therefore money against money, which is the only basis on
/// which the two sides compare.
///
/// The costs cover the two parts of the bill a charger can be held to. The customer charge, the
/// standard supply administration charge and the wholesale market service charge are none of them
/// in either, so a surplus of zero is not the same as breaking even on the whole invoice.
///
/// # Arguments
///
/// - `bill` - the Toronto Hydro bill for the period, which supplies the period and every rate the
///   two costs use.
/// - `gb_period_values` - the meter export's figures for the period, as
///   [`peak_power_cost`](super::peak_power::peak_power_cost) takes them.
/// - `sessions` - every session from every report covering the period, as
///   [`energy`](super::energy::energy) takes them.
/// - `recovery_rates_at_start` - the rates in effect on the period's first day.
/// - `recovery_rates_at_end` - the rates the period changed to, or `None` if it did not.
///
/// There is no `billing_period_ending` argument, for the reason the two costing functions have
/// none: the bill states which period it covers, and one passed alongside could only agree with it
/// or contradict it. Here that matters more than it does to either alone, since a date disagreeing
/// with the bill would subtract two periods' figures from each other.
///
/// # Errors
///
/// [`CostRecoverySurplusError`], carrying whichever of the three parts refused. The recovery is
/// computed first, so rates that do not fit the period are reported before the meter export is
/// examined.
pub fn cost_recovery_surplus(
    bill: &HydroBill,
    gb_period_values: PeriodValues,
    sessions: &[RSession],
    recovery_rates_at_start: CostRecoveryRates,
    recovery_rates_at_end: Option<CostRecoveryRates>,
) -> Result<CostRecoverySurplus, CostRecoverySurplusError> {
    // The bill is the single source of the period, so all three parts are for the same one by
    // construction rather than by a check. An off-cycle bill is refused by each of them in turn;
    // the recovery is called first, so that is where it surfaces.
    let billing_period_ending = bill.period_end_date();

    let recovery = cost_recovery(
        billing_period_ending,
        sessions,
        recovery_rates_at_start,
        recovery_rates_at_end,
    )?;
    let delivery = peak_power_cost(bill, gb_period_values, sessions)?;
    let energy = energy_cost(bill, sessions)?;

    Ok(CostRecoverySurplus {
        surplus: to_the_cent(
            to_the_cent(recovery.cost_recovery)
                - to_the_cent(delivery.delivery_cost)
                - to_the_cent(energy.energy_cost),
        ),
        recovery,
        delivery,
        energy,
    })
}

/// An amount rounded to the cent, as the reports state it.
///
/// Through the formatter rather than by arithmetic on the value. `(x * 100.0).round() / 100.0`
/// rounds a half away from zero while `{:.2}` rounds it to even, so the two disagree on an amount
/// landing exactly on half a cent -- and a surplus that disagreed with its own column in that case
/// would be the one defect this rounding exists to prevent. The round trip through a string is what
/// makes the result the printed figure by construction rather than by an argument that the two
/// rules coincide.
fn to_the_cent(amount: f64) -> f64 {
    format!("{amount:.2}")
        .parse()
        .expect("a decimal written by this formatter parses back")
}

/// One stretch of the period priced at one schedule of rates.
///
/// The dates and the instants are given separately because they are not the same cut. `from` and
/// `to` are what a reader checks against a calendar; `start` and `end` are where the energy is
/// actually divided, and the period's own ends sit at standard-time midnight rather than on a date.
fn stretch(
    rates: CostRecoveryRates,
    from: Date,
    to: Date,
    start: Timestamp,
    end: Timestamp,
    counted: &[RSession],
) -> CostRecoveryStretch {
    let kwh = tou_kwh(Interval::from_start_end(start, end), counted);
    CostRecoveryStretch {
        on_peak_recovery: kwh.on_peak * rates.on_peak,
        mid_peak_recovery: kwh.mid_peak * rates.mid_peak,
        off_peak_recovery: kwh.off_peak * rates.off_peak,
        rates,
        from,
        to,
        kwh,
    }
}

/// The four columns one time-of-use band occupies in a recovery table.
fn band_row(name: &str, kwh: f64, rate: f64, recovery: f64) -> Vec<String> {
    vec![
        name.to_owned(),
        format!("{kwh:.3}"),
        format!("{rate:.5}"),
        format!("{recovery:.2}"),
    ]
}

/// The table one stretch of the period is shown as, bands then total.
///
/// The total row leaves the rate cell empty rather than averaging the three: a weighted mean of
/// rates is not a rate anybody was charged, and the column exists to be checked against the
/// schedule that was published.
fn stretch_table(s: &CostRecoveryStretch) -> String {
    let rows = vec![
        band_row(
            "On-peak",
            s.kwh.on_peak,
            s.rates.on_peak,
            s.on_peak_recovery,
        ),
        band_row(
            "Mid-peak",
            s.kwh.mid_peak,
            s.rates.mid_peak,
            s.mid_peak_recovery,
        ),
        band_row(
            "Off-peak",
            s.kwh.off_peak,
            s.rates.off_peak,
            s.off_peak_recovery,
        ),
        vec![
            "Total".to_owned(),
            format!("{:.3}", s.kwh.total_kwh()),
            String::new(),
            format!("{:.2}", s.recovery()),
        ],
    ];
    table(
        &["TOU", "kWh", "EV rate", "Recovery"],
        &rows,
        &[Left, Right, Right, Right],
    )
}

impl fmt::Display for CostRecovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}\n", h1("EV Cost Recovery"))?;
        writeln!(
            f,
            "{}",
            field("Period", &billing_period_span(self.billing_period_ending))
        )?;

        // One schedule is the ordinary case, and it gets one table under a heading line rather than
        // a section of its own followed by a total that repeats it.
        if let [only] = &self.stretches[..] {
            writeln!(
                f,
                "{}\n",
                field(
                    "EV rates",
                    &format!("effective {}", only.rates.effective_date)
                )
            )?;
            return writeln!(f, "{}", stretch_table(only));
        };

        writeln!(f)?;
        for s in &self.stretches {
            writeln!(
                f,
                "{}\n",
                h2(&format!(
                    "EV rates effective {}  ({} - {})",
                    s.rates.effective_date, s.from, s.to
                ))
            )?;
            writeln!(f, "{}\n", stretch_table(s))?;
        }

        writeln!(f, "{}\n", h2("EV Cost Recovery Total"))?;
        let mut rows: Vec<(String, f64)> = self
            .stretches
            .iter()
            .map(|s| {
                (
                    format!("At rates effective {}", s.rates.effective_date),
                    s.recovery(),
                )
            })
            .collect();
        rows.push(("Cost recovery".to_owned(), self.cost_recovery));
        let rows: Vec<(&str, f64)> = rows.iter().map(|(l, a)| (l.as_str(), *a)).collect();
        writeln!(f, "{}", amounts(&rows))
    }
}

impl fmt::Display for CostRecoverySurplus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}\n", h1("EV Cost Recovery Surplus"))?;
        writeln!(
            f,
            "{}\n",
            field(
                "Period",
                &billing_period_span(self.recovery.billing_period_ending)
            )
        )?;

        // The costs are shown negative so the column adds down to the surplus. A reader checking a
        // figure that is meant to be a subtraction cannot do it against three positive numbers.
        writeln!(
            f,
            "{}\n",
            amounts(&[
                ("Cost recovery", self.recovery.cost_recovery),
                ("EV energy cost", -self.energy.energy_cost),
                ("EV delivery cost", -self.delivery.delivery_cost),
                ("Surplus", self.surplus),
            ])
        )?;
        writeln!(
            f,
            "{}\n",
            wrap(VERDICT[usize::from(self.surplus < 0.0)], "")
        )?;

        // The three parts in full, under their own headings. The figure above is a subtraction of
        // three numbers, and none of them can be checked without the report it came from.
        writeln!(f, "{}", self.recovery)?;
        writeln!(f, "{}", self.energy)?;
        write!(f, "{}", self.delivery)
    }
}

/// What the sign of the surplus means, indexed by `surplus < 0.0`.
///
/// Spelled out because the sign alone is read the wrong way about as often as the right way: money
/// coming in is positive here, so a shortfall is the negative number.
const VERDICT: [&str; 2] = [
    "The cost-recovery rates covered the chargers' share of the bill for this period, with the \
     surplus above left over.",
    "The cost-recovery rates fell short of the chargers' share of the bill for this period, by the \
     amount above.",
];

// cargo test --lib -- api::pure::recovery::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use crate::api::pure::test_support::{
        KVA_PEAK_HOUR, KW_PEAK_HOUR, NOP_PEAK_HOUR, bill, period_ending_date,
        period_values_with_nop, two_reports,
    };
    use crate::session::test_support::session;
    use jiff::civil::date;

    /// The period every fixture here belongs to: 24 May to 23 June 2026.
    fn ending() -> Date {
        period_ending_date()
    }

    /// The meter figures the surplus fixture uses: all three maxima stated, as a real export has
    /// them.
    fn peaks() -> PeriodValues {
        period_values_with_nop(Some(KW_PEAK_HOUR), Some(KVA_PEAK_HOUR), Some(NOP_PEAK_HOUR))
    }

    fn rates(effective: Date, on_peak: f64, mid_peak: f64, off_peak: f64) -> CostRecoveryRates {
        CostRecoveryRates {
            effective_date: effective,
            on_peak,
            mid_peak,
            off_peak,
        }
    }

    /// Flat rates, so a recovery is the energy times one number and can be checked by hand.
    fn flat(effective: Date, rate: f64) -> CostRecoveryRates {
        rates(effective, rate, rate, rate)
    }

    /// The ordinary case: one schedule, one stretch, and the recovery is the rate times what was
    /// drawn.
    #[test]
    fn one_schedule_prices_the_whole_period() {
        // 02:00 EDT on 10 June, an hour long, wholly inside the period.
        let s = session("June.csv", 2, "IN", "2026-06-10T06:00:00Z", 60, 7.0);

        let r = cost_recovery(ending(), &[s], flat(date(2026, 5, 1), 0.10), None)
            .expect("23 June closes a billing period");

        assert_eq!(r.stretches.len(), 1);
        assert_eq!(r.stretches[0].from, date(2026, 5, 24));
        assert_eq!(r.stretches[0].to, date(2026, 6, 23));
        assert!((r.kwh.total_kwh() - 7.0).abs() < 1e-9, "{:?}", r.kwh);
        assert!((r.cost_recovery - 0.70).abs() < 1e-9, "{}", r.cost_recovery);
    }

    /// The property the two-schedule case rests on: the stretches partition the period, so their
    /// energy sums to what one schedule over the whole period sees. Not approximately -- each
    /// session is cut at the change the way it is cut at the period's ends, so nothing is counted
    /// twice and nothing falls between.
    #[test]
    fn a_rate_change_splits_the_energy_without_losing_any() {
        let sessions = [
            // Before the change, and before the period's own start: only its tail counts.
            session("May.csv", 2, "EARLY", "2026-05-23T22:00:00Z", 240, 8.0),
            // Squarely inside the first stretch.
            session("May.csv", 3, "MAY", "2026-05-28T06:00:00Z", 60, 7.0),
            // Straddling local midnight on 1 June, so it lands in both stretches.
            session("June.csv", 2, "ACROSS", "2026-06-01T02:00:00Z", 240, 12.0),
            // Squarely inside the second stretch.
            session("June.csv", 3, "JUNE", "2026-06-10T06:00:00Z", 60, 7.0),
        ];

        let whole = cost_recovery(ending(), &sessions, flat(date(2026, 5, 1), 0.10), None)
            .expect("23 June closes a billing period");
        let split = cost_recovery(
            ending(),
            &sessions,
            flat(date(2026, 5, 1), 0.10),
            Some(flat(date(2026, 6, 1), 0.10)),
        )
        .expect("1 June falls inside the period");

        assert_eq!(split.stretches.len(), 2);
        // Both stretches see energy, so the sum below is a real test rather than one of them being
        // the whole and the other zero.
        for s in &split.stretches {
            assert!(s.kwh.total_kwh() > 0.0, "{s:?}");
        }
        for band in [
            |k: &TouKwh| k.on_peak,
            |k: &TouKwh| k.mid_peak,
            |k: &TouKwh| k.off_peak,
        ] {
            let parts: f64 = split.stretches.iter().map(|a| band(&a.kwh)).sum();
            assert!(
                (parts - band(&whole.kwh)).abs() < 1e-9,
                "{parts} vs {}",
                band(&whole.kwh)
            );
        }
        // At one flat rate throughout, splitting the period cannot change what it recovers.
        assert!(
            (split.cost_recovery - whole.cost_recovery).abs() < 1e-9,
            "{} vs {}",
            split.cost_recovery,
            whole.cost_recovery
        );
    }

    /// The stretches are dated by where the rates applied *here*, not by when they took effect: the
    /// first runs from the period's own start, not from 1 May.
    #[test]
    fn the_stretches_are_dated_by_the_period_not_by_the_effective_dates() {
        let r = cost_recovery(
            ending(),
            &[],
            flat(date(2026, 5, 1), 0.10),
            Some(flat(date(2026, 6, 1), 0.12)),
        )
        .expect("1 June falls inside the period");

        let [first, second] = &r.stretches[..] else {
            panic!(
                "two schedules give two stretches, got {}",
                r.stretches.len()
            );
        };
        assert_eq!(
            (first.from, first.to),
            (date(2026, 5, 24), date(2026, 5, 31))
        );
        assert_eq!(
            (second.from, second.to),
            (date(2026, 6, 1), date(2026, 6, 23))
        );
    }

    /// Each band is priced at its own rate, so a report cannot silently charge one band's rate on
    /// another's kilowatt-hours.
    #[test]
    fn each_band_is_priced_at_its_own_rate() {
        // 02:00 EDT on a June Wednesday: off-peak, which runs to 07:00.
        let off = session("June.csv", 2, "OFF", "2026-06-10T06:00:00Z", 60, 10.0);
        let r = cost_recovery(
            ending(),
            &[off],
            rates(date(2026, 5, 1), 1.0, 2.0, 3.0),
            None,
        )
        .expect("23 June closes a billing period");

        assert!((r.kwh.off_peak - 10.0).abs() < 1e-9, "{:?}", r.kwh);
        // Per stretch, which is the only level a band has one rate behind it.
        let s = &r.stretches[0];
        assert_eq!(s.on_peak_recovery, 0.0);
        assert_eq!(s.mid_peak_recovery, 0.0);
        assert!((s.off_peak_recovery - 30.0).abs() < 1e-9, "{s:?}");
        assert!((r.cost_recovery - 30.0).abs() < 1e-9, "{}", r.cost_recovery);
    }

    /// A date that closes no billing period is the caller's mistake, and is reported as such rather
    /// than reaching the panic in `BillingPeriod::ending_on`.
    #[test]
    fn a_date_that_does_not_close_a_billing_period_is_refused() {
        let err = cost_recovery(date(2026, 6, 30), &[], flat(date(2026, 5, 1), 0.10), None)
            .expect_err("30 June does not label a billing period");
        assert!(
            matches!(err, CostRecoveryError::NotABillingPeriodEnding(_)),
            "{err}"
        );
    }

    /// Opening rates that begin after the period does would leave its first days charged at no rate
    /// at all. Refused rather than backdated.
    #[test]
    fn opening_rates_must_reach_the_periods_first_day() {
        let err = cost_recovery(ending(), &[], flat(date(2026, 6, 1), 0.10), None)
            .expect_err("1 June is after the period starts on 24 May");
        assert!(
            matches!(
                err,
                CostRecoveryError::RatesNotYetInEffect {
                    period_start,
                    effective_date,
                } if period_start == date(2026, 5, 24) && effective_date == date(2026, 6, 1)
            ),
            "{err}"
        );

        // Rates already in effect when the period opens are the ordinary case, and the common one:
        // a period starting on the 24th is nearly always charged at rates set earlier that month.
        assert!(cost_recovery(ending(), &[], flat(date(2026, 5, 1), 0.10), None).is_ok());
        // In effect exactly on the first day is inside, not outside.
        assert!(cost_recovery(ending(), &[], flat(date(2026, 5, 24), 0.10), None).is_ok());
    }

    /// A change dated outside the period names a split this period does not contain. Both ends are
    /// checked, and the period's own boundaries are the ones that count -- not the month's.
    #[test]
    fn a_rate_change_must_fall_inside_the_period() {
        let start = flat(date(2026, 5, 1), 0.10);
        let outside = |change: Date| {
            cost_recovery(ending(), &[], start, Some(flat(change, 0.12)))
                .expect_err("{change} is outside the period")
        };

        // On the first day, which would leave the opening rates covering nothing.
        assert!(
            matches!(
                outside(date(2026, 5, 24)),
                CostRecoveryError::RateChangeOutsidePeriod { .. }
            ),
            "a change on the period's first day"
        );
        // Before it, and after its last day.
        for change in [date(2026, 5, 1), date(2026, 6, 24), date(2026, 7, 1)] {
            assert!(
                matches!(
                    outside(change),
                    CostRecoveryError::RateChangeOutsidePeriod { .. }
                ),
                "a change on {change}"
            );
        }

        // The two days just inside each end are accepted, which is what fixes the boundary.
        for change in [date(2026, 5, 25), date(2026, 6, 23)] {
            assert!(
                cost_recovery(ending(), &[], start, Some(flat(change, 0.12))).is_ok(),
                "a change on {change}"
            );
        }
    }

    /// One schedule and two are laid out differently -- one table against a section each plus a
    /// total -- so both are rendered here. Neither may claim a figure the other does not.
    #[test]
    fn the_report_states_the_rates_it_charged() {
        let s = session("June.csv", 2, "IN", "2026-06-10T06:00:00Z", 60, 10.0);

        let one = cost_recovery(
            ending(),
            std::slice::from_ref(&s),
            flat(date(2026, 5, 1), 0.10),
            None,
        )
        .expect("23 June closes a billing period")
        .to_string();
        assert!(one.contains("EV Cost Recovery"), "{one}");
        assert!(one.contains("0.10000"), "{one}");
        assert!(one.contains("effective 2026-05-01"), "{one}");
        // With one schedule there is no per-stretch section and no total section repeating it.
        assert!(!one.contains("EV Cost Recovery Total"), "{one}");

        let two = cost_recovery(
            ending(),
            &[s],
            flat(date(2026, 5, 1), 0.10),
            Some(flat(date(2026, 6, 1), 0.12)),
        )
        .expect("1 June falls inside the period")
        .to_string();
        assert!(two.contains("EV rates effective 2026-05-01"), "{two}");
        assert!(two.contains("EV rates effective 2026-06-01"), "{two}");
        assert!(two.contains("EV Cost Recovery Total"), "{two}");
        assert!(two.contains("| Cost recovery"), "{two}");
    }

    /// The surplus is a subtraction and nothing else: each part equals what its own function
    /// returns for the same inputs, and the figure is those three combined.
    #[test]
    fn the_surplus_is_the_recovery_less_the_two_costs() {
        let rates = flat(date(2026, 5, 1), 0.10);
        let s = cost_recovery_surplus(&bill(), peaks(), &two_reports(), rates, None)
            .expect("the fixture bill closes a billing period and has all three maxima");

        // Recomputed independently, so a surplus built from anything other than these three parts
        // fails here rather than in the arithmetic below.
        let recovery = cost_recovery(ending(), &two_reports(), rates, None).expect("the recovery");
        let delivery =
            peak_power_cost(&bill(), peaks(), &two_reports()).expect("the delivery cost");
        let energy = energy_cost(&bill(), &two_reports()).expect("the energy cost");

        assert_eq!(s.recovery.cost_recovery, recovery.cost_recovery);
        assert_eq!(s.delivery.delivery_cost, delivery.delivery_cost);
        assert_eq!(s.energy.energy_cost, energy.energy_cost);
        // To the cent, since that is what the surplus is; the parts keep their unrounded totals.
        assert_eq!(
            s.surplus,
            to_the_cent(
                to_the_cent(recovery.cost_recovery)
                    - to_the_cent(delivery.delivery_cost)
                    - to_the_cent(energy.energy_cost)
            )
        );
    }

    /// The contract the summary is read under: the four printed amounts, put into a calculator,
    /// give the printed surplus.
    ///
    /// Read back off the rendered report rather than off the struct, because those four numbers are
    /// all a reader checking the bill has.
    ///
    /// Swept across many rate schedules rather than asserted once. Whether the rounding of three
    /// amounts happens to agree with the rounding of their difference depends on where each falls
    /// against a half cent, so a single fixture proves nothing about the next bill.
    #[test]
    fn the_printed_amounts_add_up_to_the_printed_surplus() {
        let amount = |report: &str, label: &str| -> f64 {
            let row = report
                .lines()
                .find(|l| l.starts_with(&format!("| {label}")))
                .unwrap_or_else(|| panic!("no {label} row in\n{report}"));
            row.rsplit('|')
                .nth(1)
                .expect("an amount cell")
                .trim()
                .parse()
                .expect("an amount")
        };

        // Rates in tenth-of-a-cent steps, and three unequal bands, so the three totals land all
        // over the cent rather than tracking each other.
        for step in 0..200 {
            let base = f64::from(step) * 0.001;
            let rates = rates(date(2026, 5, 1), base, base + 0.0007, base + 0.0003);
            let report = cost_recovery_surplus(&bill(), peaks(), &two_reports(), rates, None)
                .expect("the fixture bill closes a billing period")
                .to_string();

            let recovery = amount(&report, "Cost recovery");
            let energy = amount(&report, "EV energy cost");
            let delivery = amount(&report, "EV delivery cost");
            let surplus = amount(&report, "Surplus");

            // The costs print negative, so the column is added rather than subtracted. Compared as
            // printed strings, since that is the comparison a reader makes.
            assert_eq!(
                format!("{:.2}", recovery + energy + delivery),
                format!("{surplus:.2}"),
                "at rate {base:.4}:\n{report}"
            );
        }
    }

    /// The sign is the answer, so both directions are exercised. Rates high enough to cover the
    /// share give a positive surplus; rates of zero give a negative one of exactly the two costs.
    #[test]
    fn the_sign_says_whether_the_rates_covered_the_share() {
        let surplus_at = |rate: f64| {
            cost_recovery_surplus(
                &bill(),
                peaks(),
                &two_reports(),
                flat(date(2026, 5, 1), rate),
                None,
            )
            .expect("the fixture bill closes a billing period")
        };

        let none = surplus_at(0.0);
        assert_eq!(none.recovery.cost_recovery, 0.0);
        // The two costs to the cent, since that is what the surplus is built from.
        assert_eq!(
            none.surplus,
            to_the_cent(
                -to_the_cent(none.delivery.delivery_cost) - to_the_cent(none.energy.energy_cost)
            )
        );
        assert!(none.surplus < 0.0, "{}", none.surplus);

        // Far above any plausible rate, so the recovery outruns the share whatever the fixture's
        // costs come to.
        assert!(surplus_at(100.0).surplus > 0.0);
    }

    /// Each of the three parts can refuse, and the error says which did. The recovery is computed
    /// first, so its failure is the one reported when more than one would fire.
    #[test]
    fn a_failure_says_which_part_refused() {
        // Rates that begin after the period does: the recovery's own complaint.
        let err = cost_recovery_surplus(
            &bill(),
            peaks(),
            &two_reports(),
            flat(date(2026, 6, 1), 0.10),
            None,
        )
        .err()
        .expect("1 June is after the period starts on 24 May");
        assert!(
            matches!(
                err,
                CostRecoverySurplusError::Recovery(CostRecoveryError::RatesNotYetInEffect { .. })
            ),
            "{err}"
        );

        // A meter export with no kVA maximum: the delivery cost's.
        let no_kva = period_values_with_nop(Some(KW_PEAK_HOUR), None, Some(NOP_PEAK_HOUR));
        let err = cost_recovery_surplus(
            &bill(),
            no_kva,
            &two_reports(),
            flat(date(2026, 5, 1), 0.10),
            None,
        )
        .err()
        .expect("no kVA maximum to estimate against");
        assert!(
            matches!(
                err,
                CostRecoverySurplusError::PeakPower(PeakPowerError::NoPeak { .. })
            ),
            "{err}"
        );

        // A bill stating no on-peak consumption: the energy cost's, since it states no on-peak rate
        // to price the EV share at.
        let mut flat_band = bill();
        flat_band.on_peak_kwh = 0.0;
        let err = cost_recovery_surplus(
            &flat_band,
            peaks(),
            &two_reports(),
            flat(date(2026, 5, 1), 0.10),
            None,
        )
        .err()
        .expect("the bill states no on-peak rate");
        assert!(
            matches!(
                err,
                CostRecoverySurplusError::Energy(EnergyError::NoRate { .. })
            ),
            "{err}"
        );
    }

    /// The summary is a subtraction the reader checks by eye, so it states all four figures and
    /// says which way the sign runs. The three parts follow in full, since none of the three can be
    /// checked without the report it came from.
    #[test]
    fn the_surplus_report_carries_its_three_parts() {
        let s = cost_recovery_surplus(
            &bill(),
            peaks(),
            &two_reports(),
            flat(date(2026, 5, 1), 0.10),
            None,
        )
        .expect("the fixture bill closes a billing period")
        .to_string();

        assert!(s.starts_with("EV Cost Recovery Surplus\n"), "{s}");
        for line in [
            "| Cost recovery",
            "| EV energy cost",
            "| EV delivery cost",
            "| Surplus",
        ] {
            assert!(s.contains(line), "{line} missing from\n{s}");
        }
        // Rates of 0.10 do not cover the share, so the shortfall wording is the one shown.
        assert!(s.contains("fell short"), "{s}");

        // Each part's own report, by its heading.
        for heading in [
            "EV Cost Recovery\n=",
            "EV Energy Cost\n=",
            "EV Delivery Cost\n=",
        ] {
            assert!(s.contains(heading), "{heading:?} missing from\n{s}");
        }
    }
}
