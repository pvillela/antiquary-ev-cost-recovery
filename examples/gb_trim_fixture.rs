//! Cuts a small, still-valid ESPI feed out of a large one, to make a test fixture.
//!
//! Fixtures are trimmed from real data rather than synthesised, so what the tests exercise is the
//! feed a utility actually produces. This is committed so that a fixture can be regenerated and
//! its provenance checked, rather than being a binary nobody can account for.
//!
//! Trimming works at whole-`<entry>` granularity: the `LocalTimeParameters`, `UsagePoint`,
//! `ReadingType` and `MeterReading` entries are always kept, so the link chain the parser follows
//! stays intact, and `IntervalBlock` entries are kept when the day they cover falls in range. Each
//! block is a whole day anchored to 05:00 UTC, which is local midnight in winter but 01:00 in
//! summer, so a range is not expected to line up with a billing period's local-midnight edges --
//! give it a day of slack either side and let the partial periods stand. They exercise the
//! incomplete-period highlight.

use ev_cost_recovery::green_button::local_date;
use jiff::Timestamp;
use jiff::civil::Date;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "\
trim_fixture -- cut a small ESPI feed out of a large one.

Keeps every entry needed to make the feed parse, plus the IntervalBlocks covering days from FROM to
TO inclusive, both read as Toronto local dates. Writes to stdout.

Give a day of slack either side of the billing period you want covered: blocks are anchored to
05:00 UTC, so in summer they start at 01:00 local and will not line up with a period boundary.

Usage:
    trim_fixture <XML> <FROM> <TO> > out.XML

Example:
    trim_fixture data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML 2025-07-23 2025-08-24

See docs/maintenance-manual.md.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let [input, from, to] = args.as_slice() else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };

    match run(Path::new(input), from, to) {
        Ok(out) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &Path, from: &str, to: &str) -> Result<String, String> {
    let from: Date = from.parse().map_err(|e| format!("{from}: {e}"))?;
    let to: Date = to.parse().map_err(|e| format!("{to}: {e}"))?;
    let xml = std::fs::read_to_string(input).map_err(|e| format!("{}: {e}", input.display()))?;
    trim(&xml, from, to)
}

fn trim(xml: &str, from: Date, to: Date) -> Result<String, String> {
    let feed_open = xml.find("<feed").ok_or("no <feed> element")?;
    let head_end = xml[feed_open..].find('>').ok_or("unterminated <feed>")? + feed_open + 1;

    let mut out = String::with_capacity(xml.len() / 8);
    out.push_str(&xml[..head_end]);

    let mut kept_blocks = 0usize;
    let mut rest = &xml[head_end..];
    while let Some(start) = rest.find("<entry>") {
        let Some(end) = rest[start..].find("</entry>") else {
            break;
        };
        let end = start + end + "</entry>".len();
        let entry = &rest[start..end];

        let keep = match block_day(entry)? {
            // Not an IntervalBlock: part of the link chain, always kept.
            None => true,
            Some(day) => {
                let keep = (from..=to).contains(&day);
                if keep {
                    kept_blocks += 1;
                }
                keep
            }
        };
        if keep {
            out.push_str(&rest[..end]);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);

    if kept_blocks == 0 {
        return Err(format!("no IntervalBlock covers {from} to {to}"));
    }
    eprintln!("kept {kept_blocks} interval blocks, {from} to {to}");
    Ok(out)
}

/// The local date an `IntervalBlock` entry covers, or `None` if the entry is not one.
fn block_day(entry: &str) -> Result<Option<Date>, String> {
    if !entry.contains("<espi:IntervalBlock") {
        return Ok(None);
    }
    // The block's own <espi:interval> comes before any reading's <espi:timePeriod>, so the first
    // <espi:start> in the entry is the day it covers.
    let at = entry
        .find("<espi:start>")
        .ok_or("an IntervalBlock has no start")?;
    let at = at + "<espi:start>".len();
    let end = entry[at..]
        .find("</espi:start>")
        .ok_or("unterminated start")?
        + at;
    let seconds: i64 = entry[at..end]
        .trim()
        .parse()
        .map_err(|e| format!("bad start: {e}"))?;
    let ts = Timestamp::from_second(seconds).map_err(|e| e.to_string())?;
    Ok(Some(local_date(ts)))
}
