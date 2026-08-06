# Clock skew and drift

## Context

`README.md` on branch `clock-skew` adds a "Clock skew and drift" section specifying that the
estimates account for the two unreconciled clocks — Toronto Hydro's meter, which fixes the interval
of interest, and Evolute's, which fixes the session times. `src/common.rs` gained
`CLOCK_SKEW_AND_DRIFT` and a `CLOCK_SKEW_MARGIN` derived from it. Neither is referenced anywhere
else, so none of the treatment is implemented.

A grilling pass over the new section found the defects and gaps resolved below. The spec is settled
first, then the code, following `remove-clamping-plan.md`.

The README stays in the present tense throughout, per the user's direction: it is the spec the code
is written against, and the gap between it and the code is temporary.

## Spec decisions

1. **The object being bounded is the shifted window, not the strips.** Skew means the billed
   interval is really `[I.start + δ, I.end + δ)` for an unknown `|δ| <= M`. Every such window lies
   inside `[I.start − M, I.end + M) = L ∪ I ∪ R`, and the peak over a union is the max of the peaks
   over its parts, so evaluating the two strips alongside `I` bounds every candidate window. The
   README jumps straight to the strips without naming what they stand in for; the justification goes
   in.

   The bound is deliberately loose: it admits an instant from `L` and one from `R` that no single `δ`
   could bring into view together. Stated as such rather than left for a reader to notice.

2. **The margin is derived from a named bound**, and the derivation is the spec's, not a restatement
   of its current value:

   ```
   CLOCK_SKEW_BOUND  = 5s                                (assumption)
   CLOCK_SKEW_MARGIN = R * ceil(CLOCK_SKEW_BOUND / R)     (currently one R = 60s)
   ```

   The README's `max(absolute clock skew + drift, TIME_GRID_STEP)` is wrong on two
   counts. It reads on the actual skew, which nobody knows, rather than on the assumed bound. And
   `max` restores grid alignment only while the bound does not exceed `R` — a bound of 70s yields
   70s, which is off the grid, and the guarantee of "Boundaries and the time grid" fails silently.
   Rounding *up to a whole `R`* is the operation actually wanted; it coincides with `max` in the
   current range. `src/common.rs` already implements the `ceil` form; only the prose is stale.

3. **`CLOCK_SKEW_MARGIN == TIME_GRID_STEP` is a numeric coincidence, not an identity.**
   The note deleted from `TIME_GRID_STEP`'s doc comment — that truncation is one-sided
   and forward while skew is two-sided and applies to both end-points — is still true and still worth
   stating. `R` is the floor on the margin because of the grid, not because the two quantities are
   the same kind of thing.

4. **The `5s` bound is a heroic assumption.** Unverifiable: Evolute's clock discipline is unknown and
   Toronto Hydro's is not ours to ask about. Recorded as such in Assumptions, with no entry in
   `Questions_for_Evolute.md` — there is no answer to be had. Nothing downstream depends on the
   figure itself, only on its staying under one `R`.

5. **One term throughout: *skew margin*.** The README says "clock skew margin intervals" and then
   "drift margin estimates" for the figures drawn from them. Drift is folded into the bound, so it
   does not deserve a second name. *Skew margin interval*, *skew margin estimates*, and in the
   report, *left/right skew margin*.

6. **Each strip yields its own estimate set(s), in both readings.** The existing rules apply within
   each window unchanged, including `min_overlap` being shown only where it differs from that
   window's `nominal`. Every set's four figures therefore come from a single window and describe a
   scenario that could actually have happened — a figure-by-figure max across windows would produce
   a set whose kW came from one strip and whose breaker figure came from another.

7. **A strip appears when it beats `I` in like-for-like on either reading.** Shown when any of its
   four figures, in either reading, exceeds `I`'s corresponding figure in that same reading. The
   `min_overlap` clause is not redundant: `I`'s own nominal can be inflated by a dubious group while
   a strip's min reading still exceeds `I`'s, and that strip is worth seeing.

   Comparison uses each window's *effective* min reading — `min_overlap` where present, otherwise
   that window's `nominal`, which is what its absence means. Where `I` has no groups at all, any
   non-empty strip qualifies.

8. **The treatment is one-sided, and the generated report says nothing about it.** Skew is admitted
   only as an upward revision. The rationale belongs in the non-technical part of the spec: the
   estimates exist to bound what EV activity could have contributed to a demand charge, and the
   symmetric floor — the min over the three windows — is zero whenever a strip happens to be empty,
   which is common.

9. **The material splits across two sections.** What a skew margin is, that it revises only upward
   and why, and how the extra sets appear go under **Estimation logic**. The derivation — the `I+δ`
   union bound, the grid constraint, the rounding formula, the assumption — stays under **Technical
   Notes**.

10. **The interval boundary rules are left alone.** They constrain the interval the *caller* supplies,
    which `estimates` validates; internal windows need only lie on the `R` grid, and `I.start − M`
    does, since `M` is a multiple of `R` and `R` divides 15 minutes. No contradiction to resolve.

11. **Anomaly reporting widens to the whole span, and says which window.** Anomalies are collected
    for every session intersecting `[I.start − M, I.end + M)`, each marked with the window(s) it
    touches. Otherwise a strip figure can rest on a session whose anomalies the report never
    mentions — the situation the existing restriction to `I` exists to prevent. The excluded-session
    table's existing `Window` column extends the same way.

## A note on strips and dubiousness

Recorded because it is easy to get wrong in both directions, and it sets expectations for the report.

At `M = R` a strip spans exactly one grid cell, so it holds **exactly one group** — every session
endpoint lies on the `R` grid (`conn_start` truncated to the minute, `adj_conn_end` one `R` past a
truncated end), so no endpoint can fall strictly inside a one-cell strip. That group is therefore
narrow, hence *eligible* to be dubious.

Eligible is not the same as dubious. A member is movable only when a comparison holds with equality,
and for a strip both such conditions land on the strip's own minute:

- movable at its start ⟺ `conn_start == g.start` ⟺ its reported start lies in the strip
- movable at its end ⟺ `adj_conn_end == g.end` ⟺ its reported end lies in the strip

So a strip's group is dubious only when two or more of its members have a reported start or reported
end inside the strip's own minute, and not both anchored to the same side. Members that merely run
through the strip are `ba`, immovable, and contribute no doubt. Strips will not routinely print two
estimate sets.

This is the ordinary narrow-group rule with `g` happening to be the strip. Nothing about strips
changes it.

## `AnomalyKind::ExcessiveAvgPower`

Not part of the clock-skew work, and folded in here because building the skew-margin fixture is what
surfaced it. A first draft of that fixture gave its sessions 21 kW apiece, and the report printed:

```
The likely kW values are in the range from 41.429 kW (consumption-based) to
13.400 kW (breaker-spec-based).
```

The bracket inverted. `energy_based_kw` is a group's aggregate average power while
`count_based_kw` is that group's member count times a single rating, so the first exceeds the
second whenever a session draws more than the rating — which the hardware is supposed to make
impossible, and which nothing in the code checked. The fixture was corrected to stay under the
rating; the gap it exposed is closed by a new anomaly kind.

12. **The kind carries no payload.** `AnomalyKind` stays a plain classification and the figure is
    written into the report cell instead — `ExcessiveAvgPower(6.900)`. A payload would have cost
    three things: the `as_str`/`from_token` round-trip through the workbook's `Anomalies` column
    would have had to encode and re-parse a float; `Eq` would have gone (an `f64` is not `Eq`), and
    with it exact comparison of kinds; and the report's glossary, which deduplicates on the kind,
    would have printed one explanatory line per *session* rather than one per kind — silently, since
    it still compiles.

13. **Raised in `excel.rs`, with the other kinds.** So it reaches the workbook's `Anomalies` column
    and round-trips like the rest. Detecting it at report time would leave it invisible in the sheet.

14. **Informational only.** The session still takes part in every estimate. The figure says something
    is wrong with `Energy_Use` or `Active_Charge_Time` but not which, or whether either is;
    `InconsistentDuration` remains the only kind that excludes a session, as README.md, "Other"
    states.

15. **The test is `avg_power > BREAKER_RATING_KW`, exactly, with no tolerance.** An earlier
    draft rounded to the nearest 0.1 kW — equivalently `>= 6.75` — on the grounds that the report
    prints three decimals and a smaller discrepancy is invisible beside the figure. That was
    abandoned: it left a band, `(6.7, 6.75)`, in which a session inverts the bracket without being
    flagged, and members sitting in that band invert a group by up to 0.05 kW each, which *is*
    visible at three decimals.

    Exactness buys a clean guarantee. A group prints a backwards range only when its aggregate
    exceeds its member count times the rating, which by the pigeonhole principle requires some member
    above the rating — and every such member is now flagged. An inverted bracket therefore always has
    a flagged session behind it.

    The price is that a session meant to sit exactly at the rating may or may not be flagged, since
    `Energy_Use / Active_Charge_Time` need not land on `6.7` in binary floating point. It errs
    towards reporting, which is the right direction for a flag that excludes nothing.

16. **The figure is looked up from the tilings.** An anomaly record carries a row, an id and a kind
    and nothing else, so the renderer builds a session id → `avg_power` map from every tiling it
    holds. Every session with an anomaly intersects one of the three windows and so appears in some
    group of that window, which makes the lookup total. The excluded-sessions table needs no lookup —
    it holds the `Session` values themselves.

    A spike's figure is the one the estimating logic substituted, not the sheet's `#DIV/0!`. That is
    the number that fed the totals, which is the one worth seeing.

## README changes

1. **§Estimation logic — the four-value bullet list.** After the bracket bullets, a short passage:
   what a skew margin interval is, that the estimates are computed for the two of them, that they can
   only revise the bracket upward, and why (decision 8).

2. **§The two estimate sets.** "It is always given", "a report never shows the same four figures
   twice", and the definition of an estimate set are all scoped to `I` today. Rescope them per
   window, and describe the strip sets and their trigger (decisions 6, 7).

3. **§Estimation logic, anomalies bullet** (currently "every session that **intersects `I`**"):
   widen to the whole span per decision 11.

4. **§Technical Notes / Clock skew and drift.** Rewritten: the `I+δ` union bound and its looseness
   (decision 1), the derived margin and the grid reason for rounding up (decision 2), the coincidence
   with `R` (decision 3), the term *skew margin* (decision 5). Drop "as if they were regular
   intervals of interest" in favour of the precise claim — the same grouping and estimation machinery
   is applied to each strip.

5. **§Technical Notes / Boundaries and the time grid.** One clause: strip bounds are `I`'s bounds
   offset by a whole multiple of `R`, so they lie on the grid too.

6. **§Assumptions.** Fix the bullet's lead-in punctuation to match its neighbours (`**Clock skew and
   drift.**`), and state the `5s` bound as unverifiable per decision 4.

7. **Fix in passing**: "Includine" → "Include", and the stray leading space indenting the paragraph
   and list at the section's head.

8. **§Assumptions, breaker-ratings bullet.** A sub-bullet for `ExcessiveAvgPower`: a session drawing
   above the rating contradicts that assumption directly, is flagged rather than excluded, and would
   invert the reported bracket if its group's aggregate exceeded the member count times the rating.
   States the rounded comparison (decisions 12–16).

## Code changes

1. **`src/common.rs`.** Rename `CLOCK_SKEW_AND_DRIFT` to `CLOCK_SKEW_BOUND` — the current name says
   what the quantity is made of but not that it is a *maximum*, and everything above depends on its
   being one. Give it the doc comment carrying the assumption; today it has none and the assumption
   text sits on the static below it.

   The `CLOCK_SKEW_MARGIN` static's doc comment still ends with the superseded
   `(= max(absolute clock skew + drift, TIME_GRID_STEP))`, which contradicts the `ceil`
   formula directly beneath it. Replace it, have the prose read off `CLOCK_SKEW_BOUND` rather than
   restating `5s`, and add decision 3's note.

   The float `ceil` form stays. Its one failure mode — `(3.0 / 0.6).ceil() == 6`, overshooting by a
   step when the bound is an exact multiple of a step that binary floats cannot hold — is unreachable
   while `R` is a whole number of seconds, and fails conservative (a margin one `R` too wide) rather
   than unsound if it ever is reached. `from_secs_f64` rounds to the nearest nanosecond, which keeps
   the product on the grid.

2. **`src/estimates.rs` — window plumbing.** `max_power_estimates_for_interval` already reads the
   workbook once and then calls `groups_for_interval` + `estimates_for_groups`. Run that pair three
   times over the same `rsessions`, for `[I.start − M, I.start)`, `I`, and `[I.end, I.end + M)`.

   Introduce a per-window struct — interval, `Option<PowerEstimates>`, `session_groups` — and hold
   `I`'s alongside two `Option`s for the strips on `PowerEstimatesReport`. The group indices in
   `PowerEstimate::session_group_idx` are per-window and must stay that way; the report already names
   groups by position within a tiling.

3. **No single-group shortcut.** A strip holds one group only because `M == R` today, and the
   shortcuts that fact invites — a one-row strip summary, or figures read straight off the single
   group without going through `groups_for_interval` — are correct now and silently wrong the moment
   `CLOCK_SKEW_BOUND` exceeds 60s. Raising the bound must cost nothing but the constant. Strips go
   through the same path as `I`, and rendering handles *n* groups. This costs nothing, since that
   path already exists.

4. **The trigger predicate.** Implements decision 7 over `EstimateSet::values()`, comparing each
   window's effective min reading (`min_overlap` where `Some`, else `nominal`) as well as its
   nominal. Strips failing it are dropped from the report rather than merely hidden, so the renderer
   has nothing to decide.

5. **`collect_session_anomalies`.** Widen the filter to the full span and tag each anomaly with the
   window(s) its session touches. Its doc comment argues the current restriction to `I` at some
   length and needs rewriting, not just amending — the reason for the restriction (a spike three
   weeks away says nothing about this estimate) survives; its radius changes.

6. **`src/report.rs`.** Render up to two extra sections, each with its own group table and estimate
   table. `push_excluded`'s `Window` column and its explanatory note become three-valued. The
   header's `Interval` line should state the span the report actually covers when strips are shown.
   `interval_length` and `mmss` assume at most an hour; a strip is 60s, so they are safe, but check
   the assumption rather than inherit it.

7. **`ExcessiveAvgPower`.** The variant, its `as_str`/`from_token` tokens and its `Display` prose in
   `src/common.rs`; the test in `src/excel.rs`, in the `else` arm of the zero-charge-time branch so a
   spike cannot collect both; and `anomaly_cell` in `src/report.rs`, used by both the anomalies table
   and the excluded-sessions table so the same kind renders one way in both.

8. **Tests.** `tests/report_rendering.rs` and both golden fixtures under `tests/fixtures/` are
   affected — at minimum by the widened anomaly scope, whether or not a strip qualifies. Add coverage
   where a strip beats `I` and where it does not, so the trigger is exercised in both directions, and
   one session drawing above the breaker rating. `docs/session-grouping.md` walks the diagram fixture
   through step by step and will need a note if that fixture's output changes.

   A fixture that exercises `ExcessiveAvgPower` must keep its *groups* under the rating even while
   one session exceeds it, or the golden file bakes in the inverted bracket sentence and reads as an
   endorsement of it.

   Expect existing `excel` tests to start failing: their records were built with a flat
   `energy_use: 10.0` regardless of duration, drawing 12–20 kW, and two inline CSVs draw 7.2 kW.
   Fix the *data* rather than loosening the assertions to `contains` — a test record that draws three
   times what the hardware permits is wrong on its own terms, whatever it was written to check.

## Deliberately not done

- No downward revision of the bracket (decision 8).
- No entry in `Questions_for_Evolute.md` (decision 4).
- No change to the caller-facing interval boundary rules or to `estimates`' validation (decision 10).
- No marker in the README distinguishing specified-but-unbuilt behaviour; present tense throughout,
  per the user's direction.
- **No guard on the inverted bracket itself.** `ExcessiveAvgPower` flags the cause, but the sentence
  that states the bracket still reads "from {consumption} to {breaker-spec}" unconditionally, and
  would print a range running downwards if a group's aggregate average power ever exceeded its member
  count times the rating. The anomaly makes that visible rather than impossible — reliably so, since
  decision 15's exact threshold guarantees such a group carries at least one flagged member. Whether
  the renderer should also detect the inversion and reword is left open.
