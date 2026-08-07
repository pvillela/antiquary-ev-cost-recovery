# Maintenance manual

What a maintainer of this crate has to know that the code cannot tell them. Everything here is a
convention, an invariant nothing enforces, or a procedure — not an explanation of what a function
does, which belongs in its rustdoc.

This is not the user manual. It is for whoever changes the software, not whoever runs it.

## 1. Which constants are free, and which are derived

The electrical model lives in `src/site_load.rs`. Its constants fall into two groups, and the
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
`loading_ratio()`, and `BREAKER_RATING_KW` in `src/common.rs`, which is `ev_real_power_kw()` under
the name the rest of the crate uses.

### The rule the tests are written to

> **No test may depend on the numeric value of any freely-declared constant.**

Relationships between values may be relied on; the values may not.
`idle_transformer_draws_only_excitation` is the model: it asserts against `XFMR_NO_LOAD_LOSS_KW`
and `XFMR_MAGNETIZING_PU * XFMR_RATING_KVA`, not against 0.35 and 1.5.

This is enforced by review, not by tooling, so it is worth knowing the two places it is easy to
break by accident:

- **Fixture energy figures.** A CSV fixture states `Energy_Use` and `Active_Charge_Time` as fixed
  text, so whether the average power they imply clears `BREAKER_RATING_KW` depends on
  `BREAKER_RATING_A`. Lower the breaker rating and every fixture session starts picking up an
  `ExcessiveAvgKw` flag. This is why the timestamp tests in `src/excel.rs` filter their anomaly
  lists through `timing_anomalies` rather than asserting on them whole. If you add a test there
  that reads a whole anomaly list, filter it the same way.
- **Two constants read against each other.** `full_occupancy_stays_within_nameplate` does this on
  purpose: it asserts that `BREAKER_COUNT` vehicles do not exceed `XFMR_RATING_KVA`. That is a
  sizing invariant, not a number — a configuration violating it describes an installation that
  would trip — so the test failing is the correct outcome and the constants are what is wrong. It
  is the only deliberate instance; add another only with the same justification, and say so in the
  test's doc comment.

### Checking the rule still holds

Change one free constant, run the suite, and confirm only the golden-fixture tests fail:

```sh
# In src/site_load.rs, temporarily: BREAKER_RATING_A = 40.0 -> 32.0
cargo test
# Expect exactly three failures, all golden-file comparisons:
#   report_rendering::rendered_reports_match_their_golden_files
#   report_rendering::the_site_load_table_matches_its_golden_file
#   ev_peak_gui state::test::the_app_produces_the_same_report_as_the_command_line
# Then revert.
```

Anything else failing is a test that has acquired a dependency on the value, and should be
reformulated rather than updated.

## 2. Golden files

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

## 3. The `R` divides 15 minutes invariant

`TIME_GRID_STEP` — written `R`, currently 60 seconds — is the resolution at which the session
report states session start and end times. `SEGMENT_DURATION` is 15 minutes.

> **`R` must divide `SEGMENT_DURATION` without remainder.**

**Nothing enforces this.** There is no assertion, no `const` block, no test. If Evolute ever
reports seconds and `R` is changed to something that does not divide 15 minutes, segments will no
longer land on the time grid: session boundaries and segment boundaries will fall between each
other's ticks, and the overlap brackets will quietly stop meaning what they say.

If you change `TIME_GRID_STEP`, check this by hand. The candidates that work are the divisors of
900 seconds; the ones anyone would plausibly want are 1, 5, 10, 15, 30 and 60 seconds.

Changing `R` also moves `Adj_conn_end`, which is the reported end plus exactly one `R`, and the
half-width of the consistency band a sound record's `Conn_start + Conn_Duration` must land in.
Both follow automatically — that is why the constant exists — but both will move every figure in
the golden files.

## 4. Adding an `AnomalyKind`

`AnomalyKind` in `src/common.rs` classifies rows that need review. Adding a variant touches three
things, and deliberately not a fourth.

**The wire format.** `as_str` writes the variant name into the workbook's `Anomalies` column, and
`from_token` reads it back. These are a **stable wire format**, not display text: a workbook
written by one version is read by another, and an unrecognised token is a hard error rather than a
shrug. Add the variant to both, spelled identically, and never rename an existing one — a rename
makes every workbook already written unreadable.

`ExcessiveAvgPower` was renamed to `ExcessiveAvgKw` once, deliberately, at the same time as the
workbook's `Avg_power` column became `Avg_kw`. A workbook written before that carries the old token
and will now fail to read with an unrecognised-token error. That was judged acceptable because a
workbook is regenerated from its CSV in seconds and the CSV is the record of account — but it is
the cost the rule above exists to avoid, and it should not be paid twice.

**The prose.** `fmt::Display` carries the human wording, and it is free-form: reword it whenever it
reads badly. It is deliberately distinct from `as_str` for exactly that reason. The report's
glossary is generated from `Display`, so there is one wording to maintain rather than a second copy
in `report.rs`.

**Whether it excludes.** `InconsistentDuration` is the only kind that removes a session from the
estimates, and it does so where the buckets are sorted in `session_list`. Everything else is
informational: the session still counts towards every figure. If a new kind should exclude, that is
a decision to make explicitly and to record in README's "Other" section — not something that
follows from adding the variant.

**What you do *not* have to wire up:** `collect_session_anomalies` in `src/estimates.rs` matches on
nothing. It is deliberately blind to the kind, so a variant added here surfaces in the report
without anyone having to remember it. Keep it that way — the moment it grows a `match`, adding a
kind acquires a step that is easy to forget and silent when forgotten.

If the new kind is *about a figure* — as `ExcessiveAvgKw` is about average power — the figure
goes in the report cell, via `anomaly_cell` in `src/report.rs`, and not on the enum. That is what
keeps the workbook column a list of bare tokens `from_token` can read back.

## 5. Strict and lenient overlap tests

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

Both are pinned by tests in `src/estimates.rs`:
`the_strict_intersection_test_refuses_an_inverted_span` and
`an_inverted_span_is_answered_only_by_the_lenient_test`.

## 6. Where the rendering lives

`src/report.rs` is the crate's single rendering module. Both the interval report and the site-load
table are rendered there, and `examples/site_load_report.rs` is one `print!` over
`site_load_report()`.

This is not tidiness. A report saved from the GUI is byte-for-byte what the command line prints,
and README says so; that holds only because there is one rendering rather than two that could
drift. If you find yourself formatting a figure anywhere else — in a binary, in an example, in a
test helper — that is the thing to reconsider.
