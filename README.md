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
| `ev_estimate_cli <SESSION_REPORT.xlsx> <YYYY-MM-DD HH:MM [EST\|EDT]> [15m\|1h]` | Prints the peak estimate report for one interval of interest. |

`ev_estimate_cli` takes the interval start in **local time (ET)**. The length defaults to `1h` when the start is on the hour and `15m` otherwise. An interval breaking the boundary rules described earlier is rejected rather than estimated.

The two DST transitions are treated differently, because they are different problems.

- On the night DST **ends**, an hour of wall time occurs twice. That is a question the caller can answer, so `ev_estimate_cli` asks it: a bare `"2026-11-01 01:30"` is refused, and `"2026-11-01 01:30 EST"` or `"... EDT"` resolves it. The designator is accepted on any date and **checked against it** — `"2026-06-01 16:00 EST"` is an error, not a silently ignored hint — so naming the wrong one cannot produce a figure for the wrong hour.
- On the night DST **begins**, an hour of wall time never happens. There is nothing to choose between, so such a start is refused outright and no designator helps.

These rules live in one place, `src/interval.rs`, and both front-ends come through it, so the app and the command line cannot disagree about what interval a bill may be argued from.

## Excel workbook

The conversion from CSV to Excel includes the addition of new fields:

- `Adj_conn_end`, is computed as: `Conn_DateTime_End + TIME_GRID_STEP` (currently 60 seconds). It is the session's **exclusive** end: a session starting at exactly this time does not overlap this one.
- `Adj_conn_duration`, is computed as: `Adj_conn_end - Conn_DateTime_Start`.
- `Conn_start_UTC`, `Conn_end_UTC`, and `Adj_conn_end_UTC`, with UTC values corresponding to the corresponding local time fields.
- `Avg_power` in kW, is computed as: `Energy_Use / (Active_Charge_Time * 24)`.
- `Anomalies`, containing a comma-separated list of `AnomalyKind` variant names, is added as the last column.

None of the data in the Excel workbook (or the source CSV) should be modified by the user, as any changes would impact and possibly invalidate the estimates.

## Estimation logic

### Estimation algorithm overview

Given a time interval of interest **`I`** as described above, the estimation of EV peak power demand during the interval proceeds as follows:

- From the Evolute monthly session report, identify all charging sessions that intersect the interval of interest `I`.
- Partition `I` into 15-minute segments. If `I` is 1-hour long, there will be four segments. If `I` is 15-minutes long, there will only be one segment.
- For each segment:
  - Identify the charging sessions that intersect the segment.
  - For each session:
    - Compute the average power drawn by the session by dividing its energy consumed by the charge time in hours to obtain `avg_kw`.
    - Compute the overlap ratio `overlap_ratio` of the session over the segment's duration.
    - `avg_kw * overlap_ratio` is the session's contribution to the segment's aggregate kW and `overlap_ratio` is session's contribution to the segment's aggregate session count.

  - Compute the segment's aggregate kW `agg_kw` and `agg_count` by summing the above-described per-session contributions over all sessions.

  - From these two key values, compute the following ones:

    - **`energy_based_kw`**: `agg_kw`.

    - **`energy_based_kva`**: `agg_kw` divided by a power factor that reflects the combination of typical EV chargers and the Evolute infrastructure (~0.98).

    - **`count_based_kw`**: `agg_count` multiplied by the average per-EV kW rating of the Evolute infrastructure (~6.7 kW).

    - **`count_based_kva`**: `count_based_kw` divided by a power factor that reflects the combination of typical EV chargers and the Evolute infrastructure (~0.98).


- Identify the one or two *maximal* segments, i.e., segments that have the highest:

  - **`energy_based_kw`**: `agg_kw`.

  - **`count_based_kw`**: `agg_count` multiplied by the average per-EV kW rating of the Evolute infrastructure (~6.7 kW).
- The identified maximal segments are typically one and the same, but may be distinct in some situations.
- Report on the maximal segment(s).

- The software detects data anomalies in the reported session data. Anomalies associated with every session that **intersects `I`** are reported alongside the estimates, as well as anomalies that caused sessions to be excluded from the analysis. Other sessions elsewhere in the workbook are not included in the report.

### Sessions and segments

Sessions, segments, and intervals of interest are all **half-open**: each includes its left end-point and excludes its right one. Consecutive segments therefore meet at a single instant belonging to the later one, so no instant falls in two segments, and *abutting* stays distinguishable from *overlapping* — a distinction the estimates count on. See [Boundaries and the time grid](#boundaries-and-the-time-grid).

`TIME_GRID_STEP` — written **`R`** below, currently 60 seconds — is exactly the resolution at which the session report states session **start and end times**. It is not the resolution of everything in the report: `Conn_Duration` and `Active_Charge_Time` are stated more finely, and several of the Technical Notes depend on that difference.

A time stated to the minute is the true time truncated down to the minute, so a session reported to end at `16:34` truly ended somewhere in `[16:34:00, 16:35:00)`. The software therefore records an adjusted end, **`Adj_conn_end`**, one `R` past the reported end — the exclusive bound that contains the true end wherever in that minute it fell. That the report truncates rather than rounds is an assumption; see [Assumptions](#assumptions).

What truncation leaves behind is a residual doubt the estimates have to answer for. Where one session is reported to end in the same minute another is reported to start, the two may have genuinely overlapped for part of that minute, or may merely have abutted; the reported times cannot say which. Similarly, the same margin of uncertainty exists in the overlap of a session with the interval of interest or a segment.

#### Brackets

The software accounts for the above margin of uncertainty by providing values in *brackets*: the minimum value in the bracket and the maximum value in the bracket.

### Interval of interest with no EVs charging

In such cases, the EV charging infrastructure still impacts the overall building's peak kW and kVA, but the impact is small (currently ~ 0.35 kW and ~1.54 kVA for the transformer), and the software reports these values.

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
`Conn_end_UTC` — it misses by strictly less than one `TIME_GRID_STEP`, in either
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

Half-open is what makes segments properly cover all of the interval of interest without overlaps between them: consecutive segments meet at a single instant that belongs to the later one, so no instant falls in two segments. Closed intervals (i.e., the end is included) cannot do this — adjacent segments would either share an instant, and so disagree about which sessions were active at it, or leave a one-tick gap. It is also what makes *abutting* distinguishable from *overlapping*, which is significant for the estimates.

The padding is a full `R` rather than one tick less for the same reason. A session reported to end at `16:34` truly ended somewhere in `[16:34:00, 16:35:00)`, so `16:35:00` — exclusive — is the bound that contains it wherever it fell.

**The time grid** is a consequence the session boundary resolution being `R`. Reported start and end times lie on the `R` grid; `Adj_conn_end` adds exactly one `R`, so it lies on it too. `R` must divide 15 minutes without leaving a remainder. Otherwise, 15-minute segments can't properly partition the interval of interest.

### Assumptions

- **Session end times are truncated, not rounded.** `Adj_conn_end = Conn_DateTime_End + R` is the exclusive bound of the window the true end lies in only because the reported end is the true end rounded *down* to `R`. Under rounding to nearest, or under a convention where the reported end is the first instant the vehicle was no longer drawing power, the correct padding would differ — in the latter case it would be zero.
- **Breaker ratings are uniform across panels.** `count_based_kw` and `count_based_kva` are an aggregate session count multiplied by a single rating, so an installation mixing breakers of different ratings would skew both. Nothing else in the estimates depends on how many panels there are or on which panel a session ran: the session report carries no panel ID, and none is needed.
  - A session whose own average power exceeds that rating contradicts the assumption directly, and is flagged `ExcessiveAvgPower`. It is not excluded — the figure says something is wrong with `Energy_Use` or `Active_Charge_Time`, not which — but it is worth knowing about, because a segment whose aggregate average power exceeds its member count times the rating would put `energy_based_kw` *above* `count_based_kw` and invert the typical order of these two values. A segment can only invert if one of its sessions draws more than the rating, and every such member is flagged.

### Other

- Every session in the report is written to the workbook, anomalous ones included: the sheet is a faithful rendering of the session report, and which sessions take part in an estimate is decided on the reading side.
- Each session is checked for internal consistency, and the test is *derived* rather than chosen. Truncation puts the true start somewhere in `[Conn_start, Conn_start + R)` and the true end somewhere in `[Conn_end, Adj_conn_end)` — two half-open windows one `R` wide, the same convention used everywhere else. An honest `Conn_Duration` carries some instant of the first window to some instant of the second, so the record is sound exactly when the first window, shifted by the duration, still *meets* the second:

  ```
  sound  <=>  Conn_start + Conn_Duration  <  Adj_conn_end
         and  Conn_start + Conn_Duration  >  Conn_end - TIME_GRID_STEP
  ```

  Both bounds are strict, because both windows are half-open at the same end. That makes this the one band in the design that is open rather than half-open — an instance of the convention rather than an exception to it, since it is the *intersection* condition of two half-open windows and not an interval anyone chose. The band is not slack: it is precisely what truncation to whole minutes accounts for, and the sample data reaches to within 3 seconds of its lower edge.
- A session outside that band is flagged `InconsistentDuration` and excluded from the estimates. Both directions are faults: if a record's own fields disagree by more than the reporting can explain, neither its duration nor the span the grouping logic would place it on can be relied on. The overshoot direction subsumes the case of a session ending before it starts, since with `Conn_DateTime_End` a minute or more before `Conn_DateTime_Start` no non-negative duration can satisfy the test.
- These are the *only* sessions excluded from the estimates. Nothing else removes a session.
- Excluded sessions get a section of their own in the report, listing **every** one in the workbook rather than only those near the interval of interest, with a column saying whether each appears to fall in that interval. Appears only: a record whose own fields contradict each other cannot be trusted to say which window it belongs in, so filtering on that judgement could hide exactly the session a reader most needs to see.
- Sessions with zero `Energy_Use` and non-zero `Active_Charge_Time` do not contribute to `energy_based_kw` and `energy_based_kva` but they do contribute to `count_based_kw` and `count_based_kva`.
- A session with zero `Active_Charge_Time` delivered energy in no time at all, so its average power is unbounded or undefined.
  - The Excel `Avg_power` cell shows `#DIV/0!` so the fault is visible in the sheet. Function `session_list` returns the session as a *spike*, held apart from the normal sessions fed to the peak logic.
  - Spikes are worth reviewing individually for their effect on the building's demand charge.
  - The power estimating logic treats spikes as follows:
    - If `Energy_Use == 0`, set `Avg_power` to 0. These sessions do not contribute to `energy_based_kw` and `energy_based_kva` but they do contribute to `count_based_kw` and `count_based_kva`.
    - Otherwise, set `Avg_power` to the constant `BREAKER_RATING_KW`. These sessions contribute to all four estimate types.
