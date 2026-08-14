//! Per-period aggregates: the totals and the four maxima each billing period reports.
//!
//! Everything here runs on the raw source integers. The division that turns them into kWh, kW and
//! kVA happens once, in the sheet writer. The June 2026 invoice agrees with these figures to the
//! digit, and it would not survive accumulating 744 floating-point divisions before summing them.

use crate::green_button::{Anomaly, BillingPeriod, Reading, Readings};
use crate::time::{Interval, Tou, is_off_peak, tou_of};
use jiff::Timestamp;
use std::collections::BTreeMap;
use std::time::Duration;

const HOUR: Duration = Duration::from_secs(3600);

/// A reported maximum, and the state of the interval it was found in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peak {
    /// Raw source integer, not yet divided.
    pub value: i64,
    pub at: Timestamp,
    /// The other power figure at the same interval: kVA alongside a kW peak, kW alongside a kVA
    /// peak. `None` when that series had no reading for the hour, rather than zero.
    pub companion: Option<i64>,
    pub tou: Tou,
}

/// One row of the `Peak_values` sheet.
#[derive(Debug, Clone)]
pub struct PeriodValues {
    pub period: BillingPeriod,
    /// Hours that actually carried data. Placeholder rows standing in for a gap are excluded, so
    /// a feed with a hole reports fewer intervals than the period should contain and gets flagged.
    pub interval_count: i64,
    pub kwh_total: i64,
    /// Highest kW over every interval in the period.
    pub max_kw: Option<Peak>,
    /// Highest kW within Toronto Hydro's 7-7 demand window. `None` when the period contains no
    /// such interval at all, which happens only for a period truncated to a weekend.
    pub max_kw_nop: Option<Peak>,
    pub max_kva: Option<Peak>,
    pub max_kva_nop: Option<Peak>,
    pub anomaly_counts: BTreeMap<Anomaly, usize>,
}

impl PeriodValues {
    /// Whether the period holds every hour it should. Drives the red fill on `nbr_of_intervals`.
    pub fn is_complete(&self) -> bool {
        self.interval_count == self.period.expected_intervals()
    }
}

/// Groups hourly readings into billing periods and computes each period's row, ascending by
/// period.
pub fn period_values(readings: &Readings) -> Vec<PeriodValues> {
    let mut grouped: BTreeMap<BillingPeriod, Vec<&Reading>> = BTreeMap::new();
    for reading in &readings.rows {
        grouped
            .entry(BillingPeriod::containing(reading.start))
            .or_default()
            .push(reading);
    }

    grouped
        .into_iter()
        .map(|(period, rows)| {
            let mut anomaly_counts: BTreeMap<Anomaly, usize> = BTreeMap::new();
            for (at, kinds) in &readings.anomalies {
                if period.contains(*at) {
                    for kind in kinds {
                        *anomaly_counts.entry(*kind).or_default() += 1;
                    }
                }
            }

            PeriodValues {
                interval_count: rows.iter().filter(|r| !r.is_empty()).count() as i64,
                kwh_total: rows.iter().filter_map(|r| r.kwh).sum(),
                max_kw: peak(&rows, |r| r.kw, |r| r.kva, false),
                max_kw_nop: peak(&rows, |r| r.kw, |r| r.kva, true),
                max_kva: peak(&rows, |r| r.kva, |r| r.kw, false),
                max_kva_nop: peak(&rows, |r| r.kva, |r| r.kw, true),
                anomaly_counts,
                period,
            }
        })
        .collect()
}

/// The first interval maximising `value`, optionally restricted to the demand window.
///
/// Strictly greater, so an earlier interval keeps the title against a later equal one -- the
/// convention the reference workbook was built with.
///
/// Misaligned intervals are skipped entirely. That is what makes [`Peak::tou`] a plain `Tou`
/// rather than an `Option`: only an interval that starts on the hour can be a peak, and such an
/// interval cannot straddle a price-period boundary.
fn peak(
    rows: &[&Reading],
    value: impl Fn(&Reading) -> Option<i64>,
    companion: impl Fn(&Reading) -> Option<i64>,
    demand_window_only: bool,
) -> Option<Peak> {
    let mut best: Option<Peak> = None;
    for reading in rows {
        if !reading.is_aligned() {
            continue;
        }
        let Some(v) = value(reading) else { continue };
        let interval = Interval::new(reading.start, HOUR);
        if demand_window_only && is_off_peak(interval) {
            continue;
        }
        if best.is_some_and(|b| v <= b.value) {
            continue;
        }
        let tou = tou_of(interval).expect("an aligned hourly interval lies in one price period");
        debug_assert!(
            !demand_window_only || tou != Tou::OffPeak,
            "the demand window is exactly the complement of off-peak"
        );
        best = Some(Peak {
            value: v,
            at: reading.start,
            companion: companion(reading),
            tou,
        });
    }
    best
}

// cargo test --package green-button --lib -- peaks::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use crate::time::local_hour;
    use jiff::civil::date;

    /// Readings for consecutive hours starting at a local hour, with `(kwh, kw, kva)` each.
    fn readings_from(start: Timestamp, values: &[(i64, i64, i64)]) -> Readings {
        let rows = values
            .iter()
            .enumerate()
            .map(|(i, &(kwh, kw, kva))| Reading {
                start: Timestamp::from_second(start.as_second() + i as i64 * 3600).unwrap(),
                kwh: Some(kwh),
                kw: Some(kw),
                kva: Some(kva),
            })
            .collect();
        Readings {
            rows,
            anomalies: BTreeMap::new(),
        }
    }

    /// A tie goes to the earlier interval.
    #[test]
    fn the_first_maximising_interval_wins() {
        // Three summer-weekday hours from 12:00, all in the demand window.
        let start = local_hour(date(2026, 6, 15), 12);
        let values = period_values(&readings_from(
            start,
            &[(10, 50, 60), (10, 50, 60), (10, 40, 45)],
        ));
        let peak = values[0].max_kw.unwrap();
        assert_eq!(peak.value, 50);
        assert_eq!(peak.at, start, "the earlier of two equal maxima");
        assert_eq!(peak.companion, Some(60));
        assert_eq!(peak.tou, Tou::OnPeak);
    }

    /// The demand-window maximum ignores a larger overnight peak.
    #[test]
    fn the_demand_window_peak_excludes_off_peak_hours() {
        // 05:00 and 06:00 are off-peak; 07:00 is the first hour of the demand window.
        let start = local_hour(date(2026, 6, 15), 5);
        let values = period_values(&readings_from(
            start,
            &[(10, 99, 99), (10, 10, 10), (10, 20, 20)],
        ));
        let row = &values[0];
        assert_eq!(
            row.max_kw.unwrap().value,
            99,
            "unrestricted peak sees the overnight hour"
        );
        assert_eq!(row.max_kw.unwrap().tou, Tou::OffPeak);
        let restricted = row.max_kw_nop.unwrap();
        assert_eq!(
            restricted.value, 20,
            "the 07:00 hour, not the larger 05:00 one"
        );
        assert_eq!(restricted.at, local_hour(date(2026, 6, 15), 7));
        assert_ne!(
            restricted.tou,
            Tou::OffPeak,
            "a demand-window peak is never off-peak"
        );
    }

    /// A period made only of off-peak hours has no demand-window peak at all, and the columns are
    /// left blank rather than filled with the unrestricted figure.
    #[test]
    fn a_weekend_only_period_has_no_demand_window_peak() {
        let start = local_hour(date(2026, 6, 13), 0); // Saturday
        let values = period_values(&readings_from(start, &[(10, 50, 60); 24]));
        assert!(values[0].max_kw.is_some());
        assert!(values[0].max_kw_nop.is_none());
        assert!(values[0].max_kva_nop.is_none());
    }

    /// Totals and counts ignore holes, so an incomplete period reads as incomplete.
    #[test]
    fn a_placeholder_row_counts_towards_neither_the_total_nor_the_interval_count() {
        let start = local_hour(date(2026, 6, 15), 12);
        let mut readings = readings_from(start, &[(10, 50, 60), (10, 50, 60)]);
        readings.rows.push(Reading {
            start: Timestamp::from_second(start.as_second() + 7200).unwrap(),
            kwh: None,
            kw: None,
            kva: None,
        });
        let row = &period_values(&readings)[0];
        assert_eq!(row.interval_count, 2);
        assert_eq!(row.kwh_total, 20);
        assert!(!row.is_complete());
    }

    /// A misaligned interval cannot become a peak, however large.
    #[test]
    fn a_misaligned_interval_is_never_the_peak() {
        let start = local_hour(date(2026, 6, 15), 12);
        let mut readings = readings_from(start, &[(10, 50, 60)]);
        readings.rows.push(Reading {
            start: Timestamp::from_second(start.as_second() + 1800).unwrap(),
            kwh: Some(99),
            kw: Some(999),
            kva: Some(999),
        });
        let row = &period_values(&readings)[0];
        assert_eq!(row.max_kw.unwrap().value, 50);
        assert_eq!(
            row.kwh_total, 109,
            "but its energy still counts towards the total"
        );
    }

    /// A companion series with no reading for the hour stays blank rather than becoming zero.
    #[test]
    fn a_missing_companion_is_none_not_zero() {
        let start = local_hour(date(2026, 6, 15), 12);
        let mut readings = readings_from(start, &[(10, 50, 60)]);
        readings.rows[0].kva = None;
        let peak = period_values(&readings)[0].max_kw.unwrap();
        assert_eq!(peak.companion, None);
    }

    /// Readings either side of local midnight on the 24th land in different periods.
    #[test]
    fn readings_are_grouped_by_billing_period() {
        let start = local_hour(date(2026, 6, 23), 22);
        let values = period_values(&readings_from(start, &[(1, 1, 1); 4]));
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].period.ending, date(2026, 6, 23));
        assert_eq!(values[0].interval_count, 2);
        assert_eq!(values[1].period.ending, date(2026, 7, 23));
        assert_eq!(values[1].interval_count, 2);
    }
}
