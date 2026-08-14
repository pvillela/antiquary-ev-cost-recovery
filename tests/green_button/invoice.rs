//! Checks the computation against a real Toronto Hydro invoice.
//!
//! This is the only test whose expected values come from outside the software. Everything else
//! pins behaviour against itself or against a workbook this project's own predecessor produced; if
//! a rule here were wrong in a self-consistent way, only this test would notice.
//!
//! The invoice covers `MAY 23 2026 TO JUN 23 2026`, which is the period the `billed_period`
//! fixture carries in full.

use ev_cost_recovery::green_button::{parse, period_values};
use ev_cost_recovery::time::{Interval, Tou, tou_of};
use jiff::civil::date;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const HOUR: Duration = Duration::from_secs(3600);

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The `key value` pairs from the invoice fixture, comments and blanks skipped.
fn invoice() -> HashMap<String, String> {
    let text = std::fs::read_to_string(fixtures_dir().join("invoice_2026_06.txt")).unwrap();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once(char::is_whitespace))
        .map(|(k, v)| (k.to_string(), v.trim().to_string()))
        .collect()
}

fn number(invoice: &HashMap<String, String>, key: &str) -> f64 {
    invoice[key]
        .parse()
        .unwrap_or_else(|_| panic!("{key} is not a number"))
}

/// The invoice truncates rather than rounds: it prints 153.119 for a measured 153.119996. So a
/// generated figure agrees when it is within one thousandth *above* the printed one.
fn agrees_with_truncated(generated: f64, printed: f64) -> bool {
    let delta = generated - printed;
    (0.0..0.001).contains(&delta)
}

#[test]
fn the_billed_period_reproduces_the_invoice() {
    let invoice = invoice();
    let xml = std::fs::read_to_string(fixtures_dir().join("billed_period.XML")).unwrap();
    let feed = parse(&xml).unwrap();
    let readings = feed.readings();

    let ending = date(2026, 6, 23);
    let period = period_values(&readings)
        .into_iter()
        .find(|p| p.period.ending == ending)
        .expect("the fixture carries the billed period");

    assert_eq!(period.interval_count, 744, "31 days, no clock change");
    assert!(period.is_complete());

    let kwh = period.kwh_total as f64 / feed.kwh.divisor();
    let kw = |v: i64| v as f64 / feed.kw.divisor();
    let kva = |v: i64| v as f64 / feed.kva.divisor();

    for (name, generated, key) in [
        ("demand kW", kw(period.max_kw.unwrap().value), "demand_kw"),
        (
            "peak kW 7-7",
            kw(period.max_kw_nop.unwrap().value),
            "peak_kw_7_7",
        ),
        (
            "demand kVA",
            kva(period.max_kva.unwrap().value),
            "demand_kva",
        ),
    ] {
        let printed = number(&invoice, key);
        assert!(
            agrees_with_truncated(generated, printed),
            "{name}: generated {generated}, invoice {printed}"
        );
    }

    // The energy total does not agree exactly, and is not expected to. The invoice reads 11.16 kWh
    // higher out of 77,292 -- 0.014%. At roughly 104 kWh an hour a period-boundary error would
    // show as a ~104 kWh gap, so this is a meter read taken a few minutes off local midnight, not
    // a boundary that is wrong. See the TOU check below, where the whole difference lands in
    // off-peak, which is what a reading either side of midnight would do.
    let printed_kwh = number(&invoice, "kwh_used");
    let gap = printed_kwh - kwh;
    assert!(
        (0.0..20.0).contains(&gap),
        "kWh: generated {kwh}, invoice {printed_kwh}"
    );
}

/// The strongest check on the TOU rules: the season, the hour boundaries, the weekday rule and the
/// holiday calendar all have to be right at once for these to land.
///
/// The buckets are computed here rather than written to the workbook. The invoice states them
/// loss-factor adjusted, and the workbook deliberately reports raw meter values, so putting an
/// adjusted figure in a column would mean the sheet no longer agreed with itself.
#[test]
fn the_tou_buckets_reproduce_the_invoice() {
    let invoice = invoice();
    let loss_factor = number(&invoice, "loss_factor");
    let xml = std::fs::read_to_string(fixtures_dir().join("billed_period.XML")).unwrap();
    let feed = parse(&xml).unwrap();
    let readings = feed.readings();

    let period = period_values(&readings)
        .into_iter()
        .find(|p| p.period.ending == date(2026, 6, 23))
        .unwrap();

    let mut buckets: HashMap<Tou, i64> = HashMap::new();
    for reading in readings
        .rows
        .iter()
        .filter(|r| period.period.contains(r.start))
    {
        let Some(kwh) = reading.kwh else { continue };
        let tou = tou_of(Interval::new(reading.start, HOUR)).expect("hourly data is aligned");
        *buckets.entry(tou).or_default() += kwh;
    }

    let divisor = feed.kwh.divisor();
    let cases = [
        (Tou::OnPeak, "tou_on_peak_kwh", 0.001),
        (Tou::MidPeak, "tou_mid_peak_kwh", 0.001),
        // The whole meter-read discrepancy lands here, since both period boundaries are midnight
        // and midnight is off-peak. Tolerance covers it; the other two are exact.
        (Tou::OffPeak, "tou_off_peak_kwh", 12.0),
    ];
    for (tou, key, tolerance) in cases {
        let generated = buckets[&tou] as f64 / divisor;
        let expected = number(&invoice, key) / loss_factor;
        assert!(
            (generated - expected).abs() < tolerance,
            "{tou}: generated {generated:.3}, invoice/loss factor {expected:.3}"
        );
    }
}
