# Python code (archived)

The original Python implementation, kept after the Rust rewrite. One script here is retired; the
other still has a job.

## `build_peak_values.py` — retired

Superseded by the `gb_peak_values` binary. It is kept because it is the provenance of every figure
in `Green_Button_Peak_Values-template.xlsx`, including the June 2026 period that reconciles against
a real Toronto Hydro invoice.

Worth knowing if you ever read it: it contains no `openpyxl` code despite the dependency below.
It edits the `xl/*.xml` members of the xlsx zip directly with `zipfile`, regex and string slicing,
because re-saving through a spreadsheet library rewrites `styles.xml` and loses the hand-applied
cell formatting. That constraint is what made it hard to change, and it is the reason the Rust
version builds a new workbook from scratch instead of updating one in place.

## `explore_model.py` — still current

A read-only diagnostic. It is the **only** consumer of `greenbutton-objects`, and it is what
generated `../../Toronto_Hydro_Object_Model.md`. Re-run it when the feed's shape changes — a new
export format, a different utility, an unexpected `ReadingType` — and regenerate that document from
its output.

It has two parts. `main()` walks the feed through the library and prints the UsagePoint →
MeterReading → ReadingType → IntervalBlock → IntervalReading hierarchy with the fields that matter
(`intervalLength`, `powerOfTenMultiplier`, `uom`). `raw_xml_diagnostics()` drops to `ElementTree`
for the two facts the library does not expose: the `LocalTimeParameters` resource, and the reading
counts on the two DST-transition days — which is what establishes that the feed sits on a fixed
05:00-UTC grid with 24 readings every day, including both transition days.

## Why `greenbutton-objects` still matters

`src/espi.rs` parses the ESPI feed by hand. This library is the independent second opinion: when
the hand-rolled parse and a third-party ESPI implementation agree on the same file, a
misunderstanding of the format is unlikely to be the explanation for a wrong number.

## Running it

```
cd docs/archived/python
uv run explore_model.py
```

`XML_FILE` is a bare filename with no directory component, and it was already stale before this
directory moved. Either edit the constant to a path that resolves from here, or run from a
directory where the export sits alongside the script.

The `.venv` created by an earlier `uv` run is still at the repository root and is git-ignored;
`uv` will make a new one here.

## Dependencies

| Package | Status |
|---|---|
| `greenbutton-objects` | Used by `explore_model.py`. See above. |
| `holidays` | Used only by the retired script. The Rust version implements the OEB TOU holiday schedule directly instead — the library never shifts Canada Day even when July 1 falls on a Sunday, and its `PUBLIC` / `OPTIONAL` categories are coupled such that dropping the Civic Holiday also drops correct Boxing Day substitutes. |
| `openpyxl` | Declared but **never imported** by anything here. |
