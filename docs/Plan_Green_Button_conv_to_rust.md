# Green Button: Python → Rust conversion

## Context

`build_peak_values.py` reads a Toronto Hydro Green Button ESPI XML export and fills a
hand-formatted two-tab workbook with billing-period peak values. It works, but it is built around
an awkward constraint: to preserve the workbook's formatting it edits the xlsx zip's XML members
with `zipfile` + regex rather than a spreadsheet library, and it updates an existing template
in place. That machinery exists only to protect formatting, and it is the main reason the script
is hard to change.

The Rust rewrite drops in-place updating entirely and builds the workbook from scratch, which
removes the surgical-XML-editing problem at its root. It also aligns this project's conventions
with `../ev-peak-contrib`, ahead of eventually consolidating the two.

The design was validated against a real Toronto Hydro invoice (`bak/TH_5728140000_2026_06_29.pdf`,
Jun 29 2026) before any code was written. For the May 24 – Jun 23 2026 period, the existing
spreadsheet reproduces the bill's `Demand kW`, `Peak kW 7-7` and `Demand kVA` exactly, and the
TOU rule reproduces the bill's on-peak and mid-peak kWh **to the milli-kWh** once the 1.0295 loss
factor is divided out. The residual 11.16 kWh (0.014%) falls entirely in off-peak, consistent with
a meter read a few minutes either side of local midnight. The domain rules below are therefore
verified, not assumed.

## Scope

Functional changes from the Python:

- Create a new workbook each run; no in-place update, no incremental backfill.
- Output path derived from the input: same directory, same stem, `.xlsx` instead of `.XML`.
- **Refuse to overwrite** an existing output file — exit non-zero naming the file.
- Red-fill (`#FFC7CE`) any `nbr_of_intervals` that is not a full billing period's worth, and any
  non-empty `anomalies` cell.
- New `anomalies` column on both tabs.
- New TOU column at the end of each of the four value groups on `Peak_values`.
- All machine-readable column names normalised to `lower_snake_case`.

## Project layout

Cargo project at the repo root, crate `green-button`, edition 2024, mirroring `ev-peak-contrib`'s
lib-façade + `src/bin/` shape (`lib.rs` holds only `mod x; pub use x::*;`).

| Module | Purpose |
|---|---|
| `src/common.rs` | `Reading`, series types, `TIME_ZONE_NAME`, Excel-serial helpers |
| `src/espi.rs` | ESPI XML → three time series |
| `src/holidays.rs` | Ontario OEB holiday set (`pub mod`, not glob re-exported) |
| `src/tou.rs` | `enum Tou`, `tou_partition`, per-date schedule |
| `src/billing.rs` | Period boundaries, expected-interval count, demand-window predicate |
| `src/peaks.rs` | The four max searches per period |
| `src/excel.rs` | Workbook writing, driven by a `const COLUMNS` table |
| `src/bin/gb_peak_values.rs` | CLI |

Lift `ev-peak-contrib`'s `const COLUMNS: &[(&str, Source)]` pattern (`src/excel.rs`) as the single
source of truth for header rows, data rows, widths and number formats — the 30-column layout with
its spacer columns is exactly what it is for.

## Dependencies

- `umya-spreadsheet` — stores row heights and column widths as `f64` written straight through,
  which is what makes exact reproduction of the reference workbook possible. `rust_xlsxwriter` was
  tried first and rejected: it models both as whole pixels and cannot represent the reference's
  13.8pt rows or 1.39-wide spacers.
- `roxmltree` — whole-document tree; the two-pass href join wants random access.
- `jiff` 0.2 — matching `ev-peak-contrib`.
- `ev-peak-contrib` — **path dependency**, GUI stack included for now (no `[features]` there yet;
  refine later).
  It also reads xlsx back, which the golden-dump tests need.

Errors follow the house split: `Box<dyn Error>` on I/O paths, `Result<T, String>` for validation.
No logging crate; `eprintln!` from `src/bin/` only.

## Domain rules (also the README's content)

- **Billing period** — 24th of previous month 00:00 local to 23rd 24:00 local, `America/Toronto`;
  labelled by the 23rd. Confirmed against the bill's `MAY 23 2026 TO JUN 23 2026 / 31 days`.
- **Expected interval count** — elapsed UTC hours between the two local-midnight boundaries.
  Reproduces the observed 720 / 744 / **671** (spring-forward) / **745** (fall-back) exactly.
  Leading and trailing partial periods are highlighted; that is intended.
- **Holidays** — implement the **OEB TOU schedule** directly, not the ESA list and not a crate.
  Ten algorithmic dates (Jan 1; 3rd Mon Feb; Good Friday via Gregorian computus; Mon before May 25;
  Jul 1; 1st Mon Aug **Civic Holiday**; 1st Mon Sep; 2nd Mon Oct; Dec 25; Dec 26) plus the OEB's
  published substitution rule applied literally — *"if a holiday falls on a weekend, the next
  weekday (that is not also a holiday)"* — additive, keeping the weekend day. Print the applied
  list to stderr each run.
- **TOU periods** — boundaries as hour integers, so a half-hour boundary is unrepresentable:

  ```
  weekend or holiday        →  [(0, OffPeak)]
  weekday, May 1 – Oct 31   →  [(0,Off), (7,Mid),  (11,On),  (17,Mid), (19,Off)]
  weekday, Nov 1 – Apr 30   →  [(0,Off), (7,On),   (11,Mid), (17,On),  (19,Off)]
  ```

  Quote the OEB wording verbatim in the doc comment with its URL. Season changeover at local
  midnight and the absence of DST special-casing are **implementation choices** (the OEB is silent
  on both) and must be documented as such. Scope limit for the README: this models the current
  schedule with no historical variation.
- **Demand window** — `[07:00, 19:00)` on business days; `is_off_peak(i) ⟺ tou(i) == Tou::OffPeak`,
  derived from `tou_partition` so there is one predicate and one calendar. Assert the resulting
  invariant: a `_nop` row's TOU value is never `OffPeak`. Note in the manual that TH never defines
  "business days" publicly — excluding holidays from *demand* measurement is inherited assumption.
- **Arithmetic** — all sums and maxima on raw source integers; divide by `10^(3-powerOfTen)` only
  at cell-write time. Ties won by the **earliest** interval.

`pub fn tou_partition(interval: Interval) -> Vec<(Tou, Interval)>` (by value; `Interval` is `Copy`).
Guarantees, rustdoc'd and unit-tested: chronological, contiguous, union equals the input, no
zero-length elements, adjacent same-`Tou` pieces **merged** (maximal). Empty input → empty `Vec`.

## ESPI parsing

Proper link traversal, not the Python's id-token shortcut: `ReadingType` self-href →
`(uom, powerOfTenMultiplier, intervalLength)`; `MeterReading` self-href → its `rel="related"`
ReadingType href; `IntervalBlock` self-href with `/IntervalBlock/<n>` stripped → MeterReading href.
The Python's shortcut only works because TH happens to give both the same token. Series keyed on
`uom` (72 → kWh, 38 → kW, 61 → kVA); `kind` is not a discriminator (kWh and kVA are both `12`).

Hard-fail on: `intervalLength != 3600`, `timePeriod/duration != 3600`, a missing series, a broken
link. Build rows from the **union** of the three timestamp sets — the Python iterates kWh only, so
it cannot detect a missing kWh at all.

## Workbook schema

`Peak_values` — title row 1, blank row 2, human headers row 3, machine names row 4, data from
row 5, descending by period. 30 columns:

```
A  billing_period_ending   Billing period ending          yyyy/mm/dd
B  nbr_of_intervals        Number of intervals            #,##0        ← red fill when != expected
C  (spacer 1.39)
D  kwh                     kWh used                       #,##0.000
E  (spacer 1.39)
F  max_kw                  Demand kW                      #,##0.000
G  max_kw_interval         Demand kW interval (local time)  yyyy/mm/dd hh:mm ddd
H  max_kw_interval_utc     Demand kW interval (UTC)         yyyy/mm/dd hh:mm
I  max_kw_kva              kVA at interval                #,##0.000
J  max_kw_tou              TOU                            text
K  (spacer)
L–P   max_kw_nop{,_interval,_interval_utc,_kva,_tou}       Peak kW 7-7 …
Q  (spacer)
R–V   max_kva{,_interval,_interval_utc}, max_kva_kw, max_kva_tou   Demand kVA … / kW at interval
W  (spacer)
X–AB  max_kva_nop{,_interval,_interval_utc}, max_kva_nop_kw, max_kva_nop_tou   Peak kVA 7-7 …
AC (spacer)
AD anomalies                Anomalies                     text         ← red fill when non-empty
```

`Interval_values` — title row 1, blank row 2, headers row 3, data from row 4, descending by
`interval_utc`: `interval` (20.45), `interval_utc` (18.44), `kwh`, `kw`, `kva` (11.38 each — fixes
the reference's missing D/E widths), `anomalies`.

Formatting copied from `bak/Green_Button_Peak_Values-template.xlsx`: Arial throughout; bold 12 /
bold 13 titles; bold 10 row-3 headers; bold 7 row-4 machine names; freeze `B4` on both sheets;
number formats `yyyy/mm/dd`, `#,##0`, `#,##0.000`, `yyyy/mm/dd\ hh:mm\ ddd`, `yyyy/mm/dd\ hh:mm`;
no borders, no autofilter, no conditional formatting. Human headers keep the bill's wording
verbatim (`Peak kW 7-7`, `Demand kW`) — `U3` reads `kW at interval`, correcting the reference.

## Anomalies

```rust
enum Anomaly { MissingKwh, MissingKw, MissingKva, MissingInterval, DuplicateInterval, MisalignedInterval }
```

`as_str` is a **stable wire format** — documented, never renamed, since these sheets get read back
by name. No DST variants: the source is UTC.

- `Interval_values` — comma-delimited per row. A `MissingInterval` gets a **placeholder row**:
  timestamp present, three value cells blank, red-filled. Both rows of a duplicate pair are flagged.
- `Peak_values` — comma-delimited with counts, e.g. `MissingKw(2),MissingInterval(3)`.
- A `MisalignedInterval` is **excluded from peak selection**, so it can never be the peak and the
  TOU cell is always well-defined; its billing period is red-flagged.

## CLI

`gb_peak_values <XML>` — one positional argument, no flags, `USAGE` const, `fn main() -> ExitCode`,
`-h`/`--help` handled manually, per `ev-peak-contrib`'s binaries. Prints the written path, the
applied holiday list, and anomaly counts to stderr.

## Testing

Fast tier (default `cargo test`) — four purpose-named ESPI fixtures, each one or two billing
periods, each with a golden text dump (sheet, cell ref, value, number format, fill) regenerated
under `UPDATE_GOLDEN=1`:

| Fixture | Proves |
|---|---|
| `civic_holiday` (Jul–Aug 2025) | the OEB-vs-ESA holiday divergence — the riskiest rule |
| `dst_fall` (Oct–Nov 2025) | the 745-interval period |
| `dst_spring` (Feb–Mar 2026) | the 671-interval period |
| `billed_period` (May–Jun 2026) | ties out against the invoice |

Trimming must drop whole `<entry>` elements while keeping `LocalTimeParameters`, `UsagePoint`,
`ReadingType` and `MeterReading`, so each fixture is still valid ESPI. **Commit the trimming
script** — fixtures must be regenerable.

Bill fixture: a small tracked text file of extracted figures with account number, premises number,
address, meter number **stripped**; the PDF itself is not committed. Asserts `max_kw`,
`max_kw_nop`, `max_kva` exactly; `kwh` within a tolerance covering the observed 11.16 kWh; and,
in-test only, TOU buckets against the bill divided by 1.0295 — on-peak and mid-peak to ~0.001 kWh,
off-peak loose.

Loss factor is **not** modelled; the README states the sheet reports raw meter values matching the
bill's unadjusted columns.

One-time port gate: compare full `data/*.XML` output against
`docs/reference/Green_Button_Peak_Values-python-2026-07-16.xlsx` **by column name over the shared subset**
— it cannot match structurally, since the schema gains TOU and anomalies columns and renames every
machine name. Record in `docs/maintenance-manual.md`: columns compared, row counts, tolerances,
the commit SHA that passed, and fixture checksums. **Then delete the test**; the template remains
at `docs/reference/` as provenance only. The current formatting standard is a separate,
regenerated artifact: `tests/fixtures/billed_period.xlsx`.

## Housekeeping

- Move to `docs/archived/python/` (alongside the existing `manual-python.md`):
  `build_peak_values.py`, `explore_model.py`, `pyproject.toml`, `uv.lock`, `.python-version`.
  Add `docs/archived/python/README.md` explaining the ongoing roles of `greenbutton-objects` and
  `explore_model.py` (the only consumer of that library, and the source of
  `docs/Toronto_Hydro_Object_Model.md`).
- Delete `main.py` (a `uv init` stub).
- Move `docs/Prompt_Green_Button_conv_to_rust.md` to `docs/archived/` once executed.
- Note: `openpyxl` is a declared Python dependency that nothing imports.

## Docs

- `README.md` — user-facing, plus the full domain rules above (the spec, previously only in
  git-ignored `bak/Prompt_Green_Button_peak_values.md`).
- `docs/maintenance-manual.md` — invariants nothing enforces, procedures, the one-time gate record,
  the OEB re-verification procedure, and the "business days" caveat.
- `python/README.md` — as above.

## Verification

1. `cargo fmt --check`, `cargo clippy --all-targets` clean, `cargo build --all-targets` with no
   warnings.
2. `cargo test` — four fixture pairs plus unit tests (holiday dates 2024–2028 against the OEB
   table; `tou_partition` guarantees; expected-interval counts for 671/720/744/745).
3. `cargo test -- --ignored` — the one-time full-data gate, before it is deleted.
4. `gb_peak_values data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML`, then open
   `data/TH_Electric_Usage_23-11-2024_to_24-06-2026.xlsx` and compare against
   `tests/fixtures/billed_period.xlsx` by eye for formatting.
5. Re-run the same command and confirm it **refuses** rather than overwriting.
6. Confirm the June 2026 row matches the invoice: `max_kw` 153.119, `max_kw_nop` 152.639,
   `max_kva` 183.359, `kwh` ≈ 77,281.6.
