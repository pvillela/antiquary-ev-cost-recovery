//! Plain-text run logs written beside each output file.
//!
//! One per operation, overwritten each run: `<stem>.convert.log` for CSV to Excel,
//! `<stem>.read.log` for Excel back to sessions. A log always says one of two things — that
//! everything was fine, or what was found — so an empty-looking run is distinguishable from one
//! that was never made.
//!
//! # Why discrepancies are not anomalies
//!
//! [`AnomalyKind`](super::AnomalyKind) describes the *session*: something about the reported data
//! needs a judgement call, and one kind, `InconsistentDuration`, removes the session from every
//! estimate. A discrepancy describes the *workbook*: a stored column disagrees with what the
//! `Session` methods recompute, which means the sheet is stale or was edited, and says nothing
//! about the session itself.
//!
//! Keeping them apart is the point. If a stale cell could raise an anomaly, editing a workbook
//! could silently change which sessions feed an estimate — and the estimate would still look
//! clean. The recomputed value always wins; the disagreement is logged and nothing else changes.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// What a log records: either it is empty, or every line is something that needs a reader.
#[derive(Debug, Default)]
pub struct RunLog {
    /// Free-text lines, in the order they were found.
    entries: Vec<String>,
}

impl RunLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn note(&mut self, line: impl Into<String>) {
        self.entries.push(line.into());
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The log's text, ending in a newline.
    ///
    /// `operation` names what was run and `subject` the file it ran on; both appear in the header
    /// so a log read on its own still says what it is about.
    pub fn render(&self, operation: &str, subject: &Path) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{operation}: {}", subject.display());
        if self.entries.is_empty() {
            let _ = writeln!(
                out,
                "\nNothing to report. No errors, warnings or anomalies."
            );
            return out;
        }
        let _ = writeln!(
            out,
            "\n{} item(s) to review, in the order found:\n",
            self.entries.len()
        );
        for entry in &self.entries {
            let _ = writeln!(out, "  {entry}");
        }
        out
    }

    /// Writes the log beside `output`, replacing any extension with `<suffix>.log`.
    ///
    /// Returns where it went, so a caller can tell the user. A failure to write the log is
    /// returned rather than swallowed: a log the user believes exists and does not is worse than
    /// no log at all.
    pub fn write_beside(
        &self,
        output: &Path,
        suffix: &str,
        operation: &str,
    ) -> Result<PathBuf, Box<dyn Error>> {
        let path = log_path(output, suffix);
        std::fs::write(&path, self.render(operation, output))?;
        Ok(path)
    }
}

/// `<stem>.<suffix>.log` beside `output`.
fn log_path(output: &Path, suffix: &str) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session_report".to_owned());
    output.with_file_name(format!("{stem}.{suffix}.log"))
}

// cargo test --lib -- sessions::log::test --nocapture
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn an_empty_log_says_so_rather_than_being_blank() {
        let text = RunLog::new().render("Converted", Path::new("/tmp/June.xlsx"));
        assert!(text.contains("Nothing to report"), "{text}");
        assert!(text.contains("June.xlsx"), "{text}");
    }

    #[test]
    fn a_log_lists_what_it_found_in_order() {
        let mut log = RunLog::new();
        log.note("row 2: first");
        log.note("row 9: second");
        let text = log.render("Read", Path::new("/tmp/June.xlsx"));
        assert!(text.contains("2 item(s)"), "{text}");
        assert!(
            text.find("first").unwrap() < text.find("second").unwrap(),
            "{text}"
        );
        assert!(!text.contains("Nothing to report"), "{text}");
    }

    /// The log sits beside the workbook and replaces its extension, so a `.xlsx` and its log share
    /// a stem and sort together in a directory listing.
    #[test]
    fn the_log_path_sits_beside_its_output() {
        assert_eq!(
            log_path(Path::new("/data/Session_Report_June.xlsx"), "convert"),
            Path::new("/data/Session_Report_June.convert.log")
        );
        assert_eq!(
            log_path(Path::new("/data/Session_Report_June.xlsx"), "read"),
            Path::new("/data/Session_Report_June.read.log")
        );
    }
}
