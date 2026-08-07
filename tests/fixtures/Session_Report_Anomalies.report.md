EV Peak Power Contribution
==========================

Source     Session_Report_Anomalies.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)


Estimates
---------

| Estimate     | Unit |    Min |    Max | Segment |
|:-------------|:-----|-------:|-------:|:--------|
| Energy-based | kW   | 16.162 | 17.474 | 16:15   |
| Energy-based | kVA  | 16.609 | 17.933 | 16:15   |
| Count-based  | kW   | 17.124 | 18.455 | 16:15   |
| Count-based  | kVA  | 17.580 | 18.925 | 16:15   |

Every figure is a bracket: the reported session times are stated only to the
minute, so each estimate runs from what those times least support to what
they most support. "Energy-based" is derived from the sessions' own
consumption, "Count-based" from how many of them were charging and the
per-EV rating of the infrastructure. "Segment" names the 15-minute segment
the figure was drawn from - the one where that derivation peaks, which the
two need not agree on.

The peak is always a 15-minute average, whatever the length of the interval
asked for, because that is the basis the demand charge is billed on. An hour
is reported as the highest of its four segments, not as an average over the
whole hour.

2 sessions in the workbook were excluded from every figure above, having
reported times that contradict each other. They are listed under Excluded
sessions.


Segments
--------

| Segment |       Count |            kW |
|:--------|------------:|--------------:|
| 16:00   | 0.267-0.400 |   1.600-2.400 |
| 16:15   | 2.533-2.733 | 15.740-17.039 |
| 16:30   | 1.000-1.200 |   6.000-7.260 |
| 16:45   | 0.000-0.000 |   0.000-0.000 |

Times are local (ET), and each segment is 15 minutes long, named by the
minute it starts on. Segments are half-open: each runs from its own start up
to but not including the next one's, so no instant falls in two of them and
they tile the interval exactly. "Count" is a session count weighted by how
much of the segment each session covered, so it is fractional; "kW" weights
each session's average power the same way.


Segment membership
------------------

- 16:00 - MARGIN, N1
- 16:15 - EXCESS, N1, N2, SPIKE
- 16:30 - EXCESS, N1, N2
- 16:45 - none


Excluded sessions
-----------------

| Row | Session  | From             | To    | In interval | Anomaly              |
|----:|:---------|:-----------------|:------|:------------|:---------------------|
|   4 | BAD      | 2026-06-15 16:05 | 16:31 | yes         | InconsistentDuration |
|   8 | REVERSED | 2026-06-15 16:30 | 16:21 | yes         | InconsistentDuration |

These sessions take no part in any estimate. Times are local (ET), and the
list covers the whole workbook rather than the interval estimated, so "From"
carries its date and "To" carries one only when the session crosses
midnight. "In interval" is whether the session appears to fall in the
interval - appears only, because a record whose own fields contradict each
other cannot be trusted to say where it belongs. It reads the same doubtful
times, so no row was dropped on its say-so.

- InconsistentDuration - Conn_start + Conn_Duration misses Conn_DateTime_End
  by a minute or more; start, end and duration are inconsistent.


Anomalies
---------

| Row | Session | Anomaly               |
|----:|:--------|:----------------------|
|   6 | SPIKE   | ZeroActiveChargeTime  |
|   9 | EXCESS  | ExcessiveAvgKw(6.900) |

Row numbers are workbook rows, so each one can be looked up directly. Only
sessions reaching the interval of interest are listed here. The Excluded
sessions table above is scoped differently - it covers the whole workbook,
and carries an "In interval" column for that reason.

- ZeroActiveChargeTime - zero Active_Charge_Time, so the session delivered
  its energy in no time at all and has no finite average power; the
  estimating logic substitutes one, and the session is worth reviewing
  individually.
- ExcessiveAvgKw - average power above the Evolute breaker rating, which the
  hardware should not allow; the session still counts towards every
  estimate, but the breaker-spec figures assume no session draws more than
  that rating.

