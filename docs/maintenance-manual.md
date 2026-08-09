# Maintenance manual

What a maintainer of this crate has to know that the code cannot tell them. Everything here is a
convention, an invariant nothing enforces, or a procedure — not an explanation of what a function
does, which belongs in its rustdoc.

## 1. The port gate — run once, recorded here, then removed

The Rust implementation replaced a Python one. The question "does it reproduce what the Python
produced?" was answered once, by comparing full output against the workbook the Python had filled,
and the answer is recorded here rather than kept as a test.

| | |
|---|---|
| Date | 2026-08-09 |
| Code | commit `c9a8c46` |
| Input | `data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML` (18,018,534 bytes) |
| Reference | `docs/reference/Green_Button_Peak_Values-template.xlsx`, sha256 `6ea76c29efbcf4a613a659abf72efb35b6eb97c8fdb0e20a07cdd29ad1b2a5f0` |
| Method | `Peak_values` compared by **column name** over the shared subset, floats to 5e-7 |
| Scope | 21 billing periods × 19 shared columns = **399 cells** |
| Result | **0 mismatches** |

The 19 columns compared were `billing_period_ending`, `nbr_of_intervals`, `kwh`, and the four
groups `max_kw{,_interval,_interval_utc,_kva}`, `max_kw_nop{…}`, `max_kva{,_interval,_interval_utc,_kw}`,
`max_kva_nop{…}`. The comparison could not be wholesale: the Rust schema adds four `*_tou` columns
and an `anomalies` column, and renames every machine name to `lower_snake_case`.

To repeat it, generate the workbook and compare by column name against the reference. There is no
test to run — deliberately. The shared subset only shrinks as the Rust version diverges further
from the Python-era sheet, so a standing test would weaken over time while looking like it still
meant something.

The `docs/reference/` workbook stays as provenance: it is the artefact whose figures were
reconciled against real invoices, and whose June 2026 period ties out to one to the milli-kWh.
Nothing in the test suite reads it.

## 2. Golden files

Regenerate with:

```
UPDATE_GOLDEN=1 cargo test --test fixtures_golden
```

Then **read the diff before committing it**. That is the entire value of the mechanism. Regenerating
without reading turns them into a rubber stamp, and every rule this project encodes — which hours
are off-peak, which periods are complete — is the kind of thing that changes a number without
changing anything you would notice.

`Peak_values` is dumped whole; `Interval_values` is dumped as an excerpt plus per-column totals. The
totals are not decoration: they are what catches a change in the 780-odd rows the excerpt skips.

## 3. Invariants nothing enforces

**The demand window is the complement of TOU off-peak.** `is_off_peak` is defined as "every piece of
the partition is `Tou::OffPeak`", and the `_nop` columns use it to mean "inside Toronto Hydro's
`[07:00, 19:00)` demand window". These are two independent concepts — a distribution-charge
measurement window and a commodity pricing period — that currently coincide exactly. They would
come apart if Toronto Hydro changed its window, or if the OEB moved the 07:00 or 19:00 boundary.
There is a `debug_assert` that a `_nop` peak is never `OffPeak`, which would fire, but only in a
debug build and only if such a peak actually occurred.

**"Business days" is undefined.** Toronto Hydro uses the term for its demand window and does not
define it publicly — no page states whether statutory holidays are excluded from demand measurement.
That holidays *are* excluded is inherited from the Python and is unsourced. The two documents that
might settle it are the Conditions of Service PDF and the EB-2023-0195 Exhibit 8 rate-design filing.

**TOU boundaries must fall on whole hours.** Enforced by the type: `Schedule` is
`&[(u8, Tou)]`, so a half-hour boundary cannot be written. This is what guarantees that an
hour-long interval starting on the hour lies in exactly one price period, which in turn is what
lets `Peak::tou` be a `Tou` rather than an `Option<Tou>`. Do not change the hour to a finer type
without working out what happens to `tou_of` and to the workbook's TOU columns.

**No test may depend on a figure that only the sample data happens to have.** The interval counts
671, 720, 744 and 745 are exceptions and are deliberate: they are properties of the calendar, not of
this meter.

## 4. What would force a re-check of the TOU rules

The schedules in `src/tou.rs` are quoted verbatim from the OEB with the URL. They model the
**current** schedule and have no historical variation — a feed from 2020, when emergency flat
pricing was in force, would be silently mispriced. The rules have changed before, and ULO was added
as a separate plan in 2023.

Two things in that module are **implementation choices, not OEB policy**, because the OEB is silent
on both:

- The season changeover happens at local midnight on May 1 and November 1. The OEB gives the seasons
  only as calendar dates.
- Daylight saving needs no special handling. Transitions are at 02:00 local, every boundary is at
  07:00, 11:00, 17:00 or 19:00, and 02:00 is inside the off-peak block in both seasons. That is a
  consequence of the two rule sets, not something the OEB has ruled on.

To re-verify: open the OEB's holiday schedule and rates pages, check the ten holidays and the four
boundary hours against `src/holidays.rs` and `src/tou.rs`, and run `cargo test`. The 2026 published
table is pinned as a test, so a change to the schedule shows up as a failure rather than as a
slightly different number.

## 5. The Ontario holiday calendar is not the ESA list

`src/holidays.rs` implements the OEB's Time-of-Use schedule. The August **Civic Holiday** is on it
and is *not* an Employment Standards Act public holiday. Dropping it on the reasoning that it is not
statutory would reclassify a summer weekday's 07:00–19:00 block as on/mid-peak and can move a
monthly peak. The `civic_holiday` fixture exists to make that failure loud.

The ESA's substitute-day entitlement is negotiated per employee within a three- or twelve-month
window. It is not a calendar rule and cannot be computed; do not try.

## 6. Why umya-spreadsheet, and what still differs

`rust_xlsxwriter` was the first choice and was wrong. It models row heights and column widths as
**whole pixels** — `set_row_height` is `(height * 4.0 / 3.0).round() as u32`, stored back as
`0.75 x pixels` — so the reference workbook's 13.8pt rows, 12.8pt data rows, 23.85pt header and
1.39-wide spacers are not representable in it at all. Left unset, its default row height of 15pt
rendered every row at 0.53cm against the reference's 0.49.

`umya-spreadsheet` stores both as `f64` written straight through, so the reproduction is exact:
`defaultRowHeight` 13.8, heights 15 / 23.85 / 16.15 / 12.8, and every column width including the
1.39 spacers. It is also the crate `ev-peak-contrib` uses.

The general lesson, if the writer is ever swapped again: a crate that models a dimension in pixels
cannot reproduce a workbook authored in points, and the discrepancy will be small enough to look
like rounding noise rather than a wrong choice.

**Row heights must not be pinned.** `umya`'s `Row::set_height` also sets `customHeight`, which tells
the application the height was chosen deliberately and must not be auto-fitted. The reference
carries `customHeight="false"` on every row, so its rows are content-fitted. Setting the flag gives
the same stored numbers and a different rendered height — a difference that shows up only when
somebody opens both files and measures, which is how it was found. `set_row_height` in `excel.rs`
clears the flag for exactly this reason; do not call `Row::set_height` directly.

Two differences from the reference remain, both harmless:

- The reference writes an explicit `ht` on every row, including rows equal to the default, and
  writes `customHeight="false"` explicitly. Rows equal to the default are left to
  `defaultRowHeight` here, and the flag is omitted rather than written false. OOXML reads an absent
  `customHeight` as false, so both render identically.
- `openpyxl` warns "Workbook contains no default style" when reading the output: umya does not emit
  the default `cellStyleXfs` entry LibreOffice does. Excel and LibreOffice both open the file
  normally; only that one reader comments on it.

## 7. Alignment follows the column, not the row

The reference left-aligns column A throughout `Peak_values` — title, human header, machine name and
data alike — and centres every other column. On `Interval_values` column A is centred, but its
title is still left. So the rule encoded in `Kind::horizontal` is "alignment follows the column",
with the A1 title left on both sheets as a special case.

This was got wrong once, with the `billing_period_ending` header centred where the reference
left-aligns it. The golden dumps now record horizontal alignment for exactly that reason.

## 8. Regenerating the fixtures

```
cargo build --release --example trim_fixture
./target/release/examples/trim_fixture <FEED> 2025-07-23 2025-08-24 > tests/fixtures/civic_holiday.XML
./target/release/examples/trim_fixture <FEED> 2025-10-23 2025-11-24 > tests/fixtures/dst_fall.XML
./target/release/examples/trim_fixture <FEED> 2026-02-23 2026-03-24 > tests/fixtures/dst_spring.XML
./target/release/examples/trim_fixture <FEED> 2026-05-23 2026-06-24 > tests/fixtures/billed_period.XML
```

Each range is the target billing period plus a day of slack either side. The slack is required:
`IntervalBlock`s are anchored to 05:00 UTC, which is local midnight in winter but 01:00 in summer,
so a range never lines up with a period's local-midnight edges. The partial periods this leaves at
each end are not waste — they exercise the incomplete-period highlight.

Current fixture checksums:

```
fbe9571876b152d14d1cc12ed55720ef3866a5707236c62d55010b7525f93647  billed_period.XML
caed5b527036f746e9e829b476f54e33fb93354137d0bef4e18aa974a93c0c32  civic_holiday.XML
cc93001f05f65c94a2991628eeb1aac65a37d576cf3c0bb171a6bfb3d8c66751  dst_fall.XML
4de65d77fff82e9b5bcfc2accdace7d295b494ba09cb48263c1e34eca858c08b  dst_spring.XML
```

## 9. The invoice fixture

`tests/fixtures/invoice_2026_06.txt` carries figures transcribed from a real bill. It is the only
test whose expected values come from outside the software.

The account number, premises number, meter number, service address and property-management name are
deliberately absent, and the PDF is not in the repository. Keep it that way when adding another
invoice: none of that is needed to check a calculation.

Note which figures on an invoice are loss-factor adjusted. The kWh lines are; the demand lines are
not. The workbook reports raw meter values, so it is the unadjusted columns that should agree
directly, and the TOU energy buckets have to be divided by the loss factor before comparison. The
loss factor is deliberately **not** modelled — it is not in the Green Button data, it varies by rate
class, and it changes between rate applications, so hardcoding it would rot silently.
