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

Two binaries, matching the workflow steps:

| Command                                                      | Purpose                                                      |
| ------------------------------------------------------------ | ------------------------------------------------------------ |
| `csv_to_xlsx <SESSION_REPORT.csv>...`                        | Converts a session report to a workbook, computing the derived columns and flagging rows that need review. |
| `estimates <SESSION_REPORT.xlsx> <YYYY-MM-DD HH:MM [EST\|EDT]> [15m\|1h]` | Prints the peak estimate report for one interval of interest. |

`estimates` takes the interval start in **local time (ET)**. The length defaults to `1h` when the start is on the hour and `15m` otherwise. An interval breaking the boundary rules described earlier is rejected rather than estimated.

The two DST transitions are treated differently, because they are different problems.

- On the night DST **ends**, an hour of wall time occurs twice. That is a question the caller can answer, so `estimates` asks it: a bare `"2026-11-01 01:30"` is refused, and `"2026-11-01 01:30 EST"` or `"... EDT"` resolves it. The designator is accepted on any date and **checked against it** — `"2026-06-01 16:00 EST"` is an error, not a silently ignored hint — so naming the wrong one cannot produce a figure for the wrong hour.
- On the night DST **begins**, an hour of wall time never happens. There is nothing to choose between, so such a start is refused outright and no designator helps.

## Excel workbook

The conversion from CSV to Excel includes the addition of new fields:

- `Adj_conn_end`, is computed as: `Conn_DateTime_End + 60 seconds`. It is the session's **exclusive** end: a session starting at exactly this time does not overlap this one.
- `Adj_conn_duration`, is computed as: `Adj_conn_end - Conn_DateTime_Start`.
- `Conn_start_UTC`, `Conn_end_UTC`, and `Adj_conn_end_UTC`, with UTC values corresponding to the corresponding local time fields.
- `Avg_power` in kW, is computed as: `Energy_Use / (Active_Charge_Time * 24)`.
- `Anomalies`, containing a comma-separated list of `AnomalyKind` variant names, is added as the last column.

None of the data in the Excel workbook (or the source CSV) should be modified by the user, as any changes would impact and possibly invalidate the estimates.

## Estimation logic

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
- These four values are the **`direct`** estimates: computed from the `SessionGroup`s exactly as the report gives them. A second set, the **`narrow`** estimates, is reported *only* when there is at least one [narrow](#narrow-groups) group.
- The software detects data anomalies in the reported session data. Anomalies associated with every session that **intersects `I`** are reported alongside the estimates, as well as anomalies that caused sessions to be excluded from the analysis. Other sessions elsewhere in the workbook are not included in report.

#### Session and group boundaries

Sessions, `SessionGroup`s, and intervals of interest are treated as half-open intervals which include the left end but exclude the right end. Because reported session start and end times are currently truncated to whole minutes, this software calculates an adjusted session end time, **`Adj_conn_end`**, by adding `SESSION_BOUNDARY_RESOLUTION` (currently 60 seconds) to the reported end time, to ensure the actual charge time is fully included between the session's start (inclusive) and end (exclusive).

Like sessions, group boundaries are constrained to lie on a **time grid** aligned to multiples of `SESSION_BOUNDARY_RESOLUTION`.

Half-open is what makes session groups properly cover all sessions over the interval of interest without overlaps between groups: consecutive groups meet at a single instant that belongs to the later one, so no instant falls in two groups. Closed intervals (i.e., the end is included) cannot do this — adjacent groups would either share an instant, and so disagree about which sessions were active at it, or leave a one-tick gap. It is also what makes *abutting* distinguishable from *overlapping*, which is significant for the estimates.

The padding is 60 seconds rather than 59 for the same reason. A session reported to end at `16:34` truly ended somewhere in `[16:34:00, 16:35:00)`, so `16:35:00` — exclusive — is the bound that contains it wherever it fell.

#### Narrow groups

If a group's duration is exactly `SESSION_BOUNDARY_RESOLUTION`  then its membership and size are ambiguous.

- In order for the group to truly exist, it must contains at least one session `s1` that ends inside the group and at least one session `s2` that starts inside the group.
- If the true end of `s1` is less than the true start of `s2` then they don't overlap, so the group size overstates the number of concurrent sessions in the group.

-  If the above condition holds and `s1` and `s2` are the only sessions in the group, then the group could conceptually be split into two subgroups occupying the same place on the time grid -- one subgroup containing just `s1` and the other containing just `s2`.

Groups of duration `SESSION_BOUNDARY_RESOLUTION` are called ***narrow*** groups and need special treatment in the software, which will report two estimates for the group: one corresponding to the case where the group's membership is taken at face value and there is maximum possible session overlap (designated the **`max`** case) and the other corresponding to the case where there is minimum possible session overlap (designated the **`min`** case).

For groups of duration greater than `SESSION_BOUNDARY_RESOLUTION`, there is no ambiguity regarding session membership and size.

#### The two possible estimate sets

An  ***estimate set*** consists of the following values: `consumption_based_kw`, `consumption_based_kva`, `breaker_spec_based_kw`, and `breaker_spec_based_kva`.

If there are no narrow groups, only one estimate set is given.

If there is at least one narrow group then two estimate sets are given:

- **`direct`**:  which uses the `max` estimates for the narrow groups;
- **`narrow`**:  which uses the `min` estimates for the narrow groups.

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

### `min` and `max` estimates for narrow groups

The `max` estimates for a narrow group are the `direct` estimates themselves.

The `min` estimates for a narrow group `g` spanning the interval `[g.start, g.end)` are defined in terms of a thought experiment:

- Take `[g.start, g.end)`as the interval of interest in an arbitrarily fine time grid.
- For each session `s` participating in group `g`, define a legal nudged version of `s`:
  - If `s` starts before `g` and `s.conn_end <= g.end`: let`nudge(s, ε)` be the modification of `s` by adding a small positive or negative `ε` to `s.conn_end` while keeping the nudged session ending within `g`.
  - If `s` ends after `g` and `s.conn_start >= g.start`: let `nudge(s, ε)` be the modification of `s` by adding a small positive or negative `ε` to `s.conn_start` while keeping the nudged session starting within `g`.
  - If `s` lies entirely within `g`, i.e., `s.conn_start >= g.start && s.conn_end <= g.end`: let `nudge(s, ε1, ε2)` be the modification of `s` by adding small positive or negative `ε1` and `ε2` to `s.conn_start` and `s.conn_end`, respectively, while keeping the nudged session within `g`.
- Obtain a legal nudged version of each of the participating sessions and apply the `direct` estimating algorithm to them with`[g.start, g.end)` as the interval of interest, producing a pair `(max_kw, max_size)`.
- The `min` estimates for `g`:
  - The `min_agg_avg_kw` estimate is the minimum, over all possible legal nudge combinations, of the `max_kw` component of the above pairs.
  - The `min_size` estimate is the minimum, over all possible legal nudge combinations, of the `max_size` component of the above pairs.

The computations of the `min` and `max` estimates for `g` are straightforward:

- `max` is the result of the direct estimates produced without regard for `g`'s narrow nature.
- `min_agg_avg_kw` is the maximum of the following:
  - Sum of `avg_kw` over all sessions that start before `g` and end in `g`.
  - Maximum of `avg_kw` over all sessions that start and end in `g`.
  - Sum of `avg_kw` over all sessions that start in `g` and end after `g`.
- `min_size` is the maximum of the following:
  - Count of sessions that start before `g` and end in `g`.
  - `1` if there are sessions that start and end in `g`, `0` otherwise.
  - Count of sessions that start in `g` and end after `g`.

### Other

- Every session in the report is written to the workbook, anomalous ones included: the sheet is a faithful rendering of the session report, and which sessions take part in an estimate is decided on the reading side.
- Each session is checked for internal consistency, and the test is *derived* rather than chosen. Truncation puts the true start somewhere in `[Conn_start, Adj_conn_start)` and the true end somewhere in `[Conn_end, Adj_conn_end)` — two half-open windows one `SESSION_BOUNDARY_RESOLUTION` wide, the same convention used everywhere else. An honest `Conn_Duration` carries some instant of the first window to some instant of the second, so the record is sound exactly when the first window, shifted by the duration, still *meets* the second:

  ```
  sound  <=>  Conn_start + Conn_Duration  <  Adj_conn_end
         and  Conn_start + Conn_Duration  >  Conn_end - SESSION_BOUNDARY_RESOLUTION
  ```

  Both bounds are strict, because both windows are half-open at the same end. That makes this the one band in the design that is open rather than half-open — an instance of the convention rather than an exception to it, since it is the *intersection* condition of two half-open windows and not an interval anyone chose. The band is not slack: it is precisely what truncation to whole minutes accounts for, and the sample data reaches to within 3 seconds of its lower edge.
- A session outside that band is flagged `InconsistentDuration` and excluded from the estimates. Both directions are faults: if a record's own fields disagree by more than the reporting can explain, neither its duration nor the span the grouping logic would place it on can be relied on. The overshoot direction subsumes the case of a session ending before it starts, since with `Conn_DateTime_End` a minute or more before `Conn_DateTime_Start` no non-negative duration can satisfy the test.
- These are the *only* sessions excluded from the estimates. The boundary margin described under *Session boundaries* flags a session rather than excluding it: a doubtful session still counts in `direct`, and only the `narrow` sets leave it out.
- Excluded sessions get a section of their own in the report, listing **every** one in the workbook rather than only those near the interval of interest, with a column saying whether each appears to fall in that interval. Appears only: a record whose own fields contradict each other cannot be trusted to say which window it belongs in, so filtering on that judgement could hide exactly the session a reader most needs to see.
- Sessions with zero `Energy_Use` and non-zero `Active_Charge_Time` do not contribute to `consumption_based_kw` and `consumption_based_kva` but they do contribute to `breaker_spec_based_kw` and `breaker_spec_based_kva`.
- A session with zero `Active_Charge_Time` delivered energy in no time at all, so its average power is unbounded or undefined.
  - The Excel `Avg_power` cell shows `#DIV/0!` — the formula is written on every row rather than being skipped, so the fault is visible in the sheet. Function `session_list` returns the session as a *spike*, held apart from the normal sessions fed to the peak logic.
  - Spikes are worth reviewing individually for their effect on the building's demand charge.
  - The power estimating logic treats spikes as follows:
    - If `Energy_Use == 0`, set `Avg_power` to 0. These sessions do not contribute to `consumption_based_kw` and `consumption_based_kva` but they do contribute to `breaker_spec_based_kw` and `breaker_spec_based_kva`.
    - Otherwise, set `Avg_power` to the constant `EVOLUTE_BREAKER_KW_RATING`. These sessions contribute to all four estimate types.
