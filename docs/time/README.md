# Time

The `time` module: everything about dates, times and zones that more than one part of this software
needs. Module-specific date arithmetic stays in its own module.

`src/time/` holds the code — `base.rs` for the zone, the grid and intervals, `excel.rs` for
serial-date conversion, `tou.rs` and `holidays.rs` for Ontario's time-of-use rules.

## What lives here and what does not

| Concern | Where |
|---|---|
| The time zone, and resolving a wall time that is ambiguous or does not exist | here |
| The standard-time clock billing periods are cut on | here |
| Excel serial dates, in both directions | here |
| Ontario time-of-use periods and the holiday calendar | here |
| Truncating an instant to a grid of a given step | here |
| `TIME_GRID_STEP`, the step session boundaries are reported to | `sessions` |
| `METER_INTERVAL`, the interval a Toronto Hydro meter records | `green_button` |

The two steps live with the module that has a reason for their value. Truncation itself is here
because both use it, but neither step is a property of time.

## Boundaries and the time grid

`TIME_GRID_STEP` and the way segments tile an interval of interest are documented in
[`docs/sessions/README.md`](../sessions/README.md), under "Boundaries and the time grid". They
belong there: the grid is the session report's reporting resolution, and only `sessions` has one.

## Two clocks, and which is which

Almost everything here means **prevailing local time** — the clock a customer reads, which moves
twice a year. `local_date`, `local_hour` and `local_midnight` are that clock, and Time-of-Use
periods, the 07:00–19:00 demand window and the holiday calendar all run on it.

One thing does not. A Toronto Hydro **billing period** is cut on **standard time**, at 00:00 EST all
year round, and does not move when the clocks do. `standard_date` and `standard_midnight` are that
clock, `BILLING_OFFSET` is the offset, and `green_button::BillingPeriod` is the only caller.

The two coincide from November to March and differ by an hour from March to November, which is what
makes the distinction easy to lose and expensive to get wrong: a summer period cut on the wrong
clock is an hour out at each end, and a period containing a clock change is an hour out overall.
Cutting on prevailing local time reproduced 6 of 19 invoices; cutting on standard time reproduces
all 19 to the milli-kWh. The derivation is in
[`../hydro_bills/archive/dst-energy-anomaly-pre-fix.md`](../hydro_bills/archive/dst-energy-anomaly-pre-fix.md).

Two consequences worth knowing:

- A standard-time day is always 24 hours, so a billing period is always a whole number of days and
  matches the `Number of Days` its invoice states. Periods of 671 and 745 hours were what the
  prevailing-local boundary produced, and they are gone.
- `standard_midnight` cannot fail, where `local_midnight` can in principle: a fixed offset has no
  gap for a wall time to fall into and no fold for it to be ambiguous in.

The Green Button feed itself keeps the same standard-time day — its `IntervalBlock`s start at 05:00
UTC year-round. See `../green_button/Toronto_Hydro_Object_Model.md`, "Fixed daily grid".

## Time zone

- The session report's timestamps are stated in local time, i.e., ET. We need to convert them to UTC.
  The time zone is `America/Toronto`.
- The conversion to UTC is straightforward for almost every point in time, except for the repeated hour on the day that DST ends (move from EDT 02:00 to EST 01:00). 
  - Based on the `Conn_DateTime_Start`, `Conn_DateTime_End`, and `Conn_Duration` fields in the Evolute session report, the corresponding UTC values can be inferred, except for sessions with duration of less than 1 hour that fall between the ambiguous 01:00:00-01:59:59 interval.
  - For the above-mentioned short sessions in the ambiguous interval, we need to make an assumption. For now, our policy will be to duplicate those session records, with one copy in the 01:00:00-01:59:59 EDT interval and the other copy in the 01:00:00-01:59:59 EST interval. This should be recorded in the CSV to Excel transformation function's result.

### The inference, in detail

**The assumption it rests on.** `Conn_Duration` is *physical elapsed time*, so it spans the true
start and the true end of the connection. This is what makes the inference possible. Were
`Conn_Duration` instead a naive subtraction of local clock values, a session spanning the fold would
under-report by exactly the repeated hour, and the reported end could not distinguish the two
candidate offsets from each other.

Note the assumption holds of the *true* instants, not of the reported ones. Because the report
truncates start and end to whole minutes, `conn_start_utc + Conn_Duration` does not land on
`conn_end_utc` — it misses by strictly less than one `TIME_GRID_STEP`, in either
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
candidate is nearer to `conn_start_utc + Conn_Duration`, which is by then already known.

**Duplicated records** are given distinct ids — `<id>-EDT` and `<id>-EST` — because the peak power
contribution logic keys `Session` on its id alone and holds sessions in a `BTreeSet`. With identical
ids the second copy would be silently discarded on insertion, defeating the purpose of duplicating
it. Note also that **both copies carry the full `Energy_Use`**, so a duplicated session contributes
to the peak in both candidate hours.

## Truncating to a grid

`truncate_to(ts, step)` rounds an instant down to the nearest multiple of `step`, counting from the
Unix epoch, and `is_on_grid(ts, step)` says whether it was already there. The property everything
else rests on is

```text
truncate_to(ts, step) <= ts < truncate_to(ts, step) + step
```

which is what makes `adj_conn_start <= real_start` true in
[`docs/sessions/time-reporting-uncertainty.md`](../sessions/time-reporting-uncertainty.md).

Truncation is always **backwards**, including before 1970. The implementation uses `rem_euclid`
rather than `%` for that reason: `%` gives a negative remainder for a negative timestamp, which
would round towards zero — forwards — and break the bound above.

## Two resolvers, deliberately

Two functions resolve an ambiguous local time, and they must not be merged. They are asked
different questions:

- **`map_local`** (`sessions::ioi`) is asked *what could this wall time mean?* by a user choosing an
  interval of interest. It has nothing but the wall time, so it reports every candidate and lets the
  caller choose, or name `EST`/`EDT`.
- **`CsvSession::resolve`** (`sessions::csv`) is asked *which offset was this session actually
  at?* and has evidence the other lacks: `Conn_Duration`, untruncated elapsed time. That usually
  settles it; duplication is the fallback when it does not.

Their tie-breaks differ for the same reason. Giving the first the second's behaviour would have it
invent evidence it does not have; giving the second the first's would throw evidence away.
