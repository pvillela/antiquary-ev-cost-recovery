//! Markdown that also reads as plain text.
//!
//! Every report this crate writes goes to a terminal at least as often as it goes to a renderer, so
//! the two have to be the same document. Tables are padded to their column widths and prose is
//! wrapped by hand; nothing here needs a markdown engine to be legible.
//!
//! Crate-internal. What a report *says* belongs to the module that owns the figures; this is only
//! how it is laid out.

/// Width the prose is wrapped to. Comfortably inside 80 columns, leaving room for a quoting prefix
/// in an email reply.
const WRAP: usize = 76;

/// Column alignment for [`table`].
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Align {
    Left,
    Right,
}
pub(crate) use Align::{Left, Right};

/// A markdown pipe table, every cell padded to its column width so it also lines up in monospace.
pub(crate) fn table(headers: &[&str], rows: &[Vec<String>], align: &[Align]) -> String {
    let widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            rows.iter()
                .map(|r| r[i].chars().count())
                .chain(std::iter::once(h.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let line = |cells: &[String]| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .zip(align)
            .map(|((c, w), a)| {
                let pad = w.saturating_sub(c.chars().count());
                match a {
                    Left => format!("{c}{}", " ".repeat(pad)),
                    Right => format!("{}{c}", " ".repeat(pad)),
                }
            })
            .collect();
        format!("| {} |", padded.join(" | "))
    };

    // The alignment row carries the `:` markers so a renderer right-aligns the numbers too.
    let rule: Vec<String> = widths
        .iter()
        .zip(align)
        .map(|(w, a)| match a {
            Left => format!(":{}", "-".repeat(w + 1)),
            Right => format!("{}:", "-".repeat(w + 1)),
        })
        .collect();

    let header_cells: Vec<String> = headers.iter().map(|h| (*h).to_owned()).collect();
    let mut out = vec![line(&header_cells), format!("|{}|", rule.join("|"))];
    out.extend(rows.iter().map(|r| line(r)));
    out.join("\n")
}

/// Wraps `text` to [`WRAP`] columns on word boundaries, with `indent` prefixing every line after
/// the first. Long words are never broken, so an identifier stays intact even if it overruns.
pub(crate) fn wrap(text: &str, indent: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() {
            word.len()
        } else {
            cur.chars().count() + 1 + word.chars().count()
        };
        if !cur.is_empty() && candidate > WRAP {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(indent);
        }
        if !cur.is_empty() && !cur.ends_with(' ') && cur != *indent {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines.join("\n")
}

pub(crate) fn h1(s: &str) -> String {
    format!("{s}\n{}", "=".repeat(s.chars().count()))
}

pub(crate) fn h2(s: &str) -> String {
    format!("{s}\n{}", "-".repeat(s.chars().count()))
}
