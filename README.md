# Contribution of EVs to Building's Peak Power Consumption

This software supports the estimation of the impact of EV charging on the building's peak power demand. Peak kW and kVA are used by Toronto Hydro to calculate distribution and transmission charges.

## Data sources and intervals of interest

For a given billing period, we can identify the time intervals in which the peak kW and kVA occurred based on metering data downloads from Toronto Hydro.

Given a time interval of interest, this software estimates the peak kW and kVA demand associated with EV charging activity during the interval. The data source for EV power demand is the Evolute monthly session report.

**Interval of interest boundaries** are constrained as follows:

- The left and right end-points are always of the form HH:00:00 or HH:15:00 or HH:30:00 or HH:45:00.
- The difference between the right end-point and the left end-point can be either:
  - 1 hour -- only if the left end-point is of the form HH:00:00.
  - 15 minutes -- in all four cases.
- The interval is half-open: it includes the left end-point and excludes the right end-point.

## Workflow

This is the typical workflow used with this software to estimate the impact of EV charging activity on a particular Toronto Hydro bill:

- Preliminary steps (out of scope for this software):
  - Download Toronto Hydro metering data for the time period of interest.
  - Based on the downloaded data, identify the interval(s) of interest during which the billing period's peak kW and/or peak kVA occurred.
  - Obtain the *session report* file from Evolute covering the interval(s) of interest.
- Using this software:
  - Transform the relevant Evolute *session report* CSV file to an Excel file. The transformation process includes some data validation and computes additional columns that are included in the Excel file.
  - Access the relevant Excel file and compute the peak kW and kVA brackets for the interval(s) of interest.

## Tools

The same work is available two ways: a desktop app covering the whole workflow, and two command-line binaries, one per workflow step. Both compute their figures with the same library code, and a report saved from the app is byte-for-byte what the command line prints.

### Desktop app

`ev_peak_gui` is a single self-contained binary — no installer, no runtime to put on the machine first. It opens on a choice of the two jobs, and neither is entered until it is asked for:

- **Convert** — pick an Evolute session report CSV; the workbook is written beside it, and the rows that needed a judgement call are listed. Converting over an existing workbook asks first.
- **Estimate** — pick a workbook, choose the interval of interest, read the figures. The report is shown in full, and can be copied or saved.

The interval is chosen from controls rather than typed, and they offer only intervals the boundary rules allow: the start minute is one of `:00 :15 :30 :45`, and `1 hour` is on offer only from `HH:00`. Selecting a workbook reads it once, to show what it covers and to start the date on the first day it has anything to say about.

The two DST transitions appear where they matter and nowhere else. On the night the clocks go forward, the skipped hour is simply absent from the hour list. On the night they go back, choosing the repeated hour asks which of the two is meant, and Estimate stays disabled until that is answered.

**Running it.** On Windows, double-click `ev_peak_gui.exe`. It is not code-signed, so the first run shows SmartScreen's "Windows protected your PC" — choose *More info* then *Run anyway*; later runs are silent. On Linux, mark it executable once (`chmod +x ev_peak_gui`) and run it.

### Command line

| Command                                                      | Purpose                                                      |
| ------------------------------------------------------------ | ------------------------------------------------------------ |
| `ev_csv_to_xlsx <SESSION_REPORT.csv>...`                     | Converts a session report to a workbook, computing the derived columns and flagging rows that need review. Takes several files at once; the app takes one. |
| `ev_peak_cli <SESSION_REPORT.xlsx> <YYYY-MM-DD HH:MM [EST\|EDT]> [15m\|1h]` | Prints the peak estimate report for one interval of interest. |

`ev_peak_cli` takes the interval start in **local time (ET)**. The length defaults to `1h` when the start is on the hour and `15m` otherwise. An interval breaking the boundary rules described earlier is rejected rather than estimated.

The two DST transitions are treated differently, because they are different problems.

- On the night DST **ends**, an hour of wall time occurs twice. That is a question the caller can answer, so `ev_peak_cli` asks it: a bare `"2026-11-01 01:30"` is refused, and `"2026-11-01 01:30 EST"` or `"... EDT"` resolves it. The designator is accepted on any date and **checked against it** — `"2026-06-01 16:00 EST"` is an error, not a silently ignored hint — so naming the wrong one cannot produce a figure for the wrong hour.
- On the night DST **begins**, an hour of wall time never happens. There is nothing to choose between, so such a start is refused outright and no designator helps.

These rules live in one place, `src/interval.rs`, and both front-ends come through it, so the app and the command line cannot disagree about what interval a bill may be argued from.

## Excel workbook

The conversion from CSV to Excel includes the addition of new fields:

- `Adj_conn_end`, is computed as: `Conn_DateTime_End + SESSION_BOUNDARY_RESOLUTION` (currently 60 seconds). It is the session's **exclusive** end: a session starting at exactly this time does not overlap this one.
- `Adj_conn_duration`, is computed as: `Adj_conn_end - Conn_DateTime_Start`.
- `Conn_start_UTC`, `Conn_end_UTC`, and `Adj_conn_end_UTC`, with UTC values corresponding to the corresponding local time fields.
- `Avg_power` in kW, is computed as: `Energy_Use / (Active_Charge_Time * 24)`.
- `Anomalies`, containing a comma-separated list of `AnomalyKind` variant names, is added as the last column.

None of the data in the Excel workbook (or the source CSV) should be modified by the user, as any changes would impact and possibly invalidate the estimates.

## Estimation logic

### Estimating algorithm overview

Given a time interval of interest **`I`** as described above, the estimation of EV peak power demand during the interval proceeds as follows:

- From the Evolute monthly session report, identify all charging sessions that intersect the interval of interest `I`.
- At any time **`t`** within the interval of interest, the set of EV charging sessions that contain `t` can be determined. Such a set may be empty, contain a single session, or contain multiple sessions.
- The sets of EV charging sessions that are concurrently active may change a finite number of times during the interval of interest. These are called <strong>`SessionGroup`</strong>s.
- The algorithm implemented by this software identifies all non-empty `SessionGroup`s for the given interval of interest `I`.
- For each `SessionGroup`, the algorithm computes the following values:
  - **`avg_kw`**:  sum over all sessions in the `SessionGroup` of each session's average power demand. For each session, the average power demand is the session's total energy consumption divided by the session's charging time.
  - **`size`**: number of sessions in the `SessionGroup`.
- For the interval of interest `I`, the algorithm computes the following values:
  - **`consumption_based_kw`**: highest value of `avg_kw` over all `SessionGroup`s.
  - **`consumption_based_kva`**: `consumption_based_kw` divided by a power factor constant **`EV_POWER_FACTOR`** that reflects the combination of typical EV chargers and the Evolute infrastructure.
  - **`breaker_spec_based_kw`**: highest value of `size` over all `SessionGroup`s multiplied by the Evolute smart breaker kW rating of 6.7 kW.
  - **`breaker_spec_based_kva`**: highest value of `size` over all `SessionGroup`s multiplied by the Evolute smart breaker kVA rating of 7.5 kVA.
- These four values provide brackets for the EV peak power demand during the interval of interest `I`:
  - The actual peak kW associated with EV charging activity during `I` is likely between `consumption_based_kw` and `breaker_spec_based_kw`.
  - The actual peak kVA associated with EV charging activity during `I` is likely between `consumption_based_kva` and `breaker_spec_based_kva`.
- These four values are the **`nominal`** estimates: computed from the `SessionGroup`s exactly as the report gives them. A second set, the **`min_overlap`** estimates, is reported when [dubious](#sessions-groups-and-doubt) groups make its figures differ from the `nominal` ones.
- The interval of interest comes from Toronto Hydro's metering data while the session times come from Evolute, and nothing reconciles the two clocks. The estimates therefore also cover a ***skew margin interval*** at each end of `I` — a short window immediately before it and another immediately after — computed by the same algorithm and reported alongside `I` when its figures come out higher. See [Clock skew and drift](#clock-skew-and-drift).
  - Skew is admitted only as an *upward* revision. These figures exist to bound what EV charging could have contributed to a demand charge, so a candidate window that would lower them changes nothing worth reporting — and the symmetric floor, the lowest figure over the three windows, is zero whenever a margin happens to hold no session at all.
- The software detects data anomalies in the reported session data. Anomalies associated with every session that **intersects `I` or either skew margin** are reported alongside the estimates, each marked with the window or windows it touches, as well as anomalies that caused sessions to be excluded from the analysis. Other sessions elsewhere in the workbook are not included in the report.

### Sessions, groups, and doubt

Sessions, `SessionGroup`s, and intervals of interest are all **half-open**: each includes its left end-point and excludes its right one. Consecutive groups therefore meet at a single instant belonging to the later one, so no instant falls in two groups, and *abutting* stays distinguishable from *overlapping* — a distinction the estimates turn on. See [Boundaries and the time grid](#boundaries-and-the-time-grid).

`SESSION_BOUNDARY_RESOLUTION` — written **`R`** below, currently 60 seconds — is exactly the resolution at which the session report states session **start and end times**. It is not the resolution of everything in the report: `Conn_Duration` and `Active_Charge_Time` are stated more finely, and several of the Technical Notes depend on that difference.

A time stated to the minute is the true time truncated down to the minute, so a session reported to end at `16:34` truly ended somewhere in `[16:34:00, 16:35:00)`. The software therefore records an adjusted end, **`Adj_conn_end`**, one `R` past the reported end — the exclusive bound that contains the true end wherever in that minute it fell. That the report truncates rather than rounds is an assumption; see [Assumptions](#assumptions).

What truncation leaves behind is a residual doubt the estimates have to answer for. Where one session is reported to end in the same minute another is reported to start, the two may have genuinely overlapped for part of that minute, or may merely have abutted; the reported times cannot say which. A group in which that question is live is called a ***dubious*** group:

> A group is **dubious** when it has two members that need not have overlapped each other.

Only a group exactly one `R` long can be dubious, and not every such group is — see [Dubious groups](#dubious-groups). For a dubious group, the software computes two readings: **`max`**, taking the group's reported membership at face value, and **`min`**, assuming as little overlap as the reported times allow.

### The two estimate sets

An ***estimate set*** consists of the following values: `consumption_based_kw`, `consumption_based_kva`, `breaker_spec_based_kw`, and `breaker_spec_based_kva`. Each window the report covers — the interval of interest, and each skew margin that earns a place — has its own sets, and every figure in a set is drawn from a group in that one window. A set therefore always describes a state of affairs that could actually have obtained.

Within a window:

- **`nominal`** uses the `max` reading of every dubious group. It is always given, and it is the figure to quote when only one is wanted.
- **`min_overlap`** uses the `min` reading instead. It is given only when its figures differ from that window's `nominal` ones: a dubious group that carries no peak changes no reported number, and no window shows the same four figures twice.

`min_overlap <= nominal` on all four figures, always. Each figure names the `SessionGroup` it was drawn from, and the two sets may name different groups — lowering a dubious group can hand the peak to one that was never in doubt. Where two groups tie on a figure, the one whose figure is certain is the one named. Dubious groups are marked in the report's group table whether or not a second estimate set appears.

A skew margin's sets appear only when the margin could raise the estimate: when any of its four figures, in either reading, exceeds the corresponding figure for the interval of interest in that same reading. Readings are compared like with like, taking a window's `nominal` as its min reading wherever no `min_overlap` was given — which is exactly what its absence means. Both clauses earn their place: a margin can beat `I` on the min reading alone, when `I`'s own `nominal` is the one inflated by a dubious group. A margin that cannot raise anything is left out of the report entirely.

A worked example, kept current by a golden-file test: [`tests/fixtures/Session_Report_Diagram.report.md`](tests/fixtures/Session_Report_Diagram.report.md), walked through step by step in [`docs/session-grouping.md`](docs/session-grouping.md).

## Technical Notes

### Time zone

- The session report's timestamps are stated in local time, i.e., ET. We need to convert them to UTC.
  The time zone is `America/Toronto`.
- The conversion to UTC is straightforward for almost every point in time, except for the repeated hour on the day that DST ends (move from EDT 02:00 to EST 01:00). 
  - Based on the `Conn_DateTime_Start`, `Conn_DateTime_End`, and `Conn_Duration` fields in the Evolute session report, the corresponding UTC values can be inferred, except for sessions with duration of less than 1 hour that fall between the ambiguous 01:00:00-01:59:59 interval.
  - For the above-mentioned short sessions in the ambiguous interval, we need to make an assumption. For now, our policy will be to duplicate those session records, with one copy in the 01:00:00-01:59:59 EDT interval and the other copy in the 01:00:00-01:59:59 EST interval. This should be recorded in the CSV to Excel transformation function's result.

#### The inference, in detail

**The assumption it rests on.** `Conn_Duration` is *physical elapsed time*, so it spans the true
start and the true end of the connection. This is what makes the inference possible. Were
`Conn_Duration` instead a naive subtraction of local clock values, a session spanning the fold would
under-report by exactly the repeated hour, and the reported end could not distinguish the two
candidate offsets from each other.

Note the assumption holds of the *true* instants, not of the reported ones. Because the report
truncates start and end to whole minutes, `Conn_start_UTC + Conn_Duration` does not land on
`Conn_end_UTC` — it misses by strictly less than one `SESSION_BOUNDARY_RESOLUTION`, in either
direction, on a perfectly sound record.
Every test below is stated as a tolerance for that reason, and the exact size of the discrepancy is
derived in step 2.

**The procedure**, applied to `Conn_DateTime_Start`:

1. If the local time maps to exactly one instant, use it. This is every timestamp except during the
   two transitions each year.
2. If it falls in the **fold** — the repeated 01:00:00-01:59:59 hour — there are two candidate
   instants, one at the EDT offset (UTC-4) and one at the EST offset (UTC-5). Take each candidate
   in turn, add `Conn_Duration`, convert back to local time, and check whether the result matches
   the reported `Conn_DateTime_End`. **A candidate matches when the two are less than 60 seconds
   apart**, not when they are equal. Both reported timestamps are truncated to the whole minute
   while `Conn_Duration` carries seconds, so for a consistent record `Conn_start + Conn_Duration`
   lands within a minute of the reported end *on either side*: writing the true start as
   `S + α` and the true end as `E + β` with `α, β ∈ [0, 60)`, the implied end is `E + (β − α)`.
   Demanding equal minutes therefore rejects every record with `β < α` — roughly half of them, and
   116 of the 238 rows in this project's `data` directory. The tolerance cannot blur the two
   candidates together: they lie a full hour apart.

   The comparison is made on *local wall time*, which is what lets both candidates match a session
   short enough to fit inside the repeated hour — the very ambiguity being tested for. It must also
   stay two-sided: a one-sided test would accept a candidate landing an hour *early* and duplicate a
   session that is not ambiguous at all.
   - *Exactly one candidate matches* — that offset is the session's; the ambiguity is resolved.
   - *Both candidates match* — the reported end cannot discriminate, so the record is duplicated
     per the policy above. This is precisely the "duration of less than 1 hour" case: both
     candidates agree exactly when the session is short enough to end inside the repeated hour.
     Note it is *derived* from the test rather than applied as a hardcoded 1-hour threshold.
   - *Neither candidate matches* — the record is internally inconsistent. The earlier (EDT) offset
     is assumed and the row is reported.
3. If it falls in the **gap** — the 02:00:00-02:59:59 hour skipped when DST begins, a wall time that
   never occurred — the instant is resolved forward to just after the gap, and the row is reported.
   Such a timestamp indicates a fault upstream; it is surfaced rather than silently accepted.

`Conn_DateTime_End` is resolved the same way, except that a fold is settled by taking whichever
candidate is nearer to `Conn_start_UTC + Conn_Duration`, which is by then already known.

**Duplicated records** are given distinct ids — `<id>-EDT` and `<id>-EST` — because the peak power
contribution logic keys `Session` on its id alone and holds sessions in a `BTreeSet`. With identical
ids the second copy would be silently discarded on insertion, defeating the purpose of duplicating
it. Note also that **both copies carry the full `Energy_Use`**, so a duplicated session contributes
to the peak in both candidate hours.

### Boundaries and the time grid

Half-open is what makes session groups properly cover all sessions over the interval of interest without overlaps between groups: consecutive groups meet at a single instant that belongs to the later one, so no instant falls in two groups. Closed intervals (i.e., the end is included) cannot do this — adjacent groups would either share an instant, and so disagree about which sessions were active at it, or leave a one-tick gap. It is also what makes *abutting* distinguishable from *overlapping*, which is significant for the estimates.

The padding is a full `R` rather than one tick less for the same reason. A session reported to end at `16:34` truly ended somewhere in `[16:34:00, 16:35:00)`, so `16:35:00` — exclusive — is the bound that contains it wherever it fell.

**The time grid** is a consequence rather than a rule imposed on the software. Reported start and end times lie on the `R` grid; `Adj_conn_end` adds exactly one `R`, so it lies on it too; and an end-point clamped into the interval of interest lands on one of `I`'s own bounds, which are multiples of 15 minutes. Every group boundary therefore lies on the `R` grid, and every group duration is a multiple of `R` — **provided `R` divides 15 minutes**. That last is a requirement on the *report format*, not on this software, and the current 60 seconds satisfies it.

The [skew margin intervals](#clock-skew-and-drift) do not disturb any of this. Their bounds are `I`'s own bounds offset by `CLOCK_SKEW_MARGIN`, which is a whole multiple of `R` by construction, so they lie on the same grid and the argument above carries over to them unchanged.

### Dubious groups

Every member of a group runs the group's whole span, in the only sense the grouping can use: a session is taken to be active over `[conn_start, adj_conn_end)`. That is what a group is: a stretch of time over which the set of active sessions does not change, whose members are the sessions active throughout it. A session active at every instant of `[g.start, g.end)` must start at or before `g.start` and end at or after `g.end` — so `conn_start <= g.start` and `adj_conn_end >= g.end` for every member.

Neither `conn_start` nor `adj_conn_end` is an instant the session actually started or stopped at. `conn_start` is the true start truncated down to the minute, and `adj_conn_end` is one `R` past `conn_end`, which is the true end truncated down the same way. Each of the two therefore stands for a whole minute, with the true instant lying somewhere in it. Where in that minute it lies is what the rest of this section is about — and a member's true span need not cover all of `g`.

A group of duration exactly `R` is called ***narrow***. Only a narrow group can be dubious, as shown at the end of this section, so take `g` to be narrow from here on.

Each of the above start and end comparisons holds either with equality or as a strict inequality, and the difference is what the rest of this section turns on. Reading them on the grid established above:

- **`adj_conn_end == g.end`.** The true end lies in `[adj_conn_end - R, adj_conn_end)`, which for a narrow group *is* `[g.start, g.end)`. The session stopped somewhere inside `g` and the report does not say where. It did draw power somewhere in `g`, though: the reported end is the minute the session was last drawing in, truncated down (see [Assumptions](#assumptions)), and for a narrow group that minute is `g` itself. Had the session been off for all of `g`, the report would have named an earlier minute.
- **`adj_conn_end > g.end`.** Then `adj_conn_end` is at least `g.end + R`, so the true end lies at or after `g.end`. The session did not stop inside `g`: wherever in its last minute it truly stopped, that was at or after `g.end`.
- **`conn_start == g.start`.** The true start lies in `[g.start, g.start + R)`, again `g` itself. Inside the group, position unknown.
- **`conn_start < g.start`.** Then `conn_start` is at most `g.start - R`, so the true start lies before `g.start`. The session was already drawing power when `g` began.

A strict inequality therefore *settles* that end of the session: the true instant falls outside `g` — before `g.start` for a start, at or after `g.end` for an end — so on that side the session covers `g` right up to the boundary, and no truncation can pull it inside. Only an equality leaves an end-point loose inside the group. A session needs both inequalities strict to cover the group outright. Two comparisons, two ways each, gives four classes, whose names say where the true start and the true end lie — **b** before the group, **i** inside it, **a** after it:

| set | `conn_start` | `adj_conn_end` | true start | true end |
| ------------- | ------------ | ---------- | --------------------- | --------------------- |
| `ba_sessions` | `< g.start`  | `> g.end`  | before `g.start`      | at or after `g.end`   |
| `bi_sessions` | `< g.start`  | `== g.end` | before `g.start`      | inside `g`, where unknown |
| `ia_sessions` | `== g.start` | `> g.end`  | inside `g`, where unknown | at or after `g.end` |
| `ii_sessions` | `== g.start` | `== g.end` | inside `g`, where unknown | inside `g`, where unknown |

The above comparisons read each session's **own** `conn_start` and `adj_conn_end` — never `conn_end`, whose padding has not yet been applied, and never the end-points clamped into the interval of interest, which would make a session running through `I`'s edge look like one confined to the group.

`ba_sessions` have nothing loose in them: their true start is before `g.start` and their true end at or after `g.end`, so they cover `g` whatever the truncation hid. Every other session category has at least one end-point free to "move" inside `g` and we refer to such sessions as "movable". That "freedom to move" is the whole source of doubt — which pairs of them can be arranged not to overlap is what decides whether the group is dubious:

| pair | can be disjoint | why |
| ------------- | --- | ---------------------------------------------- |
| `bi` & `ia`   | yes | one ends inside, the other starts inside |
| `bi` & `ii`   | yes | one ends inside, the other floats freely inside `g` |
| `ia` & `ii`   | yes | one starts inside, the other floats freely inside `g` |
| `ii` & `ii`   | yes | both float freely inside `g` |
| `bi` & `bi`   | no  | both start before `g` and still draw in it, so both run at `g.start` |
| `ia` & `ia`   | no  | both run past `g.end`, so both run just before it |
| `ba` & any    | no  | `ba` covers all of `g` |

Intuitively, a group is ***dubious*** exactly when it is narrow and has at least two "movable" members that are not anchored to the same end of the group. Formally, a narrow group is dubious if and only if:
- `bi_sessions` and `ia_sessions` are both non-empty, or
- `ii_sessions` is non-empty and the group has a further member outside `ba_sessions` — a second `ii` member, or a `bi` or `ia` one.

This condition is equivalent to `min_size < size`, which is how the software checks it; it is derived from the `min_size` formula in [`min` and `max` estimates for dubious groups](#min-and-max-estimates-for-dubious-groups) below. A narrow group whose movable members all sit in one of `bi_sessions` or `ia_sessions` is **not** dubious, and neither is one all of whose members span it — which is possible, since a group's two boundaries can both be created by sessions that are not its members.

**No group longer than `R` can be dubious.** Group durations are multiples of `R`, so a group that is not narrow is at least `2R` long. Every member's true start is before `g.start + R`, and every member's true end is at or after `g.end - R`, which is at or after `g.start + R`. Take `t` to be the latest true start among the members: `t` is at or after every true start, and `t < g.start + R <=` every true end, so every member is running at `t`. No pair can be disjoint, whatever the true times turn out to be.

### `min` and `max` estimates for dubious groups

The `max` estimates for a dubious group are the `nominal` estimates themselves.

The `min` estimates for a dubious group `g` spanning the interval `[g.start, g.end)` are defined in terms of a thought experiment:

- Take `[g.start, g.end)` as the interval of interest in an arbitrarily fine time grid.
- For each session `s` participating in group `g`, define a legal nudged version of `s`, by the class `s` falls in above:
  - `ba_sessions`: its end-points remain unchanged.
  - `bi_sessions`: let `nudge(s, ε)` be the modification of `s` by adding a small positive or negative `ε` to `s.adj_conn_end` while keeping the nudged session ending within `g` and after `g.start` — it was still drawing power somewhere in `g`, as shown in the previous section, so it cannot be nudged out of `g` altogether.
  - `ia_sessions`: let `nudge(s, ε)` be the modification of `s` by adding a small positive or negative `ε` to `s.conn_start` while keeping the nudged session starting within `g`.
  - `ii_sessions`: let `nudge(s, ε1, ε2)` be the modification of `s` by adding small positive or negative `ε1` and `ε2` to `s.conn_start` and `s.adj_conn_end`, respectively, while keeping the nudged session within `g`.
- Obtain a legal nudged version of each of the participating sessions and apply the `nominal` estimating algorithm to them with `[g.start, g.end)` as the interval of interest, producing a pair `(max_kw, max_size)`.
- The `min` estimates for `g`:
  - The `min_agg_avg_kw` estimate is the minimum, over all possible legal nudge combinations, of the `max_kw` component of the above pairs.
  - The `min_size` estimate is the minimum, over all possible legal nudge combinations, of the `max_size` component of the above pairs.

The minimum is attained rather than merely approached: only the pattern of which sessions overlap affects the result, and there are finitely many such patterns.

The computations of the `min` and `max` estimates for `g` are straightforward. Write `ba_sessions_avg_kw` for the sum of `avg_kw` over `ba_sessions` and `ba_sessions_size` for its size, and likewise for the other three sets.

- `max` is the result of the `nominal` estimates, produced without regard for `g`'s dubious nature.
- `min_agg_avg_kw` is `ba_sessions_avg_kw` plus the maximum of the following:
  - `bi_sessions_avg_kw`.
  - `ia_sessions_avg_kw`.
  - maximum `avg_kw` over `ii_sessions`.
- `min_size` is `ba_sessions_size` plus the maximum of the following:
  - `bi_sessions_size`.
  - `ia_sessions_size`.
  - `1` if `ii_sessions` is non-empty, `0` otherwise.

The `ba_sessions` term is unconditional because those sessions certainly run throughout `g`; the maximum over the other three is the best arrangement available, with every `bi` member ending as early inside `g` as it can, every `ia` member starting as late as it can, and the `ii` members spread out between them. Both figures are bounded above by the corresponding `max`, since each is a sum over a subset of the same members, so `min_overlap <= nominal` follows for the whole estimate set.

**The dubiousness condition follows from the `min_size` formula.** Write `n_ba`, `n_bi`, `n_ia`, `n_ii` for the four class sizes, and `[n_ii > 0]` for `1` when `ii_sessions` is non-empty and `0` otherwise. Then `size` is `n_ba + n_bi + n_ia + n_ii` and `min_size` is `n_ba + max(n_bi, n_ia, [n_ii > 0])`. The `n_ba` term is on both sides and cancels:

```
min_size < size  <=>  max(n_bi, n_ia, [n_ii > 0])  <  n_bi + n_ia + n_ii
```

- **`ii_sessions` empty.** The condition reads `max(n_bi, n_ia) < n_bi + n_ia`. If either class is empty the maximum *is* the whole sum, and the group is not dubious; if both are non-empty the sum strictly exceeds the maximum, and it is. That is the first clause of the condition stated [above](#dubious-groups).
- **`ii_sessions` non-empty with nothing else outside `ba_sessions`** — that is, `n_ii == 1` and `n_bi == n_ia == 0`. Both sides are `1`, so the group is not dubious: a single member loose inside `g` has nothing to fail to overlap.
- **`ii_sessions` non-empty with a further member outside `ba_sessions`.** The right-hand side then exceeds the left by at least one. If `n_ii >= 2`, the `ii` members alone account for the `[n_ii > 0]` term and contribute at least one more than it. If `n_ii == 1` and one of `n_bi`, `n_ia` is positive, the maximum is that class's own size and the remaining two terms add at least one. Either way the group is dubious, which is the second clause.

Note that `min_size == size` therefore holds in exactly four shapes: no movable members at all, `bi` members only, `ia` members only, or a single `ii` member. In each of them the `min_agg_avg_kw` formula reduces to `ba_sessions_avg_kw` plus the whole movable sum — the same value as `max` — so the power figure never falls unless the size figure does. That is why the software decides dubiousness on the size figure alone. The converse does not hold: a dubious group whose surplus members draw no power reports the same kW in both readings, and the report then withholds the second estimate set only if all four figures agree.

### Clock skew and drift

The interval of interest comes from Toronto Hydro's metering data and the session times from Evolute, and nothing reconciles the two clocks. The two may also drift apart over the reporting period. Write **`M`** below for `CLOCK_SKEW_MARGIN`, derived at the end of this section.

**What is uncertain is which window of Evolute time the bill's interval names.** If the meter's clock leads or lags Evolute's by `δ`, the interval really at issue is `[I.start + δ, I.end + δ)` for an unknown `δ` with `|δ| <= M`. That shifted window — not any fixed interval — is the thing the estimates have to answer for.

Every such window lies inside `[I.start - M, I.end + M)`, which is `I` together with the two ***skew margin intervals*** `[I.start - M, I.start)` and `[I.end, I.end + M)`. The peak over a union is the highest of the peaks over its parts, so computing the estimates for the two margins alongside `I` bounds every candidate window at once, without anyone having to know `δ`.

The bound is deliberately loose. It admits an instant from the left margin and one from the right that no single `δ` could ever bring into view together, so a reported margin figure says the peak *could* have fallen just outside the metered window, not that any one shift puts it there. The tight alternative — evaluating each shifted window `I + δ` in turn, a finite family since all boundaries lie on the `R` grid — buys little at the margin sizes in play.

The same grouping and estimation machinery is applied to each margin interval; none of it is special-cased for them. In particular, a margin is not assumed to contain a single group, though at the current `M` it always does: a margin is then exactly one grid cell, and no session end-point can fall strictly inside one.

**The margin** is derived from an assumed bound, never from a measured skew — no measurement is available:

```
CLOCK_SKEW_BOUND  = 5s
CLOCK_SKEW_MARGIN = R * ceil(CLOCK_SKEW_BOUND / R)   (currently one R)
```

Rounding *up to a whole* `R` is what keeps `I.start - M` and `I.end + M` on the `R` grid — see [Boundaries and the time grid](#boundaries-and-the-time-grid). A margin of `5s` would put both off it, and group durations would stop being multiples of `R`. Taking the greater of the bound and `R` would do as well while the bound stays under `R`, and silently stop working above it.

That `M` currently equals `R` is arithmetic, not identity. The two are different quantities measuring different things: truncation is one-sided and forward, and applies to the reported session times; skew is two-sided, and applies to the interval's end-points. `R` is a floor on the margin because of the grid, not because the two are the same kind of thing. Raising `CLOCK_SKEW_BOUND` past `R` widens the margins and changes nothing else — the cost is conservatism, since each end then admits more time that the meter never counted.

The bound itself is an assumption, and an unverifiable one; see [Assumptions](#assumptions).

### Assumptions

- **Session end times are truncated, not rounded.** `Adj_conn_end = Conn_DateTime_End + R` is the exclusive bound of the window the true end lies in only because the reported end is the true end rounded *down* to `R`. Under rounding to nearest, or under a convention where the reported end is the first instant the vehicle was no longer drawing power, the correct padding would differ — in the latter case it would be zero. The resolution `R` and the padding are two separate facts about the report, and only the first is settled by observation; `Questions_for_Evolute.md` carries the outstanding question about the second.
- **Breaker ratings are uniform across panels.** `breaker_spec_based_kw` and `breaker_spec_based_kva` are a session count multiplied by a single rating, so an installation mixing breakers of different ratings would skew both. Nothing else in the estimates depends on how many panels there are or on which panel a session ran: the session report carries no panel ID, and none is needed.
  - A session whose own average power exceeds that rating contradicts the assumption directly, and is flagged `ExcessiveAvgPower`. It is not excluded — the figure says something is wrong with `Energy_Use` or `Active_Charge_Time`, not which — but it is worth knowing about, because a group whose aggregate average power exceeds its member count times the rating would put `consumption_based_kw` *above* `breaker_spec_based_kw` and invert the bracket. The comparison is against the rating exactly, with no tolerance, which makes the flag a complete account of that inversion: a group can only invert if one of its members draws more than the rating, and every such member is flagged.
- **Clock skew and drift.** The maximum absolute skew between the Toronto Hydro and Evolute clocks, plus the sum of each clock's absolute drift over the reporting period, is assumed to be no more than `CLOCK_SKEW_BOUND`, currently `5s`. Unlike the assumption above it, this one is unverifiable rather than merely unverified: Evolute's clock discipline is undocumented, and Toronto Hydro's is not ours to ask about. Nothing downstream depends on the figure itself, only on its staying below `R` — a larger bound widens the [skew margins](#clock-skew-and-drift), which costs conservatism but breaks nothing.

### Other

- Every session in the report is written to the workbook, anomalous ones included: the sheet is a faithful rendering of the session report, and which sessions take part in an estimate is decided on the reading side.
- Each session is checked for internal consistency, and the test is *derived* rather than chosen. Truncation puts the true start somewhere in `[Conn_start, Conn_start + R)` and the true end somewhere in `[Conn_end, Adj_conn_end)` — two half-open windows one `R` wide, the same convention used everywhere else. An honest `Conn_Duration` carries some instant of the first window to some instant of the second, so the record is sound exactly when the first window, shifted by the duration, still *meets* the second:

  ```
  sound  <=>  Conn_start + Conn_Duration  <  Adj_conn_end
         and  Conn_start + Conn_Duration  >  Conn_end - SESSION_BOUNDARY_RESOLUTION
  ```

  Both bounds are strict, because both windows are half-open at the same end. That makes this the one band in the design that is open rather than half-open — an instance of the convention rather than an exception to it, since it is the *intersection* condition of two half-open windows and not an interval anyone chose. The band is not slack: it is precisely what truncation to whole minutes accounts for, and the sample data reaches to within 3 seconds of its lower edge.
- A session outside that band is flagged `InconsistentDuration` and excluded from the estimates. Both directions are faults: if a record's own fields disagree by more than the reporting can explain, neither its duration nor the span the grouping logic would place it on can be relied on. The overshoot direction subsumes the case of a session ending before it starts, since with `Conn_DateTime_End` a minute or more before `Conn_DateTime_Start` no non-negative duration can satisfy the test.
- These are the *only* sessions excluded from the estimates. Nothing else removes a session: the doubt described under [Dubious groups](#dubious-groups) changes which figures a group reports, never which sessions belong to it.
- Excluded sessions get a section of their own in the report, listing **every** one in the workbook rather than only those near the interval of interest, with a column saying whether each appears to fall in that interval. Appears only: a record whose own fields contradict each other cannot be trusted to say which window it belongs in, so filtering on that judgement could hide exactly the session a reader most needs to see.
- Sessions with zero `Energy_Use` and non-zero `Active_Charge_Time` do not contribute to `consumption_based_kw` and `consumption_based_kva` but they do contribute to `breaker_spec_based_kw` and `breaker_spec_based_kva`.
- A session with zero `Active_Charge_Time` delivered energy in no time at all, so its average power is unbounded or undefined.
  - The Excel `Avg_power` cell shows `#DIV/0!` — the formula is written on every row rather than being skipped, so the fault is visible in the sheet. Function `session_list` returns the session as a *spike*, held apart from the normal sessions fed to the peak logic.
  - Spikes are worth reviewing individually for their effect on the building's demand charge.
  - The power estimating logic treats spikes as follows:
    - If `Energy_Use == 0`, set `Avg_power` to 0. These sessions do not contribute to `consumption_based_kw` and `consumption_based_kva` but they do contribute to `breaker_spec_based_kw` and `breaker_spec_based_kva`.
    - Otherwise, set `Avg_power` to the constant `EVOLUTE_BREAKER_KW_RATING`. These sessions contribute to all four estimate types.
