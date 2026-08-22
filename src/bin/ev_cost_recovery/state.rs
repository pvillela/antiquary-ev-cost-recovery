//! The app's state, and every decision about it, with no egui in sight.
//!
//! The widget code above this is meant to be thin enough to check by eye; everything that could be
//! *wrong* rather than merely ugly — which hours a date offers, when the EST/EDT question has to be
//! asked, whether an estimate may be run at all, what a saved report is called — is decided here
//! and tested here.

use ev_cost_recovery::{
    session::{
        ConversionReport, HourEntry, IntervalEstimates, IoiLength, Sessions, checked_interval,
        excel::{session_csv_to_xlsx, session_list},
        hours_of, interval_estimates,
    },
    time::{Interval, TZ_OFFSETS, time_zone},
};
use jiff::civil;
use std::path::{Path, PathBuf};

/// Which of the two jobs the user is doing. `None` is the landing screen: the app opens with
/// neither tab chosen, so that converting and estimating are both deliberate acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Convert,
    Estimate,
}

#[derive(Default)]
pub struct AppState {
    pub tab: Option<Tab>,
    pub convert: ConvertState,
    pub estimate: EstimateState,
    pub working_dir: WorkingDir,
}

/// The folder the user is working in, shared by every file dialog in the app.
///
/// A month's CSV, the workbook made from it and the reports taken out of it all live together, and
/// the two tabs are two halves of one job — so this is one folder for the whole app rather than one
/// per dialog. It lasts as long as the app does and no longer: nothing is written to disk, so a
/// fresh launch starts wherever the system would have started anyway.
#[derive(Default)]
pub struct WorkingDir(Option<PathBuf>);

impl WorkingDir {
    /// The folder a dialog should open in, or `None` before the user has picked anything.
    pub fn get(&self) -> Option<&Path> {
        self.0.as_deref()
    }

    /// Remembers where a file was picked from or written to.
    pub fn remember(&mut self, file: &Path) {
        // A bare filename's parent is `""`, which would send the next dialog nowhere in particular.
        if let Some(dir) = file.parent()
            && !dir.as_os_str().is_empty()
        {
            self.0 = Some(dir.to_path_buf());
        }
    }
}

// --------------------------------------------------------------------------------------------
// Converting

/// What a finished conversion left behind.
pub struct ConvertOutcome {
    pub workbook: PathBuf,
    /// The rows that needed a judgement call — what the command line puts on stderr. Empty for a
    /// clean conversion.
    pub anomalies: Vec<String>,
}

#[derive(Default)]
pub struct ConvertState {
    pub csv: Option<PathBuf>,
    pub outcome: Option<ConvertOutcome>,
    pub error: Option<String>,
    /// Set when the workbook about to be written already exists. Replacing the file the estimates
    /// were argued from is the one destructive thing this app can do, so it is asked about.
    pub confirm_overwrite: Option<PathBuf>,
    /// The workbook a conversion has just written, waiting for the Estimate tab to collect it.
    ///
    /// Held rather than pushed, so that arriving at the Estimate tab by the button at the foot of
    /// this one and arriving by clicking the tab come to the same thing. Collected once: a
    /// workbook the user has since replaced by hand is not re-imposed on them.
    handoff: Option<PathBuf>,
}

impl ConvertState {
    pub fn select_csv(&mut self, csv: PathBuf) {
        self.csv = Some(csv);
        self.outcome = None;
        self.error = None;
    }

    /// Where the workbook will be written: the same rule `csv_to_xlsx` uses.
    pub fn target_workbook(&self) -> Option<PathBuf> {
        self.csv.as_ref().map(|p| p.with_extension("xlsx"))
    }

    /// Converts, or asks first if that would replace an existing workbook.
    pub fn start(&mut self) {
        self.error = None;
        self.outcome = None;
        match self.target_workbook() {
            Some(target) if target.exists() => self.confirm_overwrite = Some(target),
            Some(_) => self.convert(),
            None => {}
        }
    }

    pub fn convert(&mut self) {
        self.confirm_overwrite = None;
        let Some(csv) = self.csv.clone() else { return };
        match session_csv_to_xlsx(&csv) {
            Ok(ConversionReport {
                output_path,
                anomalies,
                log_path: _,
            }) => {
                self.handoff = Some(output_path.clone());
                self.outcome = Some(ConvertOutcome {
                    workbook: output_path,
                    anomalies: anomalies.iter().map(|a| a.to_string()).collect(),
                });
            }
            Err(e) => self.error = Some(format!("{}: {e}", csv.display())),
        }
    }

    /// Takes the workbook a conversion left waiting, if it has not been taken already.
    pub fn take_handoff(&mut self) -> Option<PathBuf> {
        self.handoff.take()
    }
}

// --------------------------------------------------------------------------------------------
// Estimating

/// A workbook the app has read once, to say what it covers.
pub struct Workbook {
    pub path: PathBuf,
    /// First and last local date any session in the workbook touches, or `None` for an empty one.
    pub covers: Option<(civil::Date, civil::Date)>,
}

impl Workbook {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// A finished estimate, with the report and the text of it side by side. The text is what the
/// command line prints, kept verbatim so that a saved report and a piped one are the same file.
pub struct EstimateOutcome {
    pub report: IntervalEstimates,
    pub text: String,
    /// The interval the figures are for, as it is written at the head of the report.
    pub heading: String,
}

pub struct EstimateState {
    pub workbook: Option<Workbook>,
    pub date: civil::Date,
    /// The hours `date` actually has, in order. The hour the clocks jump over when DST begins is
    /// not among them.
    pub hours: Vec<HourEntry>,
    pub hour: i8,
    pub minute: i8,
    pub length: IoiLength,
    /// The answer to the twice-a-year question, once it has been given.
    pub designator: Option<&'static str>,
    /// Whether the workbook in hand arrived from a conversion rather than being chosen here. The
    /// tab says so: a picker that fills itself in should account for itself.
    pub carried_over: bool,
    pub outcome: Option<EstimateOutcome>,
    pub error: Option<String>,
}

impl Default for EstimateState {
    fn default() -> Self {
        // Any date will do until a workbook says otherwise, and today is the least surprising one
        // to show in an empty picker.
        let date = jiff::Zoned::now().date();
        Self {
            workbook: None,
            hours: hours_of(date),
            date,
            hour: 0,
            minute: 0,
            length: IoiLength::Hour,
            designator: None,
            carried_over: false,
            outcome: None,
            error: None,
        }
    }
}

impl EstimateState {
    /// Reads the workbook once, to learn what it covers and to start the picker on a day it has
    /// something to say about. A workbook is last month's; today would be wrong nearly every time.
    /// Takes up the workbook a conversion just wrote. The same as choosing it here, except that
    /// the tab will say where it came from.
    pub fn adopt_workbook(&mut self, path: PathBuf) {
        self.select_workbook(path);
        self.carried_over = self.workbook.is_some();
    }

    pub fn select_workbook(&mut self, path: PathBuf) {
        self.clear_results();
        self.carried_over = false;
        match session_list(&path) {
            Ok(report) => {
                let covers = covered_range(&report);
                if let Some((first, _)) = covers {
                    self.set_date(first);
                }
                self.workbook = Some(Workbook { path, covers });
            }
            Err(e) => {
                self.workbook = None;
                self.error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    pub fn set_date(&mut self, date: civil::Date) {
        self.clear_results();
        self.hours = hours_of(date);
        self.date = date;
        // A date without the hour previously chosen is the DST-gap date; fall back rather than
        // leave a selection that names no instant.
        if !self.hours.iter().any(|h| h.hour == self.hour) {
            self.hour = self.hours.first().map_or(0, |h| h.hour);
        }
        // A new date is a new question, and last week's answer is not an answer to it.
        self.designator = None;
    }

    pub fn set_hour(&mut self, hour: i8) {
        self.clear_results();
        self.hour = hour;
        self.designator = None;
    }

    /// Setting a minute may take the length with it: an hour-long interval is legal only from
    /// `HH:00`, so the button that would have become illegal gives way rather than erroring.
    pub fn set_minute(&mut self, minute: i8) {
        self.clear_results();
        self.minute = minute;
        if !self.length.allowed_from(minute) {
            self.length = IoiLength::default_for(minute);
        }
    }

    pub fn set_length(&mut self, length: IoiLength) {
        self.clear_results();
        self.length = length;
    }

    pub fn set_designator(&mut self, designator: &'static str) {
        self.clear_results();
        self.designator = Some(designator);
    }

    /// Whether the chosen hour occurs twice, so that which one is meant has to be said.
    pub fn needs_designator(&self) -> bool {
        self.hours
            .iter()
            .find(|h| h.hour == self.hour)
            .is_some_and(|h| h.ambiguous)
    }

    /// Whether the Estimate button is live: a workbook in hand, and the twice-a-year question
    /// answered if it was asked.
    pub fn can_estimate(&self) -> bool {
        self.workbook.is_some() && (!self.needs_designator() || self.designator.is_some())
    }

    pub fn start_local(&self) -> civil::DateTime {
        self.date.at(self.hour, self.minute, 0, 0)
    }

    pub fn interval(&self) -> Result<Interval, String> {
        checked_interval(self.start_local(), Some(self.length), self.designator)
    }

    pub fn run(&mut self) {
        self.clear_results();
        let Some(workbook) = &self.workbook else {
            return;
        };
        let interval = match self.interval() {
            Ok(interval) => interval,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        match interval_estimates(interval, &workbook.path) {
            Ok(report) => {
                self.outcome = Some(EstimateOutcome {
                    text: report.to_markdown(),
                    heading: interval_heading(interval, self.length),
                    report,
                });
            }
            // No path prefix: the workbook reader names the file in every error it returns.
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// The name a saved report is offered under. It names the workbook and the exact window, so
    /// that a folder of them can be read without opening any.
    pub fn default_save_name(&self) -> String {
        let stem = self
            .workbook
            .as_ref()
            .and_then(|w| w.path.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "session_report".to_owned());
        let designator = self.designator.map(|d| format!("_{d}")).unwrap_or_default();
        format!(
            "{stem}_{}_{:02}{:02}{designator}_{}.report.md",
            self.date,
            self.hour,
            self.minute,
            self.length.label()
        )
    }

    /// Results describe the inputs that produced them, so changing an input drops them rather than
    /// leaving figures on screen that no longer answer what the controls now say.
    fn clear_results(&mut self) {
        self.outcome = None;
        self.error = None;
    }
}

/// First and last local date any session in the workbook touches — sessions that were excluded from
/// the estimates included, since they still say what month this is.
fn covered_range(report: &Sessions) -> Option<(civil::Date, civil::Date)> {
    let sessions = report
        .sessions
        .iter()
        .chain(&report.spikes)
        .chain(&report.excluded);
    let mut range: Option<(civil::Date, civil::Date)> = None;
    for session in sessions {
        let first = session.conn_start_local().date();
        let last = session.conn_end_local().date();
        range = Some(match range {
            None => (first, last),
            Some((lo, hi)) => (lo.min(first), hi.max(last)),
        });
    }
    range
}

/// The interval as the report writes it at its head: local times, and the offset named the way a
/// Toronto Hydro bill names it.
pub fn interval_heading(interval: Interval, length: IoiLength) -> String {
    let tz = time_zone();
    let (lo, hi) = (interval.start, interval.end());
    let start = lo.to_zoned(tz.clone());
    let end = hi.to_zoned(tz.clone());
    let name = TZ_OFFSETS
        .iter()
        .find(|(_, hours)| tz.to_offset(lo) == jiff::tz::Offset::constant(*hours))
        .map_or("", |(name, _)| name);
    let length = match length {
        IoiLength::Hour => "1 hour",
        IoiLength::FifteenMinutes => "15 minutes",
    };
    format!(
        "{} - {} {name}  ({length})",
        start.strftime("%Y-%m-%d %H:%M"),
        end.strftime("%H:%M"),
    )
}

// --------------------------------------------------------------------------------------------
// Reading the report back

/// One titled part of the report, as the report itself divides them.
pub struct Section {
    pub title: String,
    pub body: String,
}

/// Splits the report text into its sections, so each can be given its own collapsible heading.
///
/// The report is written to read as plain text, and a section title there is a line underlined by
/// dashes of its own length — which a table's `|:---|` separator is not, so tables inside a section
/// stay inside it. The preamble before the first such title is dropped: the source and the interval
/// it states are shown above these sections as headings in their own right.
pub fn report_sections(text: &str) -> Vec<Section> {
    let lines: Vec<&str> = text.lines().collect();
    let underlined = |i: usize| {
        i + 1 < lines.len()
            && !lines[i].trim().is_empty()
            && !lines[i + 1].is_empty()
            && lines[i + 1].chars().all(|c| c == '-')
            && lines[i + 1].len() == lines[i].len()
    };

    let starts: Vec<usize> = (0..lines.len()).filter(|&i| underlined(i)).collect();
    starts
        .iter()
        .enumerate()
        .map(|(k, &start)| {
            let end = starts.get(k + 1).copied().unwrap_or(lines.len());
            Section {
                title: lines[start].to_owned(),
                body: lines[start + 2..end]
                    .join("\n")
                    .trim_matches('\n')
                    .to_owned(),
            }
        })
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;

    fn state_on(date: civil::Date) -> EstimateState {
        let mut state = EstimateState::default();
        state.set_date(date);
        state
    }

    /// The whole path the app drives, end to end: convert a CSV, pick up the workbook it wrote,
    /// choose an interval from the controls, and estimate — checked against the same golden file
    /// the command line is checked against.
    ///
    /// This is what makes the widget layer safe to leave untested. Everything between a click and
    /// a figure happens here; a click is only which of these it calls.
    #[test]
    fn the_app_produces_the_same_report_as_the_command_line() {
        let fixtures =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions");
        for stem in ["Session_Report_Diagram", "Session_Report_Anomalies"] {
            // Convert in a scratch directory, so no generated workbook lands in the fixtures.
            let dir = std::env::temp_dir()
                .join(format!("ev_cost_recovery_{stem}_{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let csv = dir.join(format!("{stem}.csv"));
            std::fs::copy(fixtures.join(format!("{stem}.csv")), &csv).unwrap();

            let mut convert = ConvertState::default();
            convert.select_csv(csv);
            convert.start();
            assert!(convert.error.is_none(), "{stem}: {:?}", convert.error);
            let workbook = convert
                .outcome
                .as_ref()
                .expect("a conversion writes a workbook")
                .workbook
                .clone();

            // The handoff the Convert tab offers, then the controls the Estimate tab draws.
            let mut estimate = EstimateState::default();
            estimate.select_workbook(workbook);
            assert!(estimate.error.is_none(), "{stem}: {:?}", estimate.error);
            assert_eq!(
                estimate.date,
                civil::date(2026, 6, 15),
                "{stem}: the picker should start on the first day the workbook covers"
            );
            estimate.set_hour(16);
            estimate.set_minute(0);
            estimate.set_length(IoiLength::Hour);
            assert!(estimate.can_estimate());
            estimate.run();

            let outcome = estimate.outcome.as_ref().expect("{stem}: an estimate");
            let golden =
                std::fs::read_to_string(fixtures.join(format!("{stem}.report.md"))).unwrap();
            // The workbook is in a scratch directory, so only its name matches the golden file's.
            assert_eq!(
                outcome.text.replace(&dir.display().to_string(), ""),
                golden,
                "{stem}: the saved report must be what the command line prints"
            );
            // And the heading over the figures says what the report says over its own.
            assert!(
                golden.contains(&outcome.heading),
                "{stem}: heading {:?} is not in the report",
                outcome.heading
            );

            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// The date the clocks go forward: 02:00 never happens, so it is not offered and cannot be
    /// left selected by a change of date either.
    #[test]
    fn the_gap_hour_is_neither_offered_nor_left_selected() {
        let mut state = state_on(civil::date(2026, 6, 15));
        state.set_hour(2);
        assert_eq!(state.hour, 2);

        state.set_date(civil::date(2026, 3, 8));
        assert!(!state.hours.iter().any(|h| h.hour == 2));
        assert_ne!(
            state.hour, 2,
            "a selection naming no instant must not survive"
        );
        assert!(
            state.interval().is_ok(),
            "whatever it fell back to must be real"
        );
    }

    /// The date the clocks go back: 01:00 happens twice, so the question is asked and the button
    /// waits for the answer.
    #[test]
    fn the_fold_hour_is_asked_about_before_it_can_be_estimated() {
        let mut state = state_on(civil::date(2026, 11, 1));
        // A workbook is the other half of `can_estimate`; stand one in.
        state.workbook = Some(Workbook {
            path: PathBuf::from("whatever.xlsx"),
            covers: None,
        });

        state.set_hour(0);
        assert!(!state.needs_designator());
        assert!(state.can_estimate());

        state.set_hour(1);
        assert!(state.needs_designator());
        assert!(!state.can_estimate(), "the question is unanswered");
        assert!(state.interval().is_err());

        state.set_designator("EST");
        assert!(state.can_estimate());
        assert!(state.interval().is_ok());

        // Moving off the ambiguous hour and back asks again rather than reusing the old answer.
        state.set_hour(3);
        state.set_hour(1);
        assert!(!state.can_estimate());
    }

    /// An ordinary date asks nothing at all.
    #[test]
    fn an_ordinary_date_asks_nothing() {
        let state = state_on(civil::date(2026, 6, 15));
        assert_eq!(state.hours.len(), 24);
        assert!(!state.needs_designator());
    }

    /// The length rule is enforced by the controls giving way, not by an error: choosing a minute
    /// an hour cannot start from takes the length with it.
    #[test]
    fn choosing_a_quarter_minute_gives_up_the_hour_length() {
        let mut state = state_on(civil::date(2026, 6, 15));
        state.set_minute(0);
        state.set_length(IoiLength::Hour);
        assert!(state.interval().is_ok());

        state.set_minute(15);
        assert_eq!(state.length, IoiLength::FifteenMinutes);
        assert!(state.interval().is_ok(), "no illegal interval is reachable");

        state.set_minute(0);
        assert_eq!(
            state.length,
            IoiLength::FifteenMinutes,
            "and it does not spring back"
        );
    }

    /// Every reachable combination of the controls is a legal interval. This is the whole point of
    /// constraining the pickers rather than validating them.
    #[test]
    fn no_reachable_combination_is_off_spec() {
        for date in [
            civil::date(2026, 6, 15),
            civil::date(2026, 3, 8),
            civil::date(2026, 11, 1),
        ] {
            let mut state = state_on(date);
            let hours: Vec<HourEntry> = state.hours.clone();
            for entry in hours {
                state.set_hour(entry.hour);
                if entry.ambiguous {
                    state.set_designator("EST");
                }
                for minute in [0, 15, 30, 45] {
                    state.set_minute(minute);
                    for length in [IoiLength::FifteenMinutes, IoiLength::Hour] {
                        if !length.allowed_from(minute) {
                            continue;
                        }
                        state.set_length(length);
                        assert!(
                            state.interval().is_ok(),
                            "{date} {:02}:{minute:02} {length:?} should be reachable and legal",
                            entry.hour
                        );
                    }
                }
            }
        }
    }

    /// A saved report names the workbook and the exact window it is for.
    #[test]
    fn the_save_name_names_the_window() {
        let mut state = state_on(civil::date(2026, 6, 15));
        state.workbook = Some(Workbook {
            path: PathBuf::from("/data/Session_Report_June.xlsx"),
            covers: None,
        });
        state.set_hour(16);
        state.set_minute(0);
        state.set_length(IoiLength::Hour);
        assert_eq!(
            state.default_save_name(),
            "Session_Report_June_2026-06-15_1600_1h.report.md"
        );

        state.set_minute(45);
        assert_eq!(
            state.default_save_name(),
            "Session_Report_June_2026-06-15_1645_15m.report.md"
        );

        // On the night the clocks go back, the answer is part of what the file is for.
        state.set_date(civil::date(2026, 11, 1));
        state.set_hour(1);
        state.set_minute(30);
        state.set_designator("EST");
        assert_eq!(
            state.default_save_name(),
            "Session_Report_June_2026-11-01_0130_EST_15m.report.md"
        );
    }

    /// Changing any input drops the figures, so nothing on screen ever answers a question other
    /// than the one the controls are asking.
    #[test]
    fn changing_an_input_drops_stale_figures() {
        let mut state = state_on(civil::date(2026, 6, 15));
        for change in [
            (|s: &mut EstimateState| s.set_hour(5)) as fn(&mut EstimateState),
            |s| s.set_minute(15),
            |s| s.set_length(IoiLength::FifteenMinutes),
            |s| s.set_date(civil::date(2026, 6, 16)),
        ] {
            state.error = Some("stale".to_owned());
            change(&mut state);
            assert!(state.error.is_none());
        }
    }

    /// The heading over the figures says what the report says over its own.
    #[test]
    fn the_heading_states_the_local_window() {
        let mut state = state_on(civil::date(2026, 6, 15));
        state.set_hour(16);
        state.set_minute(0);
        state.set_length(IoiLength::Hour);
        let heading = interval_heading(state.interval().unwrap(), state.length);
        assert_eq!(heading, "2026-06-15 16:00 - 17:00 EDT  (1 hour)");

        state.set_minute(45);
        let heading = interval_heading(state.interval().unwrap(), state.length);
        assert_eq!(heading, "2026-06-15 16:45 - 17:00 EDT  (15 minutes)");
    }

    /// The report divides itself into sections and the app shows them as it finds them, so a
    /// section added to the report needs no change here. Checked against the golden files, which
    /// are what the report actually produces.
    #[test]
    fn the_report_is_split_at_its_own_section_titles() {
        let text =
            std::fs::read_to_string("tests/fixtures/sessions/Session_Report_Anomalies.report.md")
                .expect("run from the crate root");
        let titles: Vec<String> = report_sections(&text)
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(
            titles,
            [
                "Estimates",
                "Segments",
                "Segment membership",
                "Excluded sessions",
                "Anomalies",
            ]
        );

        // A table's separator row is not a section title, so tables stay inside their section.
        let sections = report_sections(&text);
        let segments = sections.iter().find(|s| s.title == "Segments").unwrap();
        assert!(segments.body.contains("| 16:00 "), "{}", segments.body);
        assert!(segments.body.contains("Times are local (ET)"));
    }

    /// The other golden file divides the same way, minus the sections its data does not reach: no
    /// session in it is excluded, so it carries no Excluded sessions section.
    #[test]
    fn every_golden_report_splits_into_titled_sections() {
        let fixture = "Session_Report_Diagram";
        let text = std::fs::read_to_string(format!("tests/fixtures/sessions/{fixture}.report.md"))
            .unwrap();
        let titles: Vec<String> = report_sections(&text)
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(
            titles,
            ["Estimates", "Segments", "Segment membership", "Anomalies"],
            "{fixture}"
        );
    }

    /// A conversion leaves its workbook waiting, and the Estimate tab collects it on arrival —
    /// once. The route taken to get there is not part of the story, which is the whole point: the
    /// button at the foot of the Convert tab and a click on the tab itself now do the same thing.
    #[test]
    fn a_conversion_hands_its_workbook_on_exactly_once() {
        let fixtures =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions");
        let dir = std::env::temp_dir().join(format!("ev_peak_handoff_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("Session_Report_Diagram.csv");
        std::fs::copy(fixtures.join("Session_Report_Diagram.csv"), &csv).unwrap();

        let mut convert = ConvertState::default();
        assert_eq!(convert.take_handoff(), None, "nothing to hand on yet");

        convert.select_csv(csv);
        convert.start();
        assert!(convert.error.is_none(), "{:?}", convert.error);

        let handed = convert
            .take_handoff()
            .expect("a conversion hands its workbook on");
        assert_eq!(handed, dir.join("Session_Report_Diagram.xlsx"));
        assert_eq!(
            convert.take_handoff(),
            None,
            "collected once: a workbook the user has since replaced is not re-imposed"
        );

        // Taking it up is choosing it, plus a note saying where it came from.
        let mut estimate = EstimateState::default();
        estimate.adopt_workbook(handed.clone());
        assert!(estimate.workbook.is_some());
        assert!(estimate.carried_over);

        // Choosing one by hand is not carried over, and clears the note.
        estimate.select_workbook(handed);
        assert!(!estimate.carried_over);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The working folder follows whatever file was last touched, so the next dialog opens where
    /// the user already is.
    #[test]
    fn the_working_folder_follows_the_last_file_used() {
        let mut dir = WorkingDir::default();
        assert_eq!(dir.get(), None, "nothing is assumed before the first pick");

        dir.remember(Path::new("/data/June/Session_Report.csv"));
        assert_eq!(dir.get(), Some(Path::new("/data/June")));

        // A save into a different folder moves it; that is where the user now is.
        dir.remember(Path::new("/data/reports/June_1600.report.md"));
        assert_eq!(dir.get(), Some(Path::new("/data/reports")));

        // A bare filename has no folder to speak of, and must not blank out the one held.
        dir.remember(Path::new("bare.xlsx"));
        assert_eq!(dir.get(), Some(Path::new("/data/reports")));
    }

    /// The workbook a conversion writes is the one `csv_to_xlsx` would write, since the overwrite
    /// question is asked about that file.
    #[test]
    fn the_conversion_target_follows_the_csv() {
        let mut convert = ConvertState::default();
        convert.select_csv(PathBuf::from("/data/report (4).csv"));
        assert_eq!(
            convert.target_workbook(),
            Some(PathBuf::from("/data/report (4).xlsx"))
        );
    }
}
