# How the four estimate sets order

`PowerEstimates` reports the same interval under four readings, over two independent axes — the
**boundary** axis (`wide` counts every session, `narrow` only those whose overlap with the interval
is certain) and the **panel** axis (`direct` applies no constraint, `clamped` cuts a group down to
`EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`).

This page settles how the four compare, because the answer is not obvious and the codebase asserted
the wrong one for a while. See the **To do** at the foot.

## The answer

The order is **partial, not total**:

```
                direct
               /      \
        clamped        direct_narrow
               \      /
             clamped_narrow
```

`direct` is the greatest and `clamped_narrow` the least, so a bracket over the four runs between
them. But `clamped` and `direct_narrow` are **incomparable** — neither dominates the other, and this
is not a hypothetical. In
[`tests/fixtures/Session_Report_Four_Sets.report.md`](../tests/fixtures/Session_Report_Four_Sets.report.md):

| set | consumption kW | breaker-spec kW |
|:--|--:|--:|
| `direct` | 13.500 | 80.400 |
| `clamped` | 13.500 | 67.000 |
| `direct_narrow` | 11.500 | 80.400 |
| `clamped_narrow` | 10.000 | 67.000 |

`clamped` beats `direct_narrow` on consumption and loses to it on breaker-spec. They are
incomparable per-figure as well as jointly: which of the two is larger on any single figure depends
on whether clamping or narrowing bites the peaking group harder.

## Why `unclamped >= clamped`

Unconditional, on either boundary setting. `clamped(g)` sums over `eligible_sessions(g)`, a subset
of the members of size `min(|members|, 10)`, and counts no more of them. No `avg_power` reaching a
group is negative — a spike's infinite value is substituted before grouping. Per-group dominance
carries to the maximum over groups, so it holds of the reported figures too.

## Why `wide >= narrow`

On the `direct` axis this is the same one-line argument: the narrow member set is a subset.

On the `clamped` axis it needs work, because **`eligible_sessions` is not monotone under removing a
member.** The tiers drop short-overlap sessions ahead of low-power ones, so removing a long-overlap
member can lift a group back under the panel limit and *raise* its clamped total. Concretely, a
group of eleven — ten short-overlap at 10 kW and one long-overlap at 1 kW — clamps to `9 x 10 + 1 =
91`; drop the long-overlap member and ten remain, clamping goes inert, and the total is `100`.

What rules that out is a lemma about which tier a flagged session can occupy.

### Lemma: every flagged session is short-overlap

A session carries `IntersectsBoundaryMarginOnly` when it is admitted but fails the provable-overlap
test, which has two disjuncts:

- **head-flagged** — `adj_conn_end <= lo + R` fails to exceed the margin at the interval's start
- **tail-flagged** — `conn_start <= hi - R` fails at the interval's end

where `R` is one `SESSION_BOUNDARY_RESOLUTION`.

**Tail-flagging cannot occur.** It needs `conn_start > hi - R`, and membership needs
`conn_start < hi`, so `conn_start` would have to fall in the open interval `(hi - R, hi)`. Reported
times are truncated to whole minutes and `hi` is a whole minute, and an open interval between two
consecutive minutes contains no whole minute.

**Head-flagging forces tier 1.** A head-flagged session has `adj_conn_end == lo + R` exactly: it
must exceed `lo` to be a member, must not exceed `lo + R` to be flagged, and is a whole minute.
Since `conn_start <= conn_end == lo`, its left end-point clamps to `lo` and its right to `lo + R`.
Every group boundary is a whole minute, so no boundary falls strictly inside `[lo, lo + R)` and the
session belongs to exactly one group, that one. Its tier measure there is
`adj_conn_end - group.start == R`, which is short-overlap.

### The three cases

Write `F` for a group's flagged members and `t1`, `t2` for its short- and long-overlap tiers. By the
lemma `F` is a subset of `t1`, so **`t2` is entirely certain** — that is the whole of the work.

| case | wide keeps | narrow keeps | |
|:--|:--|:--|:--|
| `\|members\| <= 10` | everything | the certain subset | superset wins |
| `\|t2\| >= 10` | top ten of `t2` | top ten of `t2` | identical sets |
| `\|t2\| < 10` | `t2` + top `10-\|t2\|` of `t1` | `t2` + at most that many of `t1 \ F` | a top-k of `t1` dominates any k-subset of `t1 \ F` |

So `clamped >= clamped_narrow` in every case, and with the other axis the lattice above follows.

## The precondition, and where it is enforced

Everything rests on whole-minute times. Session times come that way from the workbook. Interval
bounds are checked in [`src/bin/estimates.rs`](../src/bin/estimates.rs), which rejects a start
carrying seconds — *"interval start … carries seconds; it must be a whole minute"* — and takes its
length in whole minutes.

**The library is weaker than the CLI.** `max_power_estimates_for_interval` accepts any
`(Timestamp, Timestamp)`. A caller passing `hi = 16:59:30` gets tail-flagged sessions, those can sit
in `t2`, and the counterexample above becomes reachable: `clamped_narrow` exceeds `clamped` and the
lattice breaks. The unit test named `"tail"` in
`grouping::test::boundary_margin_flags_overlap_it_cannot_establish` is exactly such a session,
constructed at a granularity no workbook has.

The report therefore computes its range as an actual minimum and maximum over the sets present,
rather than reading off `direct` and `clamped_narrow`. That stays truthful for a library caller
outside the CLI's guarantee, and it costs nothing — the extremes are often absent anyway, since
`clamped_narrow` is suppressed whenever it would repeat another set, so the code has to walk the
present sets regardless.

## To do

Four sites currently assert that the four sets do **not** nest, which is wrong for any input the
workbook and CLI can express. Each should state the partial order, the lemma it rests on, and that
the code declines to depend on it:

- [ ] `src/grouping.rs`, the `eligible_sessions` doc — "the composition is not monotone" is true of
      the function in isolation and should say so, then note that the members narrowing removes are
      always tier 1, so the composition *is* monotone on real input.
- [ ] `src/peak_est.rs`, the `PowerEstimates` doc — the "**The four do not nest**" paragraph.
- [ ] `src/report.rs`, the comment above the bracket computation in `push_estimates`.
- [ ] `README.md`, the matching paragraph under *The four estimate sets*.

Two further items, unrelated to the ordering but outstanding:

- [ ] `README.md` describes neither the dagger marking doubtful sessions under *Group membership*
      nor the conditional `Direct`/`Narrow` column pairs in the *Session groups* table.
- [ ] Consider tightening the library entry point so it cannot be handed an interval the CLI would
      reject, which would turn the precondition above into something the type system carries.
