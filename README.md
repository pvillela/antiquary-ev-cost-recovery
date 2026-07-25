# Contribution of EVs to Building Peak Power Consumption

## Notes

### Time zone

- The session report's timestamps are stated in local time, i.e., ET. We need to convert them to UTC.
  The time zone is `America/Toronto`.
- The conversion to UTC is straightforward for almost every point in time, except for the repeated hour on the day that DST ends (move from EDT 02:00 to EST 01:00). 
  - Based on the `Conn_DateTime_Start`, `Conn_DateTime_End`, and `Conn_Duration`, the corresponding UTC values can be inferred, except for sessions with duration of less than 1 hour that fall between the ambiguous 01:00:00-01:59:59 interval.
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

- Sessions with zero `Energy_Use` are included in the transformation from CSV to Excel but are excluded from peak power contribution logic.
