# Contribution of EVs to Building's Peak Power Consumption

This software supports the estimation of the impact of EV charging on the building's peak power demand. Peak kW and kVA are used by Toronto Hydro to calculate distribution and transmission charges.

## Conceptual Approach

### Data sources and intervals of interest

For a given billing period, we can identify the time intervals in which the peak kW and kVA occurred based on metering data downloads from Toronto Hydro.

Given a time interval of interest, this software estimates the peak kW and kVA demand associated with EV charging activity during the interval.

The data source for EV power demand is the Evolute monthly session report.

### Estimation logic

Given a time interval of interest **`I`** as described above, the estimation of EV peak power demand during the interval proceeds as follows:

- From the Evolute monthly session report, identify all charging sessions that intersect the interval of interest `I`.
- At any time **`t`** within the interval of interest, the set of EV charging sessions that contain `t` can be determined. Such a set may be empty, contain a single session, or contain multiple sessions.
- The sets of EV charging sessions that are concurrently active may change a finite number of times during the interval of interest. These are called **`SessionGroup`**s.
- The algorithm implemented by this software identifies all non-empty `SessionGroup`s for the given interval of interest `I`.
- For each `SessionGroup`, the algorithm computes the following values:
  - **`avg_kw`**:  sum over all sessions in the `SessionGroup` of each session's average power demand. For each session, the average power demand is the session's total energy consumption divided by the session's charging time.
  - **`count`**: number of sessions in the `SessionGroup`.
- For the interval of interest `I`, the algorithm computes the following values:
  - **`consumption_based_kw`**: highest value of `avg_kw` over all `SessionGroup`s.
  - **`consumption_based_kva`**: `consumption_based_kw` divided by a power factor constant **`EV_POWER_FACTOR`** that reflects the combination of typical EV chargers and the Evolute infrastructure.
  - **`breaker_spec_based_kw`**: highest value of `count` over all `SessionGroup`s multiplied by the Evolute smart breaker kW rating of 6.7 kW.
  - **`breaker_spec_based_kva`**: highest value of `count` over all `SessionGroup`s multiplied by the Evolute smart breaker kVA rating of 7.5 kVA.
- These four values provide brackets for the EV peak power demand during the interval of interest `I`:
  - The actual peak kW associated with EV charging activity during `I` is likely between `consumption_based_kw` and `breaker_spec_based_kw`.
  - The actual peak kVA associated with EV charging activity during `I` is likely between `consumption_based_kva` and `breaker_spec_based_kva`.

## Workflow

This is the typical workflow used with this software to estimate the impact of EV charging activity on a particular Toronto Hydro bill:

- Preliminary steps (out of scope for this software):
  - Download Toronto Hydro metering data for the time period of interest.
  - Based on the downloaded data, identify the interval(s) of interest during which the billing period's peak kW and/or peak kVA occurred.
  - Obtain the *session report* file from Evolute covering the interval(s) of interest.
- Using this software:
  - Transform the relevant Evolute *session report* CSV file to an Excel file. The transformation process includes some data validation and computes additional columns that are included in the Excel file.
  - Access the relevant Excel file and compute the peak kW and kVA brackets for the interval(s) of interest.

## Technical Notes

### Time zone

- The session report's timestamps are stated in local time, i.e., ET. We need to convert them to UTC.
  The time zone is `America/Toronto`.
- The conversion to UTC is straightforward for almost every point in time, except for the repeated hour on the day that DST ends (move from EDT 02:00 to EST 01:00). 
  - Based on the `Conn_DateTime_Start`, `Conn_DateTime_End`, and `Conn_Duration` fields in the Evolute session report, the corresponding UTC values can be inferred, except for sessions with duration of less than 1 hour that fall between the ambiguous 01:00:00-01:59:59 interval.
  - For the above-mentioned short sessions in the ambiguous interval, we need to make an assumption. For now, our policy will be to duplicate those session records, with one copy in the 01:00:00-01:59:59 EDT interval and the other copy in the 01:00:00-01:59:59 EST interval. This should be recorded in the CSV to Excel transformation function's result.

#### The inference, in detail

**The assumption it rests on.** `Conn_Duration` is *physical elapsed time*, so
`Conn_start_UTC + Conn_Duration == Conn_end_UTC` always. This is what makes the inference possible.
Were `Conn_Duration` instead a naive subtraction of local clock values, a session spanning the fold
would under-report by exactly the repeated hour, and the reported end could not distinguish the two
candidate offsets from each other.

**The procedure**, applied to `Conn_DateTime_Start`:

1. If the local time maps to exactly one instant, use it. This is every timestamp except during the
   two transitions each year.
2. If it falls in the **fold** — the repeated 01:00:00-01:59:59 hour — there are two candidate
   instants, one at the EDT offset (UTC-4) and one at the EST offset (UTC-5). Take each candidate
   in turn, add `Conn_Duration`, convert back to local time, and check whether the result matches
   the reported `Conn_DateTime_End`. **The comparison is truncated to the minute**, because the
   report's timestamps carry no seconds; comparing exactly would reject the correct candidate.
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
  - A new field, adjusted session end time `Adj_conn_end` is computed as: `min(Conn_DateTime_Start + 59 seconds + Conn_Duration, Conn_DateTime_End + 59 seconds)`. This formula can be simplified as `min(Conn_DateTime_Start + Conn_Duration, Conn_DateTime_End) + 59 seconds`, but its intent is more obvious in the longer form.
  - A new field, adjusted session duration `Adj_conn_duration` is computed as: `Adj_conn_end - Conn_DateTime_Start`.
  - Three new fields are added: `Conn_start_UTC`, `Conn_end_UTC`, and `Adj_conn_end_UTC`, with UTC values corresponding to the corresponding local time fields.

### Other

- Sessions with zero `Energy_Use` are included in the transformation from CSV to Excel but are excluded from peak power contribution logic. `session_list` is where that exclusion happens: the workbook stays a faithful rendering of the session report, and filtering is left to the reading side.
- A session with non-zero `Energy_Use` and zero `Active_Charge_Time` delivered energy in no time at all, so its average power is unbounded. The Excel `Avg_power` cell shows `#DIV/0!` — the formula is written on every row rather than being skipped, so the fault is visible in the sheet — and `session_list` returns the session as a *spike*, held apart from the sessions fed to the peak logic. Spikes are worth reviewing individually for their effect on the building's demand charge.
