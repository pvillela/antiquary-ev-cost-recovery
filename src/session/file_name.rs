//! What a session report's *file name* says, and whether a set of them reaches from one date to
//! another.
//!
//! The name is what a span of dates is checked against, rather than the records inside. A report
//! legitimately holds no session on a quiet day, so its contents cannot tell "nobody charged" apart
//! from "wrong file", and it is the second of those that would quietly halve an estimate.
//!
//! Nothing here opens anything. A `&Path` is read as a string, which is why this sits beside
//! [`csv`](super::csv) rather than inside it: that module turns the file into [`Session`](super::Session)s,
//! and this one never gets that far.
//!
//! What billing period the dates read here have to cover is not a question about a file name, so it
//! is not answered here. [`api::pure::coverage`](crate::pure::coverage) joins the two.

use jiff::civil::Date;
use std::path::{Path, PathBuf};

/// What a session report's file name says it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReportCoverage {
    pub path: PathBuf,
    /// First calendar date the report covers.
    pub from: Date,
    /// Last calendar date the report covers, inclusive.
    pub to: Date,
}

/// The calendar dates a session report's file name says it covers, as in
/// `Session_Report_June_1_2026-June_30_2026.csv`.
///
/// `None` when the name is not of that form. Nothing else about the file is inspected — in
/// particular, not whether it exists.
pub fn report_coverage(path: &Path) -> Option<SessionReportCoverage> {
    let stem = path.file_stem()?.to_str()?;
    // Anything after the closing date is ignored, so a file marked up by hand -- a `-mock`, a
    // `-bak`, a `-what-if` -- still says what it covers. The two dates are what is read; a suffix
    // is a note to a person and says nothing about the sessions inside.
    let mut parts = stem.strip_prefix("Session_Report_")?.split('-');
    let from = report_date(parts.next()?)?;
    let to = report_date(parts.next()?)?;
    Some(SessionReportCoverage {
        path: path.to_path_buf(),
        from,
        to,
    })
}

/// `June_1_2026` as a date.
fn report_date(s: &str) -> Option<Date> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    let [month, day, year] = s.split('_').collect::<Vec<_>>()[..] else {
        return None;
    };
    let month = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(month))? as i8 + 1;
    // `Date::new` rather than the `date!` macro's panic on a day the month does not have: a file
    // name is input, and `Session_Report_June_31_2026-...` is a name to reject, not to crash on.
    Date::new(year.parse().ok()?, month, day.parse().ok()?).ok()
}

/// Whether the reports cover every date from `first` to `last` inclusive.
///
/// Order-insensitive, and tolerant of the reports overlapping, which they do whenever a session
/// runs past midnight on the last of the month. What it will not accept is a gap between them, or a
/// set that stops short at either end.
pub fn reports_cover(first: Date, last: Date, coverage: &[SessionReportCoverage]) -> bool {
    let mut ranges: Vec<(Date, Date)> = coverage.iter().map(|c| (c.from, c.to)).collect();
    ranges.sort();

    // The last date covered so far, started just before the period rather than at `first` so that a
    // report reaching back before the period connects on the same test as one starting inside it,
    // and so that a report lying entirely before the period neither helps nor blocks.
    let mut through = first.yesterday().unwrap_or(Date::MIN);
    for (from, to) in ranges {
        if from > through.tomorrow().unwrap_or(Date::MAX) {
            break; // A gap. Nothing later can fill it, since the ranges are in order.
        }
        if to > through {
            through = to;
        }
    }
    through >= last
}

#[cfg(test)]
mod test {
    use super::*;
    use jiff::civil::date;

    fn coverage(name: &str) -> SessionReportCoverage {
        report_coverage(Path::new(name)).unwrap_or_else(|| panic!("{name} should parse"))
    }

    /// The name Evolute gives its exports, which is the only form this reads.
    #[test]
    fn a_report_name_states_the_dates_it_covers() {
        let c = coverage("data/Session_Report_June_1_2026-June_30_2026.csv");
        assert_eq!(c.from, date(2026, 6, 1));
        assert_eq!(c.to, date(2026, 6, 30));
        // A period straddling the new year is two years, not one repeated.
        let c = coverage("Session_Report_December_1_2025-January_31_2026.csv");
        assert_eq!(c.from, date(2025, 12, 1));
        assert_eq!(c.to, date(2026, 1, 31));
    }

    /// A suffix on the name is a note to a person, so it is ignored rather than allowed to hide
    /// what the file covers. `data` holds several such files.
    #[test]
    fn a_marked_up_name_still_states_its_dates() {
        for name in [
            "data/Session_Report_July_1_2026-July_31_2026-mock.csv",
            "Session_Report_July_1_2026-July_31_2026-bak.csv",
            "Session_Report_July_1_2026-July_31_2026-what-if.csv",
        ] {
            let c = coverage(name);
            assert_eq!((c.from, c.to), (date(2026, 7, 1), date(2026, 7, 31)));
        }
    }

    /// Anything else is refused rather than guessed at, because the guess would be checked against
    /// the billing period and could pass.
    #[test]
    fn a_name_that_does_not_state_its_dates_is_refused() {
        for name in [
            "June.csv",
            "Session_Report_June_2026.csv",
            "Session_Report_June_1_2026.csv",
            "Session_Report_Jun_1_2026-Jun_30_2026.csv",
            // June has 30 days, so this is a name to reject rather than a date to build.
            "Session_Report_June_1_2026-June_31_2026.csv",
        ] {
            assert!(
                report_coverage(Path::new(name)).is_none(),
                "{name} should not parse"
            );
        }
    }

    /// The billing period ending 23 June 2026 runs from 24 May, so it takes both months' reports.
    #[test]
    fn two_monthly_reports_cover_a_billing_period() {
        let (first, last) = (date(2026, 5, 24), date(2026, 6, 23));
        let may = coverage("Session_Report_May_1_2026-May_31_2026.csv");
        let june = coverage("Session_Report_June_1_2026-June_30_2026.csv");

        assert!(reports_cover(first, last, &[may.clone(), june.clone()]));
        // Which one is named first is not a rule; the names say what each holds.
        assert!(reports_cover(first, last, &[june.clone(), may.clone()]));
        // Either alone falls short at one end.
        assert!(!reports_cover(first, last, &[may.clone(), may.clone()]));
        assert!(!reports_cover(first, last, &[june.clone(), june.clone()]));
    }

    /// A month missing from the middle is what handing in the wrong file looks like.
    #[test]
    fn a_gap_between_the_reports_is_not_coverage() {
        let (first, last) = (date(2026, 5, 24), date(2026, 6, 23));
        let april = coverage("Session_Report_April_1_2026-April_30_2026.csv");
        let june = coverage("Session_Report_June_1_2026-June_30_2026.csv");
        assert!(!reports_cover(first, last, &[april, june]));
    }

    /// A report reaching back before the period covers its part of it, and one lying entirely
    /// before the period neither helps nor blocks the one that does.
    #[test]
    fn a_report_wider_than_the_period_still_counts() {
        let (first, last) = (date(2026, 5, 24), date(2026, 6, 23));
        let spring = coverage("Session_Report_March_1_2026-June_30_2026.csv");
        let january = coverage("Session_Report_January_1_2026-January_31_2026.csv");
        assert!(reports_cover(
            first,
            last,
            &[spring.clone(), january.clone()]
        ));
        assert!(reports_cover(first, last, &[january, spring]));
    }
}
