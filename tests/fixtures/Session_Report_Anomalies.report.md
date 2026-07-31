EV Peak Power Contribution
==========================

Source     Session_Report_Anomalies.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)


Estimates
---------

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 18.700 | 19.684 |     3 |
| Breaker-spec-based | 20.100 | 22.500 |     3 |

The likely kW values are in the range from 18.700 kW (consumption-based) to
20.100 kW (breaker-spec-based). The likely kVA values are in the range from
19.684 kVA (consumption-based) to 22.500 kVA (breaker-spec-based).

2 sessions in the workbook were excluded from every figure above, having
reported times that contradict each other. They are listed under Excluded
sessions.


Excluded sessions
-----------------

| Row | Session  | From             | To               | Window | Anomaly              |
|----:|:---------|:-----------------|:-----------------|:-------|:---------------------|
|   4 | BAD      | 2026-06-15 16:05 | 2026-06-15 16:31 | yes    | InconsistentDuration |
|   8 | REVERSED | 2026-06-15 16:30 | 2026-06-15 16:21 | yes    | InconsistentDuration |

These sessions take no part in any estimate. Times are local (ET), and the
list covers the whole workbook rather than the interval of interest.
"Window" is whether the session appears to fall in that interval - appears
only, because a record whose own fields contradict each other cannot be
trusted to say which window it belongs in. It reads the same doubtful times,
so no row was dropped on its say-so.

- InconsistentDuration - Conn_start + Conn_Duration misses Conn_DateTime_End
  by a minute or more; start, end and duration are inconsistent.


Session groups
--------------

| # | From     | To       |   Len | Count |     kW |
|--:|:---------|:---------|------:|------:|-------:|
| 0 | 16:00:00 | 16:01:00 |  1:00 |     1 |  6.000 |
| 1 | 16:10:00 | 16:15:00 |  5:00 |     1 |  6.000 |
| 2 | 16:15:00 | 16:22:00 |  7:00 |     2 | 12.000 |
| 3 | 16:22:00 | 16:23:00 |  1:00 |     3 | 18.700 |
| 4 | 16:23:00 | 16:36:00 | 13:00 |     2 | 12.000 |
| 5 | 16:36:00 | 16:41:00 |  5:00 |     1 |  6.000 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.


Group membership
----------------

- Group 0 - MARGIN
- Group 1 - N1
- Group 2 - N1, N2
- Group 3 - N1, N2, SPIKE
- Group 4 - N1, N2
- Group 5 - N1


Anomalies
---------

| Row | Session | Anomaly              |
|----:|:--------|:---------------------|
|   6 | SPIKE   | ZeroActiveChargeTime |

Row numbers are workbook rows, so each one can be looked up directly.

- ZeroActiveChargeTime - zero Active_Charge_Time, so the session delivered
  its energy in no time at all and has no finite average power; the
  estimating logic substitutes one, and the session is worth reviewing
  individually.

