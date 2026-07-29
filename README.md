# Contribution of EVs to Building's Peak Power Consumption

This software supports the estimation of the impact of EV charging on the building's peak power demand. Peak kW and kVA are used by Toronto Hydro to calculate distribution and transmission charges.

## Conceptual Approach

### Data sources and intervals of interest

For a given billing period, we can identify the time intervals in which the peak kW and kVA occurred based on metering data downloads from Toronto Hydro.

Given a time interval of interest, this software estimates the peak kW and kVA demand associated with EV charging activity during the interval. The data source for EV power demand is the Evolute monthly session report.

**Interval of interest boundaries** are constrained as follows:

- The left and right end-points are always of the form HH:00:00 or HH:15:00 or HH:30:00 or HH:45:00.
- The difference between the right end-point and the left end-point can be either:
  - 1 hour -- only if the left end-point is of the form HH:00:00.
  - 15 minutes -- in all four cases.
- The interval is half-open: it includes the left end-point and excludes the right end-point.

### Estimation logic

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
- These four values are the **`direct`** estimates: computed from the `SessionGroup`s exactly as the report gives them. A second, **`clamped`** set is reported *only* when some `SessionGroup` exceeded a single panel's concurrency limit, since otherwise it would repeat `direct` exactly. See *Limitations*.
- Every anomaly carried by every session that **intersects `I`** is reported alongside the estimates, including those of sessions excluded from them — an estimate is not interpretable without knowing what was left out of it. Sessions elsewhere in the workbook are not reported: the workbook covers a whole billing period while an estimate covers one window in it, and an unrelated finding three weeks away would only bury the ones that bear on this figure.

## Workflow

This is the typical workflow used with this software to estimate the impact of EV charging activity on a particular Toronto Hydro bill:

- Preliminary steps (out of scope for this software):
  - Download Toronto Hydro metering data for the time period of interest.
  - Based on the downloaded data, identify the interval(s) of interest during which the billing period's peak kW and/or peak kVA occurred.
  - Obtain the *session report* file from Evolute covering the interval(s) of interest.
- Using this software:
  - Transform the relevant Evolute *session report* CSV file to an Excel file. The transformation process includes some data validation and computes additional columns that are included in the Excel file.
  - Access the relevant Excel file and compute the peak kW and kVA brackets for the interval(s) of interest.

### Tools

Two binaries, matching the workflow steps:

| Command | Purpose |
|---|---|
| `csv_to_xlsx <SESSION_REPORT.csv>...` | Converts a session report to a workbook, computing the derived columns and flagging rows that need review. |
| `estimates <SESSION_REPORT.xlsx> <YYYY-MM-DD HH:MM [EST\|EDT]> [15m\|1h]` | Prints the peak estimate report for one interval of interest. |

`estimates` takes the interval start in **local time (ET)**. The length defaults to `1h` when the start is on the hour and `15m` otherwise. An interval breaking the boundary rules described earlier is rejected rather than estimated.

The two DST transitions are treated differently, because they are different problems.

- On the night DST **ends**, an hour of wall time occurs twice. That is a question the caller can answer, so `estimates` asks it: a bare `"2026-11-01 01:30"` is refused, and `"2026-11-01 01:30 EST"` or `"... EDT"` resolves it. The designator is accepted on any date and **checked against it** — `"2026-06-01 16:00 EST"` is an error, not a silently ignored hint — so naming the wrong one cannot produce a figure for the wrong hour.
- On the night DST **begins**, an hour of wall time never happens. There is nothing to choose between, so such a start is refused outright and no designator helps.

Both fall out of one test rather than being special-cased: read the wall time as if at each candidate offset, and keep the offsets the zone actually uses at the instant you land on. Two survivors means the caller must choose; one means it is settled; none means the time never existed.

Because a fold interval can begin at `01:00 EDT` and end at `01:00 EST` — the same clock reading, an hour apart — the report header names the offset at each end, and states it once when both agree.

The report goes to stdout as **markdown that also reads as plain text** — not every reader has a markdown renderer. So: no `#` headings (setext underlines instead), no emphasis markers, no indented blocks, and every table cell padded so the columns line up in a monospace font. Session ids get their own section rather than a table column, because a markdown table row is a single line and a large group cannot be wrapped inside one.

## Limitations

This software is written for a single Evolute panel. Such a panel holds 20 breakers, but its PLC runs a queued time-sharing algorithm that keeps at most **10** cars drawing power at any one instant. That figure is the constant `EVOLUTE_PANEL_MAX_CONCURRENT_SESSIONS`.

Provided that the smart breakers across all panels have the same kW/kVA ratings (specified as constants in the software), the software can be used with Evolute installations containing any number of panels. When there is more than one, however, the results may be distorted, because the session report carries no panel ID and the software therefore cannot tell whether two overlapping sessions ran on the same panel or on different ones.

To keep a single panel's estimate physically possible, a `SessionGroup` holding more than 10 sessions is **clamped**: the estimates are computed over 10 of them rather than all. Which 10 is decided in two tiers.

- A session ending within one `SESSION_BOUNDARY_RESOLUTION` of the group's start may not have overlapped the group at all. Reported times are truncated to minutes, so a session that in fact *abutted* the next one is indistinguishable from one that overlapped it. These **short-overlap** sessions are the first to be dropped, lowest average power first.
- Only if dropping every short-overlap session still leaves the group above 10 are **long-overlap** sessions dropped, again lowest average power first. Those demonstrably did overlap, so they go last.
- A short-overlap session can only arise in a group no longer than one `SESSION_BOUNDARY_RESOLUTION`, because every session in a group outlasts the group by construction. The first tier therefore fires exactly where the truncation artefact lives, and nowhere else.

The sessions themselves are never removed from the group: clamping affects only the derived figures, so a group's reported size stays truthful and carries the `ClampedSessionGroup` anomaly.

Because clamping rests on the single-panel assumption, the software does not choose. The **`direct`** estimates — computed from the groups as reported, with no panel constraint applied — are always given. The **`clamped`** estimates are given *in addition*, and only when some group actually exceeded the limit; where no group did, clamping changes nothing and a second identical set would say nothing.

A `clamped` set therefore carries information by its mere presence: it means the report claims more concurrent sessions than one panel can run, so either a second panel is installed or the data is wrong. The affected groups also carry the `ClampedSessionGroup` anomaly. When both sets are present the clamped figures never exceed the direct ones, so the two nest, and the widest honest bracket on the true peak runs from the clamped `consumption_based_kw` to the direct `breaker_spec_based_kw`.

Testing *any* group is the same as testing only the groups the `direct` estimates were drawn from, so nothing turns on the choice: the group behind `breaker_spec_based_kw` is by definition the largest there is, so if it is within the limit then every group is. A clamped group that is not one of the peaks cannot arise.

Clamping can still change *which* group peaks, but only when the peaking group is itself oversized: cutting it down may drop it below a group that was never clamped. So the two sets may point at different `SessionGroup`s, and each estimate names its own.

No report seen so far produces a `clamped` set at all; the June 2026 sample peaks at three concurrent sessions.

## Technical Notes

### Session boundaries

Sessions in the software, like intervals of interest and the session groups derived from them, are treated as half-open intervals which include the left end but exclude the right end. Because reported session start and end times are truncated to whole minutes, this software calculates an adjusted session end time, by adding `SESSION_BOUNDARY_RESOLUTION` — 60 seconds — to the reported end time, to ensure the actual charge time is fully included between the session's start and end.

Half-open is what makes session groups **tile** the interval of interest: consecutive groups meet at a single instant that belongs to the later one, so no instant falls in two groups and none falls in neither, and group durations sum to the interval's own. Closed intervals cannot do this — adjacent groups would either share an instant, and so disagree about which sessions were active at it, or leave a one-tick gap. It is also what makes *abutting* distinguishable from *overlapping*, which is the question the estimates turn on.

The padding is 60 seconds rather than 59 for the same reason. A session reported to end at `16:34` truly ended somewhere in `[16:34:00, 16:35:00)`, so `16:35:00` — exclusive — is the bound that contains it wherever it fell.

The interval of interest has a **boundary margin** equal to `SESSION_BOUNDARY_RESOLUTION`. Because reported session times are truncated to whole minutes, a session whose only overlap with the interval falls inside that margin cannot be trusted to overlap the interval at all, so it is excluded from the estimates and flagged `IntersectsBoundaryMarginOnly`. Equivalently: a session takes part only if it is active somewhere in the interval reduced by 60 seconds at each end.

The margin applies *only* at the boundaries. A session lying inside the interval is included however short. This includes any spikes, i.e., sessions whose `Active_Charge_Time` is zero. The margin also decides membership *only*: once a session is included, its `SessionGroup` end-points are clamped to the real interval, so the groups tile it and a reported peak window is a wall-clock window that can be matched against the metering data.

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

### New fields

- Session report session start and end times do not include seconds. Therefore, the following transformations are done during data ingestion:
  - Session start time `Conn_DateTime_Start` stays the same.
  - A new field, adjusted session end time `Adj_conn_end`, is computed as: `Conn_DateTime_End + 60 seconds`. It is the session's **exclusive** end: a session starting at exactly this time does not overlap this one.
  - A new field, adjusted session duration `Adj_conn_duration`, is computed as: `Adj_conn_end - Conn_DateTime_Start`.
  - Three new fields are added: `Conn_start_UTC`, `Conn_end_UTC`, and `Adj_conn_end_UTC`, with UTC values corresponding to the corresponding local time fields.
  - A new field, `Avg_power` in kW, is computed as: `Energy_Use / (Active_Charge_Time * 24)`.
  - A new field, `Anomalies`, containing a comma-separated list of `AnomalyKind` **variant names**, is added as the last column. This is a wire format, read back by `session_list`: it is how a judgement call made during ingestion reaches the power estimating logic. The `Display` strings are prose for humans and are deliberately not used here.

### Other

- Every session in the report is written to the workbook, anomalous ones included: the sheet is a faithful rendering of the session report, and which sessions take part in an estimate is decided on the reading side.
- Each session is checked for internal consistency, and the test is *derived* rather than chosen. Truncation puts the true start somewhere in `[Conn_start, Adj_conn_start)` and the true end somewhere in `[Conn_end, Adj_conn_end)` — two half-open windows one `SESSION_BOUNDARY_RESOLUTION` wide, the same convention used everywhere else. An honest `Conn_Duration` carries some instant of the first window to some instant of the second, so the record is sound exactly when the first window, shifted by the duration, still *meets* the second:

  ```
  sound  <=>  Conn_start + Conn_Duration  <  Adj_conn_end
         and  Conn_start + Conn_Duration  >  Conn_end - SESSION_BOUNDARY_RESOLUTION
  ```

  Both bounds are strict, because both windows are half-open at the same end. That makes this the one band in the design that is open rather than half-open — an instance of the convention rather than an exception to it, since it is the *intersection* condition of two half-open windows and not an interval anyone chose. The band is not slack: it is precisely what truncation to whole minutes accounts for, and the sample data reaches to within 3 seconds of its lower edge.
- A session outside that band is flagged `InconsistentDuration` and excluded from the estimates. Both directions are faults: if a record's own fields disagree by more than the reporting can explain, neither its duration nor the span the grouping logic would place it on can be relied on. The overshoot direction subsumes the case of a session ending before it starts, since with `Conn_DateTime_End` a minute or more before `Conn_DateTime_Start` no non-negative duration can satisfy the test.
- Together with the boundary margin described under *Session boundaries*, these are the only sessions excluded from the estimates.
- Sessions with zero `Energy_Use` and non-zero `Active_Charge_Time` do not contribute to `consumption_based_kw` and `consumption_based_kva` but they do contribute to `breaker_spec_based_kw` and `breaker_spec_based_kva`.
- A session with zero `Active_Charge_Time` delivered energy in no time at all, so its average power is unbounded or undefined.
  - The Excel `Avg_power` cell shows `#DIV/0!` — the formula is written on every row rather than being skipped, so the fault is visible in the sheet. Function `session_list` returns the session as a *spike*, held apart from the normal sessions fed to the peak logic.
  - Spikes are worth reviewing individually for their effect on the building's demand charge.
  - The power estimating logic treats spikes as follows:
    - If `Energy_Use == 0`, set `Avg_power` to 0. These sessions do not contribute to `consumption_based_kw` and `consumption_based_kva` but they do contribute to `breaker_spec_based_kw` and `breaker_spec_based_kva`.
    - Otherwise, set `Avg_power` to the constant `EVOLUTE_BREAKER_KW_RATING`. These sessions contribute to all four estimate types.
