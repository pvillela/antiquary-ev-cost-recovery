# Simplification: corrections and completion

## Context

The specs and core code were drastically simplified on the premise that billed peak values are
averages over 15-minute intervals, so power is now averaged over `Segment`s rather than computed
over variable-length session groups. `src/grouping.rs` and `src/quicksort.rs` were deleted,
`Segment`/`Bracket`/`IntervalEstimates` introduced, and 27 tests removed.

The simplification is not finished. **The crate does not compile**: `src/report.rs` is still
`mod`-declared at `lib.rs:13` but imports six types deleted in the rewrite, so none of the 78
declared tests run. The new segment core carries three defects, one of which panics. The README's
estimation section describes formulas the code does not implement. This plan closes all of it.

Design decisions settled during review are recorded inline as **[D]** notes so the reasoning
survives.

---

## Stage 1 — Core correctness

Numeric output changes in this stage and nowhere else, which is why it is isolated.

**Three bugs:**

- `src/common.rs:143` — `Session::intersects` returns `overlap.is_empty()`; missing `!`, so it is
  inverted. Both callers (`estimates.rs:130`, `:182`) read it as a true intersection test, so each
  segment collects exactly the sessions that *miss* it.
- `src/estimates.rs:124` — `Segment::new(ioi_start + ioi_dur * i, ...)` strides by the whole
  interval instead of `SEGMENT_DURATION`. A 1-hour IOI at 16:00 yields segments at 16:00, 17:00,
  18:00, 19:00. Use `SEGMENT_DURATION * i as u32`.
- `src/estimates.rs:159` — `hi_crit` initialises to `0.0` rather than `criterion(first)`, so the
  first segment loses to any later positive one. Add an explicit earliest-segment tie-break so
  output is deterministic. **[D]** Ranking stays on `Bracket::mid()`.

**Remove the sentinel-overlap panic path.** `SessionOverlap::empty()` builds `(MAX,MAX)/(MIN,MIN)`,
and `duration()` then calls `duration(MAX, MIN)`, which panics (`common.rs:44`). Change
`Session::interval_overlap` to return `Option<SessionOverlap>`, map `None` to `Bracket::exact(0.0)`
in `interval_overlap_ratio`, and delete `SessionOverlap::{new, empty}`. **[D]** `segments_for_ioi`
still filters on `intersects`, so non-intersecting sessions stay out of `Segment.sessions` — the
`Option` is the type-level backstop, not the filter.

**Retire the `avg_kw` field in favour of the method.** `Session::avg_kw()` (`common.rs:167`) now
absorbs the spike substitution. Delete the `avg_kw` field (`common.rs:133`), its population at
`excel.rs:858`, and the now-dead spike mutation block at `estimates.rs:81-88`. Call sites become
`avg_kw()`: `common.rs:215`, `examples/sessions.rs:66`. `ExcessiveAvgPower` detection
(`excel.rs:337`) recomputes locally from `CsvSession` and is unaffected. **[D]** The `spikes` bucket
survives as a reporting-only category; README needs no change, since where the substitution happens
is an implementation detail.

**Drop the `RefCell`.** With that mutation gone there is no `borrow_mut` anywhere in the crate.
`RSession` becomes `Rc<Session>` (`common.rs:94`); drop the five `.borrow()` calls and the
`RefCell` imports (`common.rs:4`, `estimates.rs:6`, `:94`).

**Delete unreferenced items:** `RSegment` (`common.rs:392`).

---

## Stage 2 — Compile-green

**Rewrite `src/report.rs` from scratch.** It is 1032 lines and ~40 helpers built for the group
model. **[D]** Keep only the formatting primitives verbatim — `table`, `wrap`, `h1`, `h2`, `mmss`,
`local`, `hms`, `ymd_hm` — and write the rest fresh. Its 7 tests go with the old body.

New section order and content:

| Section | Content |
|---|---|
| Header | Source, interval, length |
| Estimates | The maximal segment(s): four rows (`Energy-based` / `Count-based` × kW/kVA) with `min` and `max` columns, plus a `Segment` column naming the winning quarter by clock time (`16:15`) |
| Segments | All segments per **[D]** — `agg_count` and `agg_kw` brackets only, as `min-max` in one cell |
| Segment membership | Session ids per segment |
| Excluded sessions | Every excluded session in the workbook, with an `In interval` yes/no column |
| Anomalies | Row, session, `In interval`, kind |

**[D]** Row labels are `Energy-based` / `Count-based`, matching README and code exactly; the old
`Consumption-based` / `Breaker-spec-based` glosses move into the prose under the table.
**[D]** Brackets are always displayed in full, never as midpoints.
**[D]** With skew margins out, the four-valued `Window` column collapses to a two-valued
`In interval`, computed as `Session::intersects(ioi)` on the reported `conn_start..adj_conn_end`
span. Keep the existing `ExcessiveAvgPower(6.900)` value-in-parens rendering — it derives from
session data, not constants.
**[D]** Empty-interval branch: `report.rs:221-230` currently says "nothing to estimate", which is
now false — `site_load(0)` is the standing block that README:130 requires reporting. Print the
normal Estimates table at standing-block values plus a sentence explaining no session intersected.

**Add `pub fn site_load_report() -> String`** to `report.rs`, re-exported from `lib.rs`, holding the
render currently inlined as fifteen `println!`s in `examples/site_load_report.rs`. **[D]** `report.rs`
becomes the crate's single rendering module — the "one rendering rather than two that could drift"
principle already stated at `report.rs:178`. The example reduces to `print!("{}", site_load_report())`.

**Unify the interval type.** `checked_interval` (`interval.rs:192`) returns `(Timestamp, Timestamp)`;
`interval_estimates` takes `Interval`. **[D]** Change the producer, not the three consumers
(`ev_estimate_cli.rs:73`, `ev_peak_gui/state.rs:299`, `tests/report_rendering.rs:66`) —
`Interval::from_start_end` already exists, and README:62 makes the point that the boundary rules
live in one place. Adjust the nine unit tests' assertions.

**Repair the GUI Estimate tab.** `ev_peak_gui/estimate.rs:7,231` imports and takes `PowerEstimates`,
which is gone. **[D]** `headline` shows four bracketed figures — energy-based kW/kVA and count-based
kW/kVA of their respective maximal segments — plus the winning quarter's clock time. Drop the
`has_margins` parameter and the skew `Section` branch (`state.rs:444`, `:723-726`).

**Fix `examples/site_load_report.rs`'s imports** to `site_load::` paths. **[D]** No glob re-export —
`site_load` is deliberately namespaced and flattening would put eleven physics constants in the
crate root.

**Delete outside `docs/`:** `tests/session_grouping_diagram.rs`,
`tests/fixtures/Session_Report_SkewMargin.{csv,report.md}`, and the `SkewMargin` entry in
`report_rendering.rs`'s `CASES`. **[D]** Nothing under `docs/` is deleted — that cleanup is the
owner's, and dangling references in stale docs are out of scope.

Only after this stage will `cargo build`/`clippy` reveal the real dead-code and unused-import
inventory; sweep it then, along with the stale rustdoc links at `common.rs:120`, `interval.rs:3`,
`estimates.rs:40`, `common.rs:569`, `excel.rs:765`.

---

## Stage 3 — Tests

**[D] Governing rule: no test may depend on the numeric value of any freely-declared constant.**
Any constant in the crate that is not defined in terms of other constants must be changeable without
breaking a test. Relationships between values in `site_load` may be relied on; the values may not.
`idle_transformer_draws_only_excitation` (`site_load.rs:251`) is the model — it asserts against
`XFMR_NO_LOAD_LOSS_KW` and `XFMR_MAGNETIZING_PU * XFMR_RATING_KVA`, not 0.35 and 1.5.

**Existing violations in `src/site_load.rs`:**

- `vehicle_apparent_power_is_current_limited` (`:211`) asserts `≈ 6.656`. **[D] Delete it** — the
  only value-free reformulation restates the function body, and tautological tests are not worth
  their space.
- `site_power_factor_rises_then_plateaus` (`:259`) asserts `≈ 0.944` and `≈ 0.982`. Keep the
  relational `single < plateau`, delete both numeric assertions, add that the plateau does not
  exceed `max_true_power_factor()`.
- `full_occupancy_stays_within_nameplate` (`:268`) depends on `BREAKER_COUNT` and `XFMR_RATING_KVA`.
  **[D] Keep it, with a comment marking it a deliberate design guard** — it pins a sizing invariant,
  not a number, and a config that violates it describes an installation that would trip.

**New `estimates.rs` tests.** **[D]** Build the numeric ones on the identity *a segment fully covered
by N sessions each drawing exactly `ev_load().real_kw` yields
`energy_based_load == count_based_load == site_load(N)`* — value-free, and it exercises both
derivation paths against each other. Structural tests involve no electrical constant at all:
segment starts stride by 15 minutes for a 1-hour IOI; `intersects` is true for an overlap and false
for an abutment; the first segment wins when it is maximal; `agg_count` is `Bracket::exact(0.5)` for
a session covering exactly half a segment; an empty segment equals `site_load(0)`.

**New `tests/segment_tiling.rs`**, replacing the deleted `session_grouping_diagram.rs` and reusing
its `Session_Report_Diagram.csv` fixture — seven sessions over 16:00–17:00 exercising nesting,
staggering, same-minute start/end, and interval overrun, driven through the public API only.

**Update `excel.rs` spike tests.** `:1645` asserts `spike.avg_kw.is_infinite()`, which the new method
never returns. **[D]** Split into `assert!(spike.charge_time.is_zero())` — detection still keys on
the degenerate input — and `assert_eq!(spike.avg_kw(), BREAKER_RATING_KW)` for non-zero energy,
alongside the existing `0.0` case at `:1604`. Drop or make specific the now-trivial `is_finite()` at
`:1639`.

**Golden fixtures.** **[D]** These are the one deliberate exception to the value-independence rule:
they pin *rendering*, not physics, and no relational reformulation preserves column widths, decimal
places, or wrapping. Every number in them changes in Stage 1 regardless. Regenerate via the
mechanism that already exists — `UPDATE_REPORT_GOLDEN` at `report_rendering.rs:86` — and review the
diff. **[D]** Add a second `#[test]` in the same file honouring the same variable for
`tests/fixtures/site_load.report.txt` (`.txt`: fixed-width plain text, not markdown), so
`UPDATE_REPORT_GOLDEN=1 cargo test` regenerates everything in one command.

---

## Stage 4 — Documentation

**README** — the algorithm section's formulas stay as written. **[D]** They are a defensible
approximation and `~6.7 kW average` is correct: the technical doc's §4 table gives 33.61 kW at N=5,
i.e. 6.72 kW per EV including that segment's share of the transformer block. Add parenthetical
remarks at README:97, :101, :99, :108 marking them approximations and linking to the new subsection.

Add **"kW and kVA calculations"** under Technical Notes, between "Boundaries and the time grid" and
"Assumptions", ~3 short paragraphs: (1) the per-EV figure is an average, not a constant — the
standing block is fixed and I²R loss rises with loading, so per-EV kW falls from ~6.94 at N=1 toward
~6.6 at N=10, passing ~6.7 near N=5; (2) kVA is a quadrature sum of real, reactive and distortion
components, not `kW ÷ PF`, and the ~0.98 is good near full occupancy and poor at low counts (site PF
is 0.94 at N=1); (3) pointers to `docs/ev-charger-power-factor-and-kva-allocation.md` and
`docs/Evolute-Simultaneous_Charging.pdf`.

Also in README: add a sentence at :83 saying the reported peak is always a quarter-hour average
whatever interval length was asked for, and reconcile :112/:224 with the `In interval` column.

**`docs/maintenance-manual.md`** (new; kebab-case matches the authored-doc convention and stays
clearly distinct from the user manual planned for later). Scope — what a maintainer must do that the
code cannot tell them: (1) which electrical constants are free to change and which are derived, and
that tests are written to survive changes to the former; (2) golden-file regeneration — the one
command, what to check in the diff, why fixtures are the deliberate exception; (3) the `R` divides
15 minutes invariant on `TIME_GRID_STEP`, which nothing enforces; (4) how to add an `AnomalyKind`
(the wire format in `as_str`/`from_token`, and that `collect_session_anomalies` is deliberately
blind to the kind).

**`docs/segment-tiling.md`** (new) — prose + table walkthrough of the same seven-session example,
replacing what `docs/session-grouping.md` did for the group model. **[D]** No SVG: a uniform
15-minute partition does not need the diagram that variable-length groups did.

**`docs/ev-charger-power-factor-and-kva-allocation.md`** — it names `ev_apparent_kva()` (actual:
`ev_apparent_power_kva()`, `site_load.rs:113`) and `ev_site_load.rs` (actual: `src/site_load.rs`).
Fix both and sweep the whole identifier table against `site_load.rs`, since the README is about to
cite this doc as authority.

---

## Commits

**[D]** Four commits, one per stage. Stage 1 is the only one that moves numbers, so isolating it is
what lets you tell which change moved a figure when reviewing the regenerated fixtures.

## Verification

1. `cargo build --all-targets` — clean, no warnings. First time this has been possible.
2. `cargo clippy --all-targets` — sweep whatever it surfaces; it has produced no lint data at all
   until now.
3. `cargo test` — all tests pass.
4. **Constant-independence check**, the one that proves Stage 3's rule holds: temporarily change
   `BREAKER_RATING_A` from 40.0 to 32.0, run `cargo test`, and confirm *only* the golden-fixture
   tests fail. Revert.
5. `cargo run --example site_load_report` — table renders; compare against
   `docs/ev-charger-power-factor-and-kva-allocation.md` §4 by eye.
6. `cargo run --bin ev_estimate_cli -- <workbook.xlsx> "2026-06-15 16:00" 1h` on a `data/` workbook —
   check the Segments table shows four quarters, the Estimates section names the winning quarter,
   and every figure is a bracket.
7. `cargo run --bin ev_peak_gui` — Convert tab still works; Estimate tab headline shows four
   brackets and the winning quarter; saved report is byte-identical to the CLI's stdout, per
   README:33.
