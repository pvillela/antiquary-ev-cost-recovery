# Maintenance manual

What a maintainer of this crate has to know that the code cannot tell them. Everything here is a
convention, an invariant nothing enforces, or a procedure — not an explanation of what a function
does, which belongs in its rustdoc.

This is not the user manual. It is for whoever changes the software, not whoever runs it.

**One manual, three parts.** `sessions` and `green_button` had a manual each before they were one
crate. Shared material is stated once, under Shared; the rest is under the module it belongs to.

**Cite sections by title, never by number.** Numbering was how the old manuals were referenced, and
inserting a section renumbered every citation after it without breaking a build or failing a test.
Titles are stable; if one changes, the citation fails to find it and says so.

## Contents

### Shared

- Golden files
- Which constants are free, and which are derived
- Boundaries and the time grid
- A generated workbook is not byte-reproducible

### Sessions

- Adding an `AnomalyKind`
- Strict and lenient overlap tests
- Where the rendering lives

### Green Button

- Invariants nothing enforces
- What would force a re-check of the TOU rules
- The Ontario holiday calendar is not the ESA list
- Why umya-spreadsheet, and what still differs
- Row heights: three, and only three
- Alignment follows the column, not the row
- Regenerating the fixtures
- The invoice fixture
- The port gate — run once, recorded here, then removed

---

# Shared

## Golden files

Both modules pin output against committed files that a person reads before accepting a change.
Regenerating without reading the diff turns them into a rubber stamp.

| What | Regenerate with |
|---|---|
| Session reports and the site-load table | `UPDATE_REPORT_GOLDEN=1 cargo test --test integration -- session::report_rendering` |
| Green Button fixture dumps and the committed standard workbook | `UPDATE_GOLDEN=1 cargo test --test integration -- green_button::fixtures_golden` |

Test binaries were consolidated into `tests/integration.rs` when the two projects merged, so
`--test <file>` no longer selects anything. The form above names the binary and then filters by
module path.

### From the sessions module


Three files are pinned byte for byte, all under `tests/fixtures/`:

- `Session_Report_Diagram.report.md`
- `Session_Report_Anomalies.report.md`
- `site_load.report.txt` — the site-load table; `.txt` because it is fixed-width plain text with no
  markdown in it, and naming it otherwise would invite someone to render it

Regenerate all of them with one command:

```sh
UPDATE_REPORT_GOLDEN=1 cargo test --test report_rendering
```

Then **read the diff before committing it**. That is the entire value of the mechanism: the files
exist so that a change in wrapping, padding, column order or a figure shows up somewhere a human
looks. Regenerating without reading turns them into a rubber stamp.

What to check in the diff:

- **A figure moved that you did not expect to move.** Every number in these files is downstream of
  the estimating logic and the electrical model. If you were changing wording and a kW figure
  shifted, something else changed too.
- **Column widths.** Every table row must be the same width, and no line may exceed 90 columns.
  Both are asserted, but the assertion tells you *that* it broke, and the diff tells you which
  column grew.
- **Nothing that only a markdown renderer would show.** No four-space indents, no `#` headings, no
  backticks, no bold markers. The report has to read as plain text, and these files are what ships.

These files are the **one deliberate exception** to the rule in §1. They pin *rendering* — column
widths, decimal places, wrapping — and no relational reformulation preserves any of that. Changing
an electrical constant is therefore expected to fail exactly these and nothing else, which is what
makes the check in §1 meaningful.

### From the green_button module


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

## Which constants are free, and which are derived


The electrical model lives in `src/session/site_load.rs`. Its constants fall into two groups, and the
distinction matters because it decides what may be changed and what will follow.

**Free constants** — declared outright. Change any of them to describe a different installation:

| Constant | Meaning |
|---|---|
| `PANEL_VOLTAGE_V` | Secondary line-to-line voltage |
| `BREAKER_RATING_A` | Rating of each EVSE branch breaker |
| `CONTINUOUS_DUTY_DERATE` | Continuous-load derating (CEC Rule 8-104) |
| `BREAKER_COUNT` | Number of EVSE breakers, which bounds the vehicle count |
| `EV_TRUE_POWER_FACTOR` | True power factor of a vehicle's onboard charger at full current |
| `EV_CURRENT_THD` | Total harmonic distortion of that charger's input current |
| `XFMR_RATING_KVA` | Transformer nameplate |
| `XFMR_NO_LOAD_LOSS_KW` | Core loss, constant whenever energised |
| `XFMR_FULL_LOAD_LOSS_KW` | Copper loss at rated load |
| `XFMR_MAGNETIZING_PU` | Magnetizing current, per unit of rating |
| `XFMR_REACTANCE_PU` | Leakage reactance, per unit of rating |

**Derived values** — computed from the free ones, never declared. Do not edit these to a literal;
edit the constant behind them. `ev_pilot_current_a()`, `ev_apparent_power_kva()`,
`ev_real_power_kw()`, `max_true_power_factor()`, `ev_load()`, `transformer_load()`, `site_load()`,
`loading_ratio()`, and `BREAKER_RATING_KW` in `src/session/common.rs`, which is `ev_real_power_kw()` under
the name the rest of the crate uses.

#### The rule the tests are written to

> **No test may depend on the numeric value of any freely-declared constant.**

Relationships between values may be relied on; the values may not.
`idle_transformer_draws_only_excitation` is the model: it asserts against `XFMR_NO_LOAD_LOSS_KW`
and `XFMR_MAGNETIZING_PU * XFMR_RATING_KVA`, not against 0.35 and 1.5.

This is enforced by review, not by tooling, so it is worth knowing the two places it is easy to
break by accident:

- **Fixture energy figures.** A CSV fixture states `Energy_Use` and `Active_Charge_Time` as fixed
  text, so whether the average power they imply clears `BREAKER_RATING_KW` depends on
  `BREAKER_RATING_A`. Lower the breaker rating and every fixture session starts picking up an
  `ExcessiveAvgKw` flag. This is why the timestamp tests in `src/session/csv.rs` and
  `src/session/excel.rs` filter their anomaly lists through `timing_anomalies`
  (`src/session/test_support.rs`) rather than asserting on them whole. If you add a test in either
  that reads a whole anomaly list, filter it the same way.
- **Two constants read against each other.** `full_occupancy_stays_within_nameplate` does this on
  purpose: it asserts that `BREAKER_COUNT` vehicles do not exceed `XFMR_RATING_KVA`. That is a
  sizing invariant, not a number — a configuration violating it describes an installation that
  would trip — so the test failing is the correct outcome and the constants are what is wrong. It
  is the only deliberate instance; add another only with the same justification, and say so in the
  test's doc comment.

#### Checking the rule still holds

Change one free constant, run the suite, and confirm only the golden-fixture tests fail:

```sh
# In src/session/site_load.rs, temporarily: BREAKER_RATING_A = 40.0 -> 32.0
cargo test
# Expect exactly three failures, all golden-file comparisons:
#   session::report_rendering::rendered_reports_match_their_golden_files
#   session::report_rendering::the_site_load_table_matches_its_golden_file
#   ev_cost_recovery state::test::the_app_produces_the_same_report_as_the_command_line
# Then revert.
```

Anything else failing is a test that has acquired a dependency on the value, and should be
reformulated rather than updated.

## Boundaries and the time grid


`TIME_GRID_STEP` — written `R`, currently 60 seconds — is the resolution at which the session
report states session start and end times. `SEGMENT_DURATION` is 15 minutes.

> **`R` must divide `SEGMENT_DURATION` without remainder.**

**Nothing enforces this.** There is no assertion, no `const` block, no test. If Evolute ever
reports seconds and `R` is changed to something that does not divide 15 minutes, segments will no
longer land on the time grid: session boundaries and segment boundaries will fall between each
other's ticks, and the overlap brackets will quietly stop meaning what they say.

If you change `TIME_GRID_STEP`, check this by hand. The candidates that work are the divisors of
900 seconds; the ones anyone would plausibly want are 1, 5, 10, 15, 30 and 60 seconds.

Changing `R` also moves `adj_conn_end`, which is the reported end plus exactly one `R`, and the
half-width of the consistency band a sound record's `Conn_start + Conn_Duration` must land in.
Both follow automatically — that is why the constant exists — but both will move every figure in
the golden files.

### If the session report starts stating seconds

`TIME_GRID_STEP` is **ours**, not Evolute's, and it does not follow theirs down. Every allowance
this software makes for truncated reporting is that one value: the padding that gives
`adj_conn_end`, and the width of the window `duration_is_consistent` accepts.

Should Evolute begin reporting seconds, the allowances become wider than the data needs — sessions
get a padded end they no longer require, and the consistency window admits records it could now
reject. Nothing crashes and no figure looks wrong, which is why the conversion says so out loud:
the run log carries one line naming the count and the first three offending rows —
`<stem>.convert.log` when the CSV is converted to a workbook, `<stem>.csv.read.log` when it is read
straight to sessions. Both come from the same parse, so both say it.

The fix is **not** to set `TIME_GRID_STEP` to one second. Not because one second is an illegal
grid — it divides 15 minutes and `LEGAL_START_MINUTES` still lands on it — but because this
constant is global while the reporting resolution belongs to a report. During the changeover,
minute-resolution and second-resolution reports are processed together, and no single global value
is right for both; it has to stay at the coarsest resolution in scope. Once every report in scope
reports seconds, that constraint lifts and moving the grid becomes a real option.

If you do move it to one second, update `docs/session/README.md`, "Boundaries and the time grid".
That section describes the padding in terms of whole minutes — a session reported to end at `16:34`
ending somewhere in `[16:34:00, 16:35:00)` — and states the divisibility rule against 15 minutes.
Both go stale the moment `R` changes.

## A generated workbook is not byte-reproducible

Converting the same input twice with the same binary gives two `.xlsx` files that differ.
`xl/styles.xml` lists the same `numFmt` entries, with the same ids, in a different order — it is
`HashMap` iteration order inside umya-spreadsheet. Every worksheet is byte-identical; only the style
table moves.

So **never compare workbook bytes or hashes**. Both golden suites dump sheet contents as text for
this reason, and that is also why the committed standard workbook cannot be checked by comparing it
to a fresh one.

`Cargo.lock` is committed so that a figure in a workbook can be traced to the code that produced it.
That still holds for the figures. It does not hold for the file.

---

# Sessions

## Adding an `AnomalyKind`


`AnomalyKind` in `src/session/common.rs` classifies rows that need review. Adding a variant touches three
things, and deliberately not a fourth.

**The wire format.** `as_str` writes the variant name into the workbook's `anomalies` column, and
`from_token` reads it back. These are a **stable wire format**, not display text: a workbook
written by one version is read by another, and an unrecognised token is a hard error rather than a
shrug. Add the variant to both, spelled identically, and never rename an existing one — a rename
makes every workbook already written unreadable.

**The prose.** `fmt::Display` carries the human wording, and it is free-form: reword it whenever it
reads badly. It is deliberately distinct from `as_str` for exactly that reason. The report's
glossary is generated from `Display`, so there is one wording to maintain rather than a second copy
in `report.rs`.

**Whether it excludes.** `InconsistentDuration` is the only kind that removes a session from the
estimates, and it does so where the buckets are sorted, in `SessionReport::new`. Everything else is
informational: the session still counts towards every figure. If a new kind should exclude, that is
a decision to make explicitly and to record in README's "Other" section — not something that
follows from adding the variant.

**What you do *not* have to wire up:** `collect_session_anomalies` in `src/session/peak.rs` matches on
nothing. It is deliberately blind to the kind, so a variant added here surfaces in the report
without anyone having to remember it. Keep it that way — the moment it grows a `match`, adding a
kind acquires a step that is easy to forget and silent when forgotten.

If the new kind is *about a figure* — as `ExcessiveAvgKw` is about average power — the figure
goes in the report cell, via `anomaly_cell` in `src/session/report.rs`, and not on the enum. That is what
keeps the workbook column a list of bare tokens `from_token` can read back.

## Strict and lenient overlap tests


`Session::intersects` has a **precondition**: `adj_conn_end` must not precede `conn_start`. It
panics otherwise, and that is deliberate.

Nothing legitimate violates it. `conn_duration` is unsigned, so the soundness test's
`conn_start + conn_duration < adj_conn_end` cannot hold unless `conn_start < adj_conn_end` — an
inverted session is therefore always flagged `InconsistentDuration` and sorted into
`SessionReport::excluded`, and `interval_estimates` never puts an excluded session in front of the
estimating logic. Reaching the panic means one got somewhere it should not have, which is worth a
crash rather than a plausible-looking answer.

`Session::lenient_intersects` reads the two endpoints in whichever order puts them the right way
round, and answers instead of panicking.

> **Only the reporting module may call `lenient_intersects`, and only for the Excluded sessions
> listing.**

That listing covers the whole workbook by design, so it has to say whether a contradictory record
*appears* to touch the interval. Nothing else has any business holding such a record. If you find
yourself wanting the lenient test somewhere new, the question to ask first is how an excluded
session reached that code.

Both are pinned by tests in `src/session/peak.rs`:
`the_strict_intersection_test_refuses_an_inverted_span` and
`an_inverted_span_is_answered_only_by_the_lenient_test`.

## Where the rendering lives


`src/session/report.rs` is the crate's single rendering module. Both the interval report and the site-load
table are rendered there, and `examples/site_load_report.rs` is one `print!` over
`site_load_report()`.

This is not tidiness. A report saved from the GUI is byte-for-byte what the command line prints,
and README says so; that holds only because there is one rendering rather than two that could
drift. If you find yourself formatting a figure anywhere else — in a binary, in an example, in a
test helper — that is the thing to reconsider.

---

# Green Button

## Invariants nothing enforces


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
672, 720 and 744 are exceptions and are deliberate: they are properties of the calendar, not of this
meter.

**The billing period boundary is on standard time, everything else on prevailing local time.** The
boundary is 00:00 EST year-round; Time-of-Use periods, the 07:00–19:00 demand window and the holiday
calendar follow the clocks. `standard_midnight` and `local_midnight` in `src/time/base.rs` are the
two, and they must not be merged — a summer period cut on the wrong one is an hour out at each end.
The counts 671 and 745 are what that error used to produce, and their absence is now a signal:
seeing either again means the boundary has drifted back to prevailing time.

## What would force a re-check of the TOU rules


The schedules in `src/time/tou.rs` are quoted verbatim from the OEB with the URL. They model the
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
boundary hours against `src/time/holidays.rs` and `src/time/tou.rs`, and run `cargo test`. The 2026 published
table is pinned as a test, so a change to the schedule shows up as a failure rather than as a
slightly different number.

## The Ontario holiday calendar is not the ESA list


`src/time/holidays.rs` implements the OEB's Time-of-Use schedule. The August **Civic Holiday** is on it
and is *not* an Employment Standards Act public holiday. Dropping it on the reasoning that it is not
statutory would reclassify a summer weekday's 07:00–19:00 block as on/mid-peak and can move a
monthly peak. The `civic_holiday` fixture exists to make that failure loud.

The ESA's substitute-day entitlement is negotiated per employee within a three- or twelve-month
window. It is not a calendar rule and cannot be computed; do not try.

## Why umya-spreadsheet, and what still differs


`rust_xlsxwriter` was the first choice and was wrong. It models row heights and column widths as
**whole pixels** — `set_row_height` is `(height * 4.0 / 3.0).round() as u32`, stored back as
`0.75 x pixels` — so the reference workbook's 13.8pt rows, 12.8pt data rows, 23.85pt header and
1.39-wide spacers are not representable in it at all. Left unset, its default row height of 15pt
rendered every row at 0.53cm against the reference's 0.49.

`umya-spreadsheet` stores both as `f64` written straight through. Every column width, including the
1.39 spacers, reproduces exactly. It is also the crate `ev-peak-contrib` uses.

The general lesson, if the writer is ever swapped again: a crate that models a dimension in pixels
cannot reproduce a workbook authored in points, and the discrepancy will be small enough to look
like rounding noise rather than a wrong choice.

`openpyxl` warns "Workbook contains no default style" when reading the output: umya does not emit
the default `cellStyleXfs` entry LibreOffice does. Excel and LibreOffice both open the file
normally; only that one reader comments on it.

## Row heights: three, and only three


The whole workbook stores three row heights:

| Sheet | Row | Height | Why |
|---|---|---|---|
| both | *sheet default* | 13.8 | the body font, Arial 10 |
| `Peak_values` | 1 | 15 | title, Arial 12 bold |
| `Peak_values` | 3 | 24 | two wrapped lines of Arial 10 bold |
| `Interval_values` | 1 | 15 | title, Arial 12 bold |

Everything else — the blank row, the machine-name row, and all 13,896 data rows — has no stored
height and takes `defaultRowHeight`.

**The rule to keep:** a row either has a *pinned* height because somebody chose it, or it has no
stored height at all. Never the half-state — a stored height the application is free to re-fit.
That middle case is what made two files with identical stored numbers render at different heights,
and it cost three rounds of chasing to find, because every XML comparison said they matched.

The reference workbook stamps a height on all 13,924 of its rows. That is what LibreOffice writes,
not a decision anyone made, and reproducing it was a mistake: it buries the three heights that are
chosen among thousands that are not. Two figures in the reference are also accidents worth not
copying — its `Interval_values` data rows are 12.8 where `Peak_values` uses 13.8 for the same font
and content, and its two sheet titles are different sizes. Both are unified here.

Row 3 of `Peak_values` is the one genuinely content-dependent height: 24pt fits two wrapped lines at
the current column widths. Change a column width enough that a header collapses to one line or needs
three, and it wants revisiting.

## Alignment follows the column, not the row


The reference left-aligns column A throughout `Peak_values` — title, human header, machine name and
data alike — and centres every other column. On `Interval_values` column A is centred, but its
title is still left. So the rule encoded in `Kind::horizontal` is "alignment follows the column",
with the A1 title left on both sheets as a special case.

This was got wrong once, with the `billing_period_ending` header centred where the reference
left-aligns it. The golden dumps now record horizontal alignment for exactly that reason.

## Regenerating the fixtures


`<FEED>` is the full export, `data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML`.

```
cargo build --release --example gb_trim_fixture
./target/release/examples/gb_trim_fixture <FEED> 2025-07-23 2025-08-24 > tests/fixtures/green_button/civic_holiday.XML
./target/release/examples/gb_trim_fixture <FEED> 2025-10-23 2025-11-24 > tests/fixtures/green_button/dst_fall.XML
./target/release/examples/gb_trim_fixture <FEED> 2026-02-23 2026-03-24 > tests/fixtures/green_button/dst_spring.XML
./target/release/examples/gb_trim_fixture <FEED> 2026-05-23 2026-06-24 > tests/fixtures/green_button/billed_period.XML
```

Each range is the target billing period plus a day of slack either side. The slack is required:
`gb_trim_fixture` takes local dates, while `IntervalBlock`s are anchored to 05:00 UTC — midnight EST
— so a range never lines up exactly with a period's edges. The partial periods this leaves at each
end are not waste: they exercise the incomplete-period highlight.

That 05:00 UTC anchor is the feed keeping a permanent midnight-EST day, which is the same boundary
the billing period now uses. See `green_button/Toronto_Hydro_Object_Model.md`, "Fixed daily grid".

Current fixture checksums:

```
fbe9571876b152d14d1cc12ed55720ef3866a5707236c62d55010b7525f93647  billed_period.XML
caed5b527036f746e9e829b476f54e33fb93354137d0bef4e18aa974a93c0c32  civic_holiday.XML
cc93001f05f65c94a2991628eeb1aac65a37d576cf3c0bb171a6bfb3d8c66751  dst_fall.XML
4de65d77fff82e9b5bcfc2accdace7d295b494ba09cb48263c1e34eca858c08b  dst_spring.XML
```

## The invoice fixture


`tests/fixtures/green_button/invoice_2026_06.txt` carries figures transcribed from a real bill. It is the only
test whose expected values come from outside the software.

The account number, premises number, meter number, service address and property-management name are
deliberately absent, and the PDF is not in the repository. Keep it that way when adding another
invoice: none of that is needed to check a calculation.

Note which figures on an invoice are loss-factor adjusted. The kWh lines are; the demand lines are
not. The workbook reports raw meter values, so it is the unadjusted columns that should agree
directly, and the TOU energy buckets have to be divided by the loss factor before comparison. The
loss factor is deliberately **not** modelled — it is not in the Green Button data, it varies by rate
class, and it changes between rate applications, so hardcoding it would rot silently.

## The port gate — run once, recorded here, then removed


The Rust implementation replaced a Python one. The question "does it reproduce what the Python
produced?" was answered once, by comparing full output against the workbook the Python had filled,
and the answer is recorded here rather than kept as a test.

| | |
|---|---|
| Date | 2026-08-09 |
| Code | commit `c9a8c46` |
| Input | `data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML` (18,018,534 bytes) |
| Reference | `docs/green_button/reference/Green_Button_Peak_Values-python-2026-07-16.xlsx`, sha256 `6ea76c29efbcf4a613a659abf72efb35b6eb97c8fdb0e20a07cdd29ad1b2a5f0` |
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

### Re-run, 2026-08-19

The gate was run again after the merger review, at commit `184f467`, against the same reference and
the same input. Two changes to method, both widening it:

- `Interval_values` was compared as well as `Peak_values`. The original gate covered only the peak
  sheet, and the interval sheet is where every figure the peaks are drawn from lives.
- Every shared cell was compared, not the grid: the count below is lower than 399 because 8 cells
  are **blank in both** workbooks, one billing period having no non-peak maxima. Agreement includes
  the blanks.

| | |
|---|---|
| Code | commit `184f467` |
| `Peak_values` | 21 periods × 19 shared columns, 391 non-blank cells |
| `Interval_values` | 13,896 rows × 5 shared columns, 69,480 cells |
| Total compared | **69,871 cells** |
| Result | **0 mismatches** |
| Largest absolute difference | 1.0e-10 on `Peak_values`, 3.6e-11 on `Interval_values` — float text formatting, not arithmetic |

So everything from the merger through the four review phases left the figures where the Python put
them. That is a stronger statement than baseline parity, which only says this work changed nothing:
this says the numbers agree with an implementation outside the crate, whose own figures were
reconciled against real invoices.

The `docs/green_button/reference/` workbook stays as provenance: it is the artefact whose figures
were reconciled against real invoices, and whose June 2026 period ties out to one to the milli-kWh.
Nothing in the test suite reads it, and its **formatting is not the current standard** — see
"Row heights: three, and only three".

Three workbooks, three jobs, no overlap:

| Path | What | Committed | Read by code |
|---|---|---|---|
| `docs/green_button/reference/Green_Button_Peak_Values-python-2026-07-16.xlsx` | the Python-era output | yes | never |
| `tests/fixtures/green_button/billed_period.xlsx` | the current formatting standard | yes | regenerated with the goldens |
| `data/*.xlsx` | whatever you last generated | no, ignored | no |

The standard lives under `tests/fixtures/` rather than beside the reference precisely because the
`UPDATE_GOLDEN=1` run regenerates it. A committed workbook that nothing regenerates goes stale and
then misleads about which file is authoritative, which is worse than not having one.
