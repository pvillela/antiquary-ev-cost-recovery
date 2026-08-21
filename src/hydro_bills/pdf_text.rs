//! Text out of a PDF, with the position of every run on the page.
//!
//! A bill is a table without table markup: what makes `1,761.07` the distribution charge is that
//! it sits on the same baseline as the words `Distribution Charges`, and what separates a charge
//! from the marketing column beside it is that the two are at different horizontal positions.
//! Plain-text extraction throws both facts away, so this keeps them: each run of text is reported
//! at the point where it starts, and runs sharing a baseline are grouped into a [`Line`].
//!
//! What this deliberately does not do is lay out glyphs. Only the starting point of each shown
//! string is computed, never the width it occupies, so no font metrics are needed. That is enough
//! to read a form whose fields are separately positioned, which is what these bills are.
//!
//! `lopdf` supplies the objects, the streams and the decompression; the text is decoded here.
//! Its own `get_font_encoding` rejects the CMaps in these bills -- its grammar wants whitespace
//! where Toronto Hydro's generator writes `/CMapName/Adobe-Identity-UCS` and `>>def` -- and,
//! worse, reports the rejection as a successful fall back to StandardEncoding. That turns every
//! glyph into the wrong character while still returning `Ok`, which is exactly the failure a bill
//! parser must not have. `ToUnicode` below reads the same CMaps and returns an error when it
//! cannot.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::io::{self, Write};
use std::path::Path;

use lopdf::content::Operation;
use lopdf::{Document, Object, ObjectId};

/// How far apart two baselines may be and still count as one row, in points.
///
/// A label and its value are not always emitted at exactly the same baseline -- Toronto Hydro's
/// generator places `Statement Date` at y 883.0 and the date itself at y 882.5 -- so exact
/// equality would split rows that read as one. The tolerance has to stay well under the tightest
/// real line spacing on these bills, which is about 2.3 points between `Due Date` and the
/// `Meter Number` label below it.
const ROW_TOLERANCE: f64 = 1.5;

/// The identity transform, as PDF writes matrices: `[a b c d e f]`.
const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// One run of shown text, at the point where it starts, in PDF user space.
///
/// User space has y increasing upwards, so a larger `y` is higher on the page.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub x: f64,
    pub y: f64,
    pub text: String,
}

/// The fragments that share a baseline, ordered left to right.
#[derive(Debug, Clone)]
pub struct Line {
    pub y: f64,
    pub fragments: Vec<Fragment>,
}

impl Line {
    /// The line's text with the fragments joined by a single space, trimmed.
    pub fn text(&self) -> String {
        self.fragments
            .iter()
            .map(|f| f.text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The fragments that start left of `x`.
    ///
    /// Bills put a marketing column beside the charges, at its own horizontal position. Cutting
    /// the page at a vertical line is what keeps that column out of the charge rows it lands
    /// between.
    pub fn left_of(&self, x: f64) -> Vec<&Fragment> {
        self.fragments.iter().filter(|f| f.x < x).collect()
    }
}

/// Writes what [`read_pages`] found, page by page, with the position of every run.
///
/// This is what a reader falls back on when a parse fails. A parser over these pages finds its
/// figures by the words beside them and by where on the page they sit, so a failure means the
/// words or the positions are not what it expected -- and neither is visible in the PDF or in
/// plain-text extraction. Rendering the coordinates alongside the text is what makes the
/// difference something a person can see.
///
/// One line per baseline, its `y` first and then its runs as `x:text`:
///
/// ```text
/// ===== page 1 =====
/// [y=  883.0] 45:Statement Date  |  120:Jul 28 2025
/// [y=  520.1] 45:Distribution Charges  |  327:$1,761.07
/// ```
pub fn write_pages(pages: &[Vec<Line>], out: &mut impl Write) -> io::Result<()> {
    for (number, page) in pages.iter().enumerate() {
        writeln!(out, "===== page {} =====", number + 1)?;
        for line in page {
            let runs: Vec<String> = line
                .fragments
                .iter()
                .map(|f| format!("{:.0}:{}", f.x, f.text.trim()))
                .collect();
            writeln!(out, "[y={:7.1}] {}", line.y, runs.join("  |  "))?;
        }
    }
    Ok(())
}

/// Reads a PDF and returns its lines, page by page, each page ordered top to bottom.
pub fn read_pages(path: &Path) -> Result<Vec<Vec<Line>>, Box<dyn Error>> {
    let doc = Document::load(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut pages = Vec::new();
    for (number, id) in doc.get_pages() {
        let fragments = page_fragments(&doc, id)
            .map_err(|e| format!("{}: page {number}: {e}", path.display()))?;
        pages.push(into_lines(fragments));
    }
    Ok(pages)
}

/// Every run of text on one page, in the order the content stream shows it.
fn page_fragments(doc: &Document, page: ObjectId) -> Result<Vec<Fragment>, Box<dyn Error>> {
    let mut encodings: BTreeMap<Vec<u8>, ToUnicode> = BTreeMap::new();
    for (name, font) in doc.get_page_fonts(page)? {
        let named = || String::from_utf8_lossy(&name).into_owned();
        let stream = font
            .get_deref(b"ToUnicode", doc)
            .and_then(Object::as_stream)
            .map_err(|e| format!("font /{}: no ToUnicode CMap: {e}", named()))?;
        let cmap = stream.get_plain_content()?;
        let cmap = String::from_utf8_lossy(&cmap);
        let cmap = ToUnicode::parse(&cmap)
            .map_err(|e| format!("font /{}: unreadable ToUnicode CMap: {e}", named()))?;
        encodings.insert(name, cmap);
    }
    let content = doc.get_and_decode_page_content(page)?;

    let mut fragments = Vec::new();
    let mut ctm = IDENTITY;
    let mut ctm_stack: Vec<[f64; 6]> = Vec::new();
    // The text matrix and the text line matrix, which `Td`, `TD` and `T*` advance relative to.
    let mut tm = IDENTITY;
    let mut tlm = IDENTITY;
    let mut leading = 0.0;
    let mut encoding: Option<&ToUnicode> = None;

    for Operation { operator, operands } in &content.operations {
        match operator.as_str() {
            "q" => ctm_stack.push(ctm),
            "Q" => ctm = ctm_stack.pop().unwrap_or(IDENTITY),
            "cm" => {
                if let Some(m) = matrix(operands) {
                    ctm = multiply(&m, &ctm);
                }
            }
            "BT" => {
                tm = IDENTITY;
                tlm = IDENTITY;
            }
            "Tf" => {
                encoding = operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .and_then(|name| encodings.get(name));
            }
            "TL" => leading = number(operands.first()),
            "Td" | "TD" => {
                let (tx, ty) = (number(operands.first()), number(operands.get(1)));
                if operator == "TD" {
                    leading = -ty;
                }
                tlm = multiply(&[1.0, 0.0, 0.0, 1.0, tx, ty], &tlm);
                tm = tlm;
            }
            "Tm" => {
                if let Some(m) = matrix(operands) {
                    tlm = m;
                    tm = m;
                }
            }
            "T*" => {
                tlm = multiply(&[1.0, 0.0, 0.0, 1.0, 0.0, -leading], &tlm);
                tm = tlm;
            }
            // The three that show a string. `'` and `"` move to the next line first; `"` also
            // takes two spacing operands ahead of the string, so the string is always last.
            "Tj" | "'" | "\"" => {
                if operator != "Tj" {
                    tlm = multiply(&[1.0, 0.0, 0.0, 1.0, 0.0, -leading], &tlm);
                    tm = tlm;
                }
                let text = decode(operands.last(), encoding);
                push(&mut fragments, &tm, &ctm, text);
            }
            // An array of strings interleaved with kerning offsets. The offsets nudge glyphs
            // within the run; the run as a whole still starts where the text matrix says.
            "TJ" => {
                let text = operands
                    .first()
                    .and_then(|o| o.as_array().ok())
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| decode(Some(item), encoding))
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                push(&mut fragments, &tm, &ctm, text);
            }
            _ => {}
        }
    }
    Ok(fragments)
}

/// Records a fragment at the origin of the text space, unless it is blank.
fn push(fragments: &mut Vec<Fragment>, tm: &[f64; 6], ctm: &[f64; 6], text: String) {
    if text.trim().is_empty() {
        return;
    }
    let m = multiply(tm, ctm);
    fragments.push(Fragment {
        x: m[4],
        y: m[5],
        text,
    });
}

/// Decodes one string operand through the font's CMap. Anything else yields nothing.
fn decode(operand: Option<&Object>, encoding: Option<&ToUnicode>) -> String {
    let (Some(Object::String(bytes, _)), Some(encoding)) = (operand, encoding) else {
        return String::new();
    };
    encoding.decode(bytes)
}

/// What a font's glyph codes mean, read from its `/ToUnicode` CMap.
///
/// The fonts in these bills are subset CID fonts: a code in a shown string is an index into the
/// subset, not a character, so without this table `Bill Print Date` reads as `%LOO3ULQW'DWH`.
struct ToUnicode {
    /// How many bytes one code takes, from the CMap's codespace range.
    code_len: usize,
    text: HashMap<u32, String>,
}

impl ToUnicode {
    /// The text a shown string stands for, one code at a time.
    ///
    /// A code the CMap does not cover becomes U+FFFD, so a font this does not fully understand
    /// shows up as visible damage in whatever line it lands on rather than as a quietly shortened
    /// label.
    fn decode(&self, bytes: &[u8]) -> String {
        bytes
            .chunks_exact(self.code_len)
            .map(|code| {
                let code = code.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b));
                match self.text.get(&code) {
                    Some(text) => text.as_str(),
                    None => "\u{FFFD}",
                }
            })
            .collect()
    }

    /// Reads the `bfchar` and `bfrange` mappings out of a CMap.
    ///
    /// Only the mapping sections are looked at. The PostScript around them -- the procset
    /// preamble, the `/CIDSystemInfo` dictionary, the `defineresource` epilogue -- carries nothing
    /// this needs, and insisting on its exact shape is what makes a CMap reader brittle.
    fn parse(cmap: &str) -> Result<ToUnicode, Box<dyn Error>> {
        let code_len = sections(cmap, "begincodespacerange", "endcodespacerange")
            .into_iter()
            .flat_map(|section| hex_tokens(section).into_iter())
            .find_map(|token| match token {
                // Two hex digits to the byte: `<0000>` declares two-byte codes.
                Token::Hex(hex) => Some(hex.chars().filter(|c| !c.is_whitespace()).count() / 2),
                _ => None,
            })
            .ok_or("no codespace range")?;
        if code_len == 0 || code_len > 4 {
            return Err(format!("codespace range of {code_len} bytes").into());
        }

        let mut text = HashMap::new();
        for section in sections(cmap, "beginbfchar", "endbfchar") {
            let tokens = hex_tokens(section);
            for pair in tokens.chunks(2) {
                let [Token::Hex(code), Token::Hex(target)] = pair else {
                    return Err("bfchar entry is not a pair of hex strings".into());
                };
                text.insert(code_of(code), utf16(target));
            }
        }
        for section in sections(cmap, "beginbfrange", "endbfrange") {
            Self::read_range(&hex_tokens(section), &mut text)?;
        }
        Ok(ToUnicode { code_len, text })
    }

    /// `<lo> <hi> <first>`, where the target ascends with the code, or `<lo> <hi> [<a> <b> ...]`,
    /// where each code has its own target.
    fn read_range(tokens: &[Token], text: &mut HashMap<u32, String>) -> Result<(), Box<dyn Error>> {
        let mut rest = tokens;
        while let [Token::Hex(lo), Token::Hex(hi), tail @ ..] = rest {
            let (lo, hi) = (code_of(lo), code_of(hi));
            if hi < lo {
                return Err("bfrange runs backwards".into());
            }
            rest = match tail {
                [Token::Hex(first), tail @ ..] => {
                    // The last UTF-16 unit of the target counts up with the code; a range that
                    // would overrun it is malformed rather than something to wrap around.
                    let mut units = utf16_units(first);
                    let last = units.pop().ok_or("bfrange target is empty")?;
                    for (step, code) in (lo..=hi).enumerate() {
                        let step = u16::try_from(step).map_err(|_| "bfrange too long")?;
                        let unit = last.checked_add(step).ok_or("bfrange target overflows")?;
                        let target: Vec<u16> = units.iter().copied().chain([unit]).collect();
                        text.insert(code, String::from_utf16_lossy(&target));
                    }
                    tail
                }
                [Token::Open, tail @ ..] => {
                    let end = tail
                        .iter()
                        .position(|t| matches!(t, Token::Close))
                        .ok_or("bfrange target list is not closed")?;
                    for (code, target) in (lo..=hi).zip(&tail[..end]) {
                        let Token::Hex(target) = target else {
                            return Err("bfrange target list holds something else".into());
                        };
                        text.insert(code, utf16(target));
                    }
                    &tail[end + 1..]
                }
                _ => return Err("bfrange entry has no target".into()),
            };
        }
        Ok(())
    }
}

/// A hex string, or a bracket around a list of them.
#[derive(Debug, PartialEq, Eq)]
enum Token<'a> {
    Hex(&'a str),
    Open,
    Close,
}

/// The text between each `begin` and the `end` that follows it.
fn sections<'a>(cmap: &'a str, begin: &str, end: &str) -> Vec<&'a str> {
    let mut sections = Vec::new();
    let mut rest = cmap;
    while let Some(from) = rest.find(begin) {
        rest = &rest[from + begin.len()..];
        let to = rest.find(end).unwrap_or(rest.len());
        sections.push(&rest[..to]);
        rest = &rest[to..];
    }
    sections
}

/// The hex strings and brackets in one mapping section. Everything else is skipped.
fn hex_tokens(section: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut rest = section;
    while let Some(at) = rest.find(['<', '[', ']']) {
        let tail = &rest[at..];
        match &tail[..1] {
            "[" => {
                tokens.push(Token::Open);
                rest = &tail[1..];
            }
            "]" => {
                tokens.push(Token::Close);
                rest = &tail[1..];
            }
            _ => match tail.find('>') {
                Some(close) => {
                    tokens.push(Token::Hex(&tail[1..close]));
                    rest = &tail[close + 1..];
                }
                None => break,
            },
        }
    }
    tokens
}

/// A hex string read as a code. Digits that are not hex count as zero, which only ever widens a
/// code that was already malformed.
fn code_of(hex: &str) -> u32 {
    hex.chars()
        .filter(|c| !c.is_whitespace())
        .fold(0u32, |acc, c| (acc << 4) | c.to_digit(16).unwrap_or(0))
}

/// A hex string read as UTF-16 big-endian code units, the form a CMap states targets in.
fn utf16_units(hex: &str) -> Vec<u16> {
    let digits: Vec<u32> = hex
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_digit(16).unwrap_or(0))
        .collect();
    digits
        .chunks(4)
        .map(|unit| unit.iter().fold(0u16, |acc, d| (acc << 4) | *d as u16))
        .collect()
}

fn utf16(hex: &str) -> String {
    String::from_utf16_lossy(&utf16_units(hex))
}

/// The six operands of `cm` or `Tm`, if that is what they are.
fn matrix(operands: &[Object]) -> Option<[f64; 6]> {
    let [a, b, c, d, e, f] = operands else {
        return None;
    };
    Some([
        number(Some(a)),
        number(Some(b)),
        number(Some(c)),
        number(Some(d)),
        number(Some(e)),
        number(Some(f)),
    ])
}

fn number(operand: Option<&Object>) -> f64 {
    match operand {
        Some(Object::Integer(i)) => *i as f64,
        Some(Object::Real(r)) => *r as f64,
        _ => 0.0,
    }
}

/// `m` followed by `n`, in PDF's row-vector convention.
fn multiply(m: &[f64; 6], n: &[f64; 6]) -> [f64; 6] {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

/// Groups fragments into rows by baseline, top of the page first, each row left to right.
fn into_lines(mut fragments: Vec<Fragment>) -> Vec<Line> {
    fragments.sort_by(|a, b| b.y.total_cmp(&a.y));

    let mut lines: Vec<Line> = Vec::new();
    for fragment in fragments {
        match lines.last_mut() {
            Some(line) if (line.y - fragment.y).abs() <= ROW_TOLERANCE => {
                line.fragments.push(fragment);
            }
            _ => {
                lines.push(Line {
                    y: fragment.y,
                    fragments: vec![fragment],
                });
            }
        }
    }
    for line in &mut lines {
        line.fragments.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    lines
}

#[cfg(test)]
mod test {
    use super::*;

    fn at(x: f64, y: f64, text: &str) -> Fragment {
        Fragment {
            x,
            y,
            text: text.to_string(),
        }
    }

    #[test]
    fn baselines_within_the_tolerance_become_one_row_read_left_to_right() {
        let lines = into_lines(vec![
            at(500.0, 882.5, "Jul 28 2025"),
            at(367.0, 883.0, "Statement Date"),
        ]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text(), "Statement Date Jul 28 2025");
    }

    #[test]
    fn baselines_further_apart_than_the_tolerance_stay_separate_rows() {
        let lines = into_lines(vec![
            at(367.0, 846.0, "Due Date"),
            at(64.0, 848.5, "Meter Number"),
        ]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text(), "Meter Number");
        assert_eq!(lines[1].text(), "Due Date");
    }

    #[test]
    fn rows_come_back_top_of_the_page_first() {
        let lines = into_lines(vec![at(64.0, 100.0, "lower"), at(64.0, 700.0, "upper")]);
        assert_eq!(
            lines.iter().map(Line::text).collect::<Vec<_>>(),
            ["upper", "lower"]
        );
    }

    #[test]
    fn left_of_drops_the_column_beside_the_charges() {
        let lines = into_lines(vec![
            at(64.0, 500.0, "Transmission Connection Charge"),
            at(
                394.0,
                500.0,
                "Seeing double? You're not being charged twice.",
            ),
        ]);
        let kept = lines[0].left_of(360.0);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "Transmission Connection Charge");
    }

    #[test]
    fn a_dump_carries_the_position_of_every_run_beside_its_text() {
        let pages = vec![
            into_lines(vec![
                at(45.0, 883.0, "Statement Date"),
                at(120.4, 882.5, " Jul 28 2025 "),
            ]),
            into_lines(vec![at(64.0, 500.0, "Customer Charges")]),
        ];
        let mut out = Vec::new();
        write_pages(&pages, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "===== page 1 =====\n\
             [y=  883.0] 45:Statement Date  |  120:Jul 28 2025\n\
             ===== page 2 =====\n\
             [y=  500.0] 64:Customer Charges\n"
        );
    }

    /// The shape Toronto Hydro's generator writes, cut down to four glyphs. Note `/CMapName` and
    /// its value run together, and `>>def` has no space in it: this is the syntax `lopdf`'s own
    /// CMap grammar rejects.
    const CMAP: &str = "\
/CIDInit/ProcSet findresource begin
12 dict begin begincmap
/CMapName/Adobe-Identity-UCS def
/CIDSystemInfo<</Registry(Adobe)/Ordering(UCS)/Supplement 0>>def
/CMapType 2 def
1 begincodespacerange
<0000><FFFF>
endcodespacerange
4 beginbfchar
<0003><0020>
<0025><0042>
<0048><0065>
<008E><00A8>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop end end";

    #[test]
    fn a_subset_font_reads_back_as_the_characters_it_stands_for() {
        let cmap = ToUnicode::parse(CMAP).unwrap();
        assert_eq!(cmap.code_len, 2);
        assert_eq!(cmap.decode(&[0x00, 0x25, 0x00, 0x03, 0x00, 0x48]), "B e");
    }

    #[test]
    fn a_code_the_cmap_does_not_cover_is_visible_damage_rather_than_a_gap() {
        let cmap = ToUnicode::parse(CMAP).unwrap();
        assert_eq!(cmap.decode(&[0x00, 0x25, 0x01, 0x23]), "B\u{FFFD}");
    }

    #[test]
    fn a_bfrange_counts_its_target_up_with_the_code() {
        let cmap = ToUnicode::parse(
            "1 begincodespacerange\n<0000><FFFF>\nendcodespacerange\n\
             1 beginbfrange\n<0013><0015><0030>\nendbfrange\n",
        )
        .unwrap();
        assert_eq!(cmap.decode(&[0x00, 0x13, 0x00, 0x14, 0x00, 0x15]), "012");
    }

    #[test]
    fn a_bfrange_may_instead_list_a_target_per_code() {
        let cmap = ToUnicode::parse(
            "1 begincodespacerange\n<00><FF>\nendcodespacerange\n\
             1 beginbfrange\n<01><03>[<0041><0042><0043>]\nendbfrange\n",
        )
        .unwrap();
        assert_eq!(cmap.code_len, 1);
        assert_eq!(cmap.decode(&[0x01, 0x03, 0x02]), "ACB");
    }

    #[test]
    fn a_cmap_without_a_codespace_range_is_an_error_rather_than_a_guess() {
        assert!(ToUnicode::parse("4 beginbfchar\n<0003><0020>\nendbfchar\n").is_err());
    }

    #[test]
    fn a_translation_lands_where_the_flipped_page_transform_puts_it() {
        // Toronto Hydro's generator flips the page with `1 0 0 -1 0 1008 cm`, then writes text at
        // distances measured down from the top. A baseline 124.988 down is 883.012 up.
        let flip = [1.0, 0.0, 0.0, -1.0, 0.0, 1008.0];
        let text = [1.0, 0.0, 0.0, 1.0, 63.576, 124.988];
        let m = multiply(&text, &flip);
        assert!((m[4] - 63.576).abs() < 1e-9);
        assert!((m[5] - 883.012).abs() < 1e-9);
    }
}
