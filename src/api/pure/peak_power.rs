//! The EV share of the two hours a billing period peaked in.

use crate::api::pure::billing_period::{NotABillingPeriodEnding, billing_period_dates};
use crate::green_button::{METER_INTERVAL, Peak};
use crate::hydro_bills::HydroBill;
use crate::sessions::{IntervalEstimates, SessionReport, estimates_from_report};
use crate::time::Interval;

// Re-exported because `peak_power` takes them. `IntervalEstimates` is deliberately not: it is
// inside `PowerEstimates` rather than named by the signature, and a reader who probes that far can
// go to `sessions` for it.
pub use crate::green_button::PeriodValues;
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
pub struct DeliveryCost {
    /// `'Distribution Charges' / 'Adj. kVA'` from bill.
    pub blended_distribution_rate: f64,
    /// `'Transmission Connection Charge' / 'Adj. kW'` from bill.
    pub blended_connection_rate: f64,
    /// `'Transmission Network Charge' / 'Adj. Peak kW 7-7'` from bill.
    pub blended_network_rate: f64,

    /// Mid-point of energy-based bracket of EV kVA from sessions
    /// for Demand kVA interval of interest.
    pub demand_kva: f64,
    /// Mid-point of energy-based bracket of EV kW from sessions
    /// for Demand kW interval of interest.
    pub demand_kw: f64,
    /// Mid-point of energy-based bracket of EV kW from sessions
    /// for Peak 7-7 kW interval of interest.
    pub peak_7_7_kw: f64,

    /// Days in billing period.
    pub days_in_period: u8,
    /// `days_in_period / 30`
    pub days_adj_factor: u8,

    /// Distribution charges attributable to EV sessions.
    pub distribution_charges: f64,
    /// Transmission Connection Charge attributable to EV sessions.
    pub connection_charge: f64,
    /// Transmission Network Charge attributable to EV sessions.
    pub network_charge: f64,

    /// HST on delivery charges attributable to EV sessions, before OER.
    pub hst: f64,
    /// Onario Electricity Rebate
    pub ontario_electricity_rebate: f64,

    /// Total delivery cost attributable to EV sessions, net of HST and OER.
    pub delivery_cost: f64,
}

/// Why a billing period's figures cannot be turned into peak power estimates.
///
/// No variant names a file. Producing this is a computation, and a computation cannot fail to read
/// something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeakPowerError {
    NotABillingPeriodEnding(NotABillingPeriodEnding),

    /// The billing period carries no reading in one of the two power series, so it has no maximum
    /// to estimate against. The feed is expected to carry hourly kW *and* kVA.
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

/// Estimates the net delivery cost attributable to EV charging sessions during a billing period.
pub fn peak_power_cost(
    billing_period_ending: Date,
    gb_period_values: PeriodValues,
    sessions: &[RSession],
    bill: &HydroBill,
) -> Result<DeliveryCost, PeakPowerError> {
    todo!()
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

    // The files' sessions become one report here rather than arriving as one. Deciding which
    // records are the same session, and what each surviving session is fit for, are both functions
    // of the sessions alone, which is why they sit on this side of the split.
    //
    // One list rather than one per file: `merge_sessions` compares each session against everything
    // already kept and never looks at list boundaries, so concatenating the files first gives the
    // same answer as handing them over separately.
    //
    // No log paths, because nothing here writes a log. The field records where a *reader* put one.
    let sessions_report = SessionReport::from_session_lists(vec![sessions.to_vec()], Vec::new());

    // Both estimates come off the one report, so the two figures cannot be drawn from different
    // session data.
    let sources = sources_of(sessions);
    Ok(PowerEstimates {
        kw_estimates: estimates_from_report(kw_ioi, sources.clone(), &sessions_report),
        kva_estimates: estimates_from_report(kva_ioi, sources, &sessions_report),
    })
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
            max_kw_nop: None,
            max_kva: kva_at.map(|at| peak(at, Tou::MidPeak)),
            max_kva_nop: None,
            anomaly_counts: BTreeMap::new(),
        }
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
    }
}
