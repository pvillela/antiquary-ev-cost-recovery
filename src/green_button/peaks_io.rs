//! Reading one billing period's figures straight out of a Green Button export.
//!
//! [`period_values`] takes a parsed [`Readings`] and returns every period the feed covers. A
//! caller checking one invoice starts from a path and a closing date, and needs to be told when
//! the feed does not reach that period rather than handed nothing. This module is that call.

use crate::green_button::{PeriodValues, parse, period_values};
use crate::time::local_date;
use jiff::civil::Date;
use std::{error::Error, path::Path};

/// The largest day of the month a billing period may close on.
///
/// A period starts the day *after* it closes, in the previous month — see
/// [`bill_start_day`](crate::hydro_bills::bill_start_day) — so a closing day of 28 would put the
/// start on February 29th, a date that does not exist in three years out of four. Capping at 27
/// keeps every period's start a real date in every month.
const MAX_BILL_END_DAY: i8 = 27;

/// Reads the Green Button XML at `xml_path` and returns the values for the billing period ending
/// on `period_ending`.
///
/// `bill_end_day` is the day of the month the bill closes on — [`BILL_END_DAY`](crate::hydro_bills::BILL_END_DAY)
/// for Toronto Hydro — and must be `period_ending`'s day. It is stated rather than inferred
/// because which day a bill closes on belongs to the bill and not to the meter data; see
/// `green_button::billing`.
///
/// A period the feed covers only partly is returned, not refused: the feed's coverage and the
/// bill's period are independent facts, and which discrepancies matter is the caller's judgement.
/// [`PeriodValues::is_complete`] says whether every hour is there, and
/// [`PeriodValues::anomaly_counts`] carries every anomaly found within the period.
///
/// # Errors
///
/// Returns `Err` if the file cannot be read, if the XML cannot be parsed as an ESPI feed, if
/// `period_ending` is not `bill_end_day` of its month, if `bill_end_day` is outside
/// `1..=MAX_BILL_END_DAY`, or if the feed carries no reading in the period asked for. The last is
/// an error rather than an empty row: a caller checking an invoice against a period the export
/// does not reach needs to be told so, not handed zeroes.
pub fn period_values_xml(
    xml_path: &Path,
    period_ending: Date,
    bill_end_day: i8,
) -> Result<PeriodValues, Box<dyn Error>> {
    check_calendar(period_ending, bill_end_day)?;

    // Every error out of this function names the file it concerns, including the ones below where
    // the underlying error would not: a bare "No such file or directory" says nothing a reader can
    // act on. A caller that has the path already should therefore not add it again.
    let xml =
        std::fs::read_to_string(xml_path).map_err(|e| format!("{}: {e}", xml_path.display()))?;
    let feed = parse(&xml).map_err(|e| format!("{}: {e}", xml_path.display()))?;
    let readings = feed.readings();

    // `period_values` already scopes each period's anomaly counts to that period, so picking the
    // row out of its result is the whole of the work. Nothing is recomputed here, and no second
    // anomaly channel is built beside the counts it carries.
    period_values(&readings, bill_end_day)
        .into_iter()
        .find(|p| p.period.ending == period_ending)
        .ok_or_else(|| {
            format!(
                "{}: no readings in the billing period ending {period_ending}. {}",
                xml_path.display(),
                coverage(&readings)
            )
            .into()
        })
}

/// Rejects a `period_ending` and `bill_end_day` that cannot describe the same calendar.
///
/// Runs before the file is opened, so a caller who has mixed up two calendars is told that rather
/// than being sent looking at the export.
fn check_calendar(period_ending: Date, bill_end_day: i8) -> Result<(), Box<dyn Error>> {
    // Both checks belong here rather than to `BillingPeriod::ending_on`, which asserts the second
    // and would build an invalid `Date` on the first. Either way it panics, and these arguments
    // come from a caller.
    if !(1..=MAX_BILL_END_DAY).contains(&bill_end_day) {
        return Err(format!(
            "bill_end_day is {bill_end_day}; a billing period must close on day 1 to \
             {MAX_BILL_END_DAY} of the month, since it starts on the day after and that day has \
             to exist in February"
        )
        .into());
    }
    if period_ending.day() != bill_end_day {
        return Err(format!(
            "{period_ending} cannot end a billing period that closes on day {bill_end_day} of the \
             month; the closing date and the calendar disagree"
        )
        .into());
    }
    Ok(())
}

/// What the feed actually covers, for an error that says where to look next.
fn coverage(readings: &crate::green_button::Readings) -> String {
    match (readings.rows.first(), readings.rows.last()) {
        (Some(first), Some(last)) => format!(
            "The feed covers {} to {}.",
            local_date(first.start),
            local_date(last.start)
        ),
        _ => "The feed carries no readings at all.".to_owned(),
    }
}

// cargo test --lib -- green_button::peaks_io::test --nocapture
#[cfg(test)]
mod test {
    use super::*;
    use crate::hydro_bills::BILL_END_DAY;
    use jiff::civil::date;
    use std::path::PathBuf;

    /// The arguments are checked before the file is, so a mismatched calendar is reported as such
    /// rather than as a missing file.
    fn err_for(ending: Date, bill_end_day: i8) -> String {
        period_values_xml(
            &PathBuf::from("/nonexistent/feed.XML"),
            ending,
            bill_end_day,
        )
        .unwrap_err()
        .to_string()
    }

    #[test]
    fn a_closing_date_off_the_billing_calendar_is_refused() {
        let err = err_for(date(2026, 6, 30), BILL_END_DAY);
        assert!(err.contains("2026-06-30"), "{err}");
        assert!(err.contains("disagree"), "{err}");
    }

    /// The day after `MAX_BILL_END_DAY` would start a period on February 29th, which exists in one
    /// year out of four. `BillingPeriod::ending_on` would build that date and panic.
    #[test]
    fn a_closing_day_that_would_start_a_period_on_a_missing_date_is_refused() {
        let err = err_for(date(2026, 6, 28), 28);
        assert!(err.contains("February"), "{err}");

        // One day earlier is the last legal closing day, so it gets past the calendar check and
        // fails on the file instead.
        let err = err_for(date(2026, 6, 27), 27);
        assert!(err.contains("/nonexistent/feed.XML"), "{err}");
    }

    #[test]
    fn a_missing_file_is_named() {
        let err = err_for(date(2026, 6, 23), BILL_END_DAY);
        assert!(err.contains("/nonexistent/feed.XML"), "{err}");
    }
}
