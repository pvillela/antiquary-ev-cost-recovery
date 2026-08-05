# Remove clamping

## Context

`README.md` at HEAD (`87104cc`) revises the spec: clamping is gone, and with it the panel
concurrency limit, the `clamped`/`clamped_narrow` estimate sets and the `ClampedSessionGroup`
anomaly. In its place the spec addresses the ambiguity that minute-truncated session times leave
behind — when reported times cannot settle whether two sessions overlapped or merely abutted. The
project does not compile; a few working-tree edits stand as a guide to the intended shape.

A grilling pass over the revised spec found five soundness defects and several gaps. All are
resolved below. README is specified first, at the user's direction, then the code.

## Terminology and spec decisions

1. **The boundary-margin machinery is deleted entirely** — `AnomalyKind::IntersectsBoundaryMarginOnly`,
   `Session::overlap_is_certain`, `Boundary::{Wide,Narrow}`, the `boundary_margin` parameter, the
   report's dagger. The dubious-group treatment covers the live case; its right-hand half
   (`conn_start > hi - R`) was unreachable on the minute grid and never fired. Accepted consequence:
   a session reported to end exactly at `lo` is always counted, and the possibility that its true end
   was exactly `lo` — no overlap with `I` at all — is no longer represented.

2. **"Dubious group" replaces "narrow group" as the trigger for the second estimate set.** Narrow
   (duration `== R`) is a property of the tiling; dubious is the property that matters:

   > A group is **dubious** when it has two members that need not have overlapped each other.

   Necessary and sufficient for `min < max`. Equivalently: `bi` and `ia` both non-empty, or `ii`
   non-empty with at least two non-spanning members. Dubious implies narrow — in a group of duration
   `>= 2R` every member is running at the latest true start, so no pair can be disjoint — which makes
   narrowness a cheap screening test rather than part of the concept. A narrow group whose
   non-spanning members all sit in one bucket is **not** dubious, and neither is one whose only
   member spans it (a group opened and closed by non-members).

3. **The estimate sets are `nominal` and `min_overlap`.** `direct` lost its contrast partner with
   clamping; `nominal` says "the report taken at face value". `min`/`max` are avoided at set level
   because each set already contains a bracket whose ends are a min and a max.

4. **`min_overlap` is reported only when its four figures differ from `nominal`'s.** No report shows
   the same four numbers twice.

5. **`min_overlap <= nominal` holds unconditionally** on all four figures — per group
   (`ba + max(bi, ia, max ii) <= ba + bi + ia + ii`, and `min_size <= size`), hence under the max
   across groups. `docs/estimate-set-ordering.md` is **deleted**, being entirely about the retired
   four-set lattice and its incomparability counterexample.

6. **`TIME_GRID_STEP` (`R`) is exactly the resolution at which the Evolute report states
   session *start and end times*** — currently 60s, a plain `const`. `TIME_ERROR_MARGIN` and
   `SESSION_REPORTING_RESOLUTION` are removed: `max(reporting, margin)` is unsound in the only case
   where it does anything, since exceeding the reporting grid admits groups narrower than `R` and
   breaks both the grid and "duration `> R`" meaning "`>= 2R`". Naming start and end times
   specifically matters — `Conn_Duration` and `Active_Charge_Time` carry seconds, and the DST fold
   inference, the consistency band and the README:147 tolerance all depend on that asymmetry.

7. **The truncation assumption is stated separately from the definition of `R`.** `Adj_conn_end =
   Conn_end + R` is sound only because the reported end is the true end truncated *down* to `R`.
   Restore to `Questions_for_Evolute.md` the sub-question asking Evolute which instant their end time
   denotes: if they grant seconds, `R` and the correct padding change independently.

8. **The time grid is derived, not asserted.** Reported start/end times lie on the `R` grid, padding
   adds exactly one `R`, and `I`'s bounds are 15-minute aligned — so every group boundary is on the
   `R` grid and every duration is a multiple of `R`, **provided `R` divides 15 minutes**, a
   requirement on the report format rather than on this software.

9. **Dubious groups are marked with the existing asterisk, derived from `min_size < size`;
   `GroupAnomaly` is deleted** along with `PowerEstimate::group_anomalies` and the group-anomalies
   legend section. A single-variant enum with no wire format is ceremony. The marker still earns its
   keep, though decision 10 narrows the case: a dubious group whose non-spanning members all draw
   zero power has `min_agg == max_agg` while `min_size < size`, so it can peak on consumption with
   `min_overlap` suppressed. The asterisk is then the only thing saying the figure rests on a group
   whose membership is not settled.

10. **Ties on a figure go to the non-dubious group.** Among groups tied on a metric, the one whose
    figure is certain is named; earliest-wins remains the final fallback. This is a tie-break only —
    a dubious group with a strictly higher figure is still selected, since understating a maximum is
    the unsafe error. Applies to both estimate sets and to each of the two metrics independently.

    Consequence worth stating in the docs: if a figure's peak group is dubious, then no non-dubious
    group tied it, so every other group is strictly below and `min_overlap` comes out strictly lower.
    A dubious group carrying a peak therefore always produces a second table — except in the
    zero-power case noted in decision 9.

## README.md

Front matter stays non-technical, each derivation cross-referenced downward.

```
## Estimation logic          (bulleted algorithm, four figures, brackets)
### Sessions, groups, and doubt
### The two estimate sets
## Technical Notes
### Time zone
### Boundaries and the time grid
### Dubious groups
### min and max estimates for dubious groups
### Assumptions
### Other
```

- **Sessions, groups, and doubt** — half-open intervals; times stated to the minute so a session's
  true end is uncertain within it; groups where that leaves two members' overlap undetermined are
  *dubious*; the truncation assumption in plain terms.
- **The two estimate sets** — `nominal` / `min_overlap`, `min_overlap <= nominal`, shown only when
  the figures differ, every figure names the group it came from, `min_overlap` may peak on a
  different group, and where two groups tie on a figure the one whose figure is certain is named.
  Links `tests/fixtures/Session_Report_Diagram.report.md` as a worked example kept current by the
  golden-file test.
- **Boundaries and the time grid** — the half-open tiling argument (now README:87), the
  padding-is-a-full-`R` argument (README:89), the grid derivation and its divisibility premise.
- **Dubious groups** — the pairwise criterion, its equivalence to the bucket arithmetic, and the
  proof that duration `>= 2R` admits no doubt.
- **min and max estimates for dubious groups** — the nudge thought experiment (correct as the user
  left it), then the closed form, preceded by the fact that *every member runs its group's whole
  span* (`conn_start <= g.start`, `adj_conn_end >= g.end`) — which is what gives the buckets a
  literal referent, since no member literally "starts in" or "ends in" its group. Buckets as a table
  keyed on `conn_start` and `adj_conn_end`, never `conn_end`, never end-points clamped into `I`:

  | set | `conn_start` | `adj_conn_end` | what is certain |
  |---|---|---|---|
  | `ba_sessions` | `< g.start` | `> g.end` | runs through all of `g` |
  | `bi_sessions` | `< g.start` | `== g.end` | true end falls inside `g` |
  | `ia_sessions` | `== g.start` | `> g.end` | true start falls inside `g` |
  | `ii_sessions` | `== g.start` | `== g.end` | both true end-points fall inside `g` |

  Replaces both the `i.e.` clauses at README:189–191 and the duplicate definitions at README:201–204.
- **Assumptions** — uniform breaker kW/kVA ratings across panels; clock drift between the Toronto
  Hydro meter and the Evolute panel unmodelled; end times truncated (also stated non-technically).
  The clamping removal *retires* the old multi-panel distortion caveat — nothing in the revised
  design asks which panel a session ran on — which justifies dropping the panel-ID question from
  `Questions_for_Evolute.md`.

Corrections: README:96 "less than" → "less than **or equal to**" (subsumed by the rewrite);
README:219 `Adj_conn_start` is undefined, write `[Conn_start, Conn_start + R)`; README:228 delete the
stale boundary-margin sentence; README:51, 83, 89 express the padding as one `R` with 60s given once.
Typos: README:95 "must contains"; 205 "the sum of `avg_kw` of and size of" and "`size`" → "`_size`";
207 stray backtick; 79 "included in report"; missing spaces at 186, 189, 192; double spaces at 93,
100, 106, 112, 113.

## Code

### `src/common.rs`
Drop `TIME_ERROR_MARGIN` and `SESSION_REPORTING_RESOLUTION`; `TIME_GRID_STEP` returns to
`pub const Duration::from_secs(60)`, documented per decisions 6–8. Delete
`AnomalyKind::IntersectsBoundaryMarginOnly` (variant, `as_str`, `from_token`, `Display` arm) and
`Session::overlap_is_certain`.

### `src/grouping.rs`
- Delete `Boundary`, `Clamping`, `GroupAnomaly`, `SessionGroup::anomalies`, `pure_clamped_size`,
  `pure_clamped_agg_avg_power`, and `eligible_sessions` with its two-tier ranking.
- `View` becomes a two-variant enum (`Nominal` / `MinOverlap`); cache becomes `[Figures; 2]`;
  `size_in` / `agg_avg_power_in` keep their signatures so `estimate_set` stays written once.
- New bucket classification over the group's members, reading each session's own `conn_start` and
  `adj_conn_end`, feeding `min` figures computed in `initialize_cache` alongside the nominal ones.
- `SessionGroup::is_dubious()` = `size_in(MinOverlap) < size()`. The power figures cannot substitute:
  a zero-power member can leave `min_agg == max_agg` while the sizes differ.
- `end_points_for_interval` loses `boundary_margin` and its mutation of the sessions;
  `groups_for_interval` takes `&[RSession]`. The double-flagging hazard at `src/grouping.rs:428-432`
  disappears rather than being managed.

### `src/peak_est.rs`
- `PowerEstimates { nominal: EstimateSet, min_overlap: Option<EstimateSet> }`.
- `estimates_for_groups` computes both and keeps `min_overlap` only when its `values()` differ; the
  `shown`/`keep` machinery for four sets collapses to one comparison.
- Delete `PowerEstimate::group_anomalies`. Rewrite the `PowerEstimates` doc comment, which currently
  documents the four-set non-nesting problem that no longer exists — replaced by the unconditional
  ordering of decision 5.
- `max_group_by` gains the decision 10 tie-break. Its `score > best_score` test becomes a
  `partial_cmp`: `Greater` improves, `Equal` improves only when the incumbent is dubious and the
  candidate is not, anything else does not — which preserves both `T: PartialOrd` and earliest-wins
  as the final fallback. Its doc comment currently reads "Ties go to the earliest group, so the
  reported peak window is the first moment the peak was reached" and must state the new order:
  certain before doubtful, then earliest.

### `src/report.rs`
- Estimate labels `"Nominal"` / `"Minimum overlap"` with new glosses; the "more than one reading is
  defensible" prose reduces to the single dubious-group reason.
- Group table: extra columns gated on any dubious group, headed `Count`/`kW` and `Min count`/`Min kW`;
  `any_doubtful` becomes `any_dubious`, and the two now coincide with `any_flagged`.
- Asterisk legend rewritten to name dubious groups; the dagger (`DOUBTFUL`, `src/report.rs:143`), its
  membership column and the group-anomalies legend section are deleted.
- The bracket sentence stops being a min/max over sets present and becomes `min_overlap` to
  `nominal`, per the ordering.

### `src/excel.rs`
Revert the `LazyLock` churn: `static END_PADDING` back to `const`, and every
`*TIME_GRID_STEP` deref back to a plain reference. Update the "New fields" doc
references to the renamed README sections.

### Docs and fixtures
- Delete `docs/estimate-set-ordering.md`.
- `docs/session-grouping.md`: replace the clamping paragraph (lines 29–30) with the dubious analysis
  of group 5 — buckets `ba = {A, C}`, `bi = {D, E}`, `ia = {F}`, giving `min_agg = 12.2 + 12.5 =
  24.7 kW` against a nominal `31.4`, and `min_size = 2 + max(2, 1) = 4` against `5`. Note that group
  5 under minimum overlap *is* group 4's membership `{A, C, D, E}`, since minimum overlap is exactly
  the case where F had not started — which is why the two tie at `24.7`. Group 4 is named, and under
  decision 10 for the right reason: it is the non-dubious of the two, so the figure reported is the
  one that is certain. The example illustrates the tie-break as well as the grouping. Update the
  README cross-reference names.
- Delete fixtures `Session_Report_Clamped.*` and `Session_Report_Four_Sets.*`; keep
  `Session_Report_Anomalies.*` and `Session_Report_Diagram.*`. Update the case list and module docs
  in `tests/report_rendering.rs`, which currently describes four cases including two unreachable
  through the real path.
- `Questions_for_Evolute.md`: restore the sub-question asking which instant the reported end time
  denotes.

### Tests
Delete the clamping tests (`clamped_set_is_absent_…`, `clamped_set_appears_…`,
`clamping_can_move_the_peak_…`, `clamped_size_matches_the_eligible_set` and the `eligible_sessions`
cases). Add, weighted to the narrow/dubious gap:

- each of the four buckets, including a spanning session at `I`'s edge whose clamped end-points would
  misclassify it as `ii`;
- narrow but **not** dubious: a lone `bi`; two `bi` and nothing else; `ba` only, in a group opened and
  closed by non-members;
- the `min` closed form against a hand-computed group;
- `min_overlap` suppressed when the four values match `nominal`;
- the peak moving between the two sets;
- the decision 10 tie-break: a dubious group tying a non-dubious one on both metrics, asserting the
  non-dubious group is named in each set, and that a dubious group with a strictly higher figure is
  still selected.

`tests/session_grouping_diagram.rs`: revert the derefs and add assertions for group 5's min figures
and for both estimate sets.

## Two corrections found during implementation

Both were caught by writing the tests the plan called for, and both are now covered by named tests.

1. **The bucket arithmetic is sound only at a width of one `R`, and the plan applied it at every
   width.** Sessions `A = [20:02, 20:05)` and `B = [20:03, 20:05)` give the group `[20:03, 20:05)`,
   two resolutions wide, with `A` in `bi` and `B` in `ii` — yet `A` runs from before the group starts
   until at least `20:04` and `B` starts before `20:04`, so they certainly overlap. The closed form
   returned `min_size = 1`. `min_overlap_figures` now screens on narrowness first and returns the
   nominal figures for anything wider; the screen is a correctness requirement, not an optimisation.
   Pinned by `groups_wider_than_one_resolution_are_never_dubious`.

2. **Decision 10's tie-break was defeated by floating-point associativity.** Group 4 of the worked
   example holds `{A, C, D, E}` and group 5's minimum reading is the same four sessions, but the
   nominal figure sums them in id order while the closed form computes `(A + C) + (D + E)`. The two
   differ by 3.6e-15, so group 5 scored strictly higher and the tie never arose — in exactly the case
   the tie-break exists for. Fixed at source rather than with an epsilon: the minimum reading is a
   sub-multiset of the group's members, so its aggregate is now summed over those members in the
   group's own iteration order. Equal multisets give bit-identical totals. Pinned by
   `tests/session_grouping_diagram.rs`, which asserts the two aggregates are equal exactly.

## Verification

1. `cargo build` then `cargo test` — the build is currently broken, so first green build is the
   first checkpoint.
2. `UPDATE_REPORT_GOLDEN=1 cargo test --test report_rendering`, then read the golden diffs before
   accepting them: layout is the thing under test.
3. End to end on real data: `cargo run --bin csv_to_xlsx data/Session_Report_June_1_2026-June_30_2026.csv`
   then `cargo run --bin estimates <xlsx> "2026-06-15 16:00" 1h`. Confirm a single `Nominal` table
   when no group is dubious, and that any dubious group shows the asterisk with differing Min columns.
4. Confirm the diagram test's numbers match `docs/session-grouping.md` — it drives the real pipeline
   and asserts every figure on that page.
