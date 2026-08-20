//! `period_values_xml` against a real export.
//!
//! The unit tests beside the function cover the argument checks, which need no file. What needs a
//! fixture is the part that matters: that the packaged call returns the same period the long-hand
//! pipeline does, and that asking for a period the feed does not reach is an error rather than a
//! row of zeroes.

use ev_cost_recovery::green_button::{parse, period_values, period_values_xml};
use ev_cost_recovery::hydro_bills::BILL_END_DAY;
use jiff::civil::date;

use super::fixture;

/// The one-call form and the long-hand pipeline describe the same period.
///
/// Asserted against the pipeline rather than against copied figures, so this cannot drift from
/// what the workbook writer reports for the same period. The invoice test is what pins the figures
/// themselves to something outside the software.
#[test]
fn the_packaged_call_matches_the_long_hand_pipeline() {
    let path = fixture("billed_period.XML");
    let ending = date(2026, 6, 23);

    let xml = std::fs::read_to_string(&path).unwrap();
    let readings = parse(&xml).unwrap().readings();
    let expected = period_values(&readings, BILL_END_DAY)
        .into_iter()
        .find(|p| p.period.ending == ending)
        .expect("the fixture carries the billed period");

    let actual = period_values_xml(&path, ending, BILL_END_DAY).unwrap();

    assert_eq!(actual.period, expected.period);
    assert_eq!(actual.interval_count, expected.interval_count);
    assert_eq!(actual.kwh_total, expected.kwh_total);
    assert_eq!(actual.max_kw, expected.max_kw);
    assert_eq!(actual.max_kw_nop, expected.max_kw_nop);
    assert_eq!(actual.max_kva, expected.max_kva);
    assert_eq!(actual.max_kva_nop, expected.max_kva_nop);
    assert_eq!(actual.anomaly_counts, expected.anomaly_counts);

    // The fixture is exactly the billed period, so it is whole.
    assert!(actual.is_complete());
    assert_eq!(actual.interval_count, 744, "31 days, no clock change");
}

/// A period the export does not reach is an error, and the error says what the export does cover.
#[test]
fn a_period_outside_the_export_is_an_error_naming_what_is_there() {
    let err = period_values_xml(
        &fixture("billed_period.XML"),
        date(2020, 1, 23),
        BILL_END_DAY,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("2020-01-23"), "{err}");
    assert!(err.contains("The feed covers"), "{err}");
    assert!(err.contains("2026"), "the covered range is named: {err}");
}
