EV Peak Power Contribution
==========================

Source     Session_Report_Anomalies.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)


Estimates
---------

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 25.600 | 26.947 |     4 |
| Breaker-spec-based | 26.800 | 30.000 |     4 |

The likely kW values are in the range from 25.600 kW (consumption-based) to
26.800 kW (breaker-spec-based). The likely kVA values are in the range from
26.947 kVA (consumption-based) to 30.000 kVA (breaker-spec-based).

2 sessions in the workbook were excluded from every figure above, having
reported times that contradict each other. They are listed under Excluded
sessions.


Excluded sessions
-----------------

| Row | Session  | From             | To               | Window   | Anomaly              |
|----:|:---------|:-----------------|:-----------------|:---------|:---------------------|
|   4 | BAD      | 2026-06-15 16:05 | 2026-06-15 16:31 | interval | InconsistentDuration |
|   8 | REVERSED | 2026-06-15 16:30 | 2026-06-15 16:21 | interval | InconsistentDuration |

These sessions take no part in any estimate. Times are local (ET), and the
list covers the whole workbook rather than the windows estimated. "Window"
is which of those the session appears to fall in - the interval of interest,
the skew margin before it, the one after it, or none - appears only, because
a record whose own fields contradict each other cannot be trusted to say
which window it belongs in. It reads the same doubtful times, so no row was
dropped on its say-so.

- InconsistentDuration - Conn_start + Conn_Duration misses Conn_DateTime_End
  by a minute or more; start, end and duration are inconsistent.


Session groups
--------------

| # | From     | To       |  Len | Count |     kW |
|--:|:---------|:---------|-----:|------:|-------:|
| 0 | 16:00:00 | 16:01:00 | 1:00 |     1 |  6.000 |
| 1 | 16:10:00 | 16:15:00 | 5:00 |     1 |  6.000 |
| 2 | 16:15:00 | 16:20:00 | 5:00 |     2 | 12.000 |
| 3 | 16:20:00 | 16:22:00 | 2:00 |     3 | 18.900 |
| 4 | 16:22:00 | 16:23:00 | 1:00 |     4 | 25.600 |
| 5 | 16:23:00 | 16:31:00 | 8:00 |     3 | 18.900 |
| 6 | 16:31:00 | 16:36:00 | 5:00 |     2 | 12.000 |
| 7 | 16:36:00 | 16:41:00 | 5:00 |     1 |  6.000 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.


Group membership
----------------

- Group 0 - MARGIN
- Group 1 - N1
- Group 2 - N1, N2
- Group 3 - EXCESS, N1, N2
- Group 4 - EXCESS, N1, N2, SPIKE
- Group 5 - EXCESS, N1, N2
- Group 6 - N1, N2
- Group 7 - N1


Anomalies
---------

| Row | Session | Window   | Anomaly                  |
|----:|:--------|:---------|:-------------------------|
|   6 | SPIKE   | interval | ZeroActiveChargeTime     |
|   9 | EXCESS  | interval | ExcessiveAvgPower(6.900) |

Row numbers are workbook rows, so each one can be looked up directly.
"Window" is which of the estimated windows the session reaches - the
interval of interest, the skew margin before it, the one after it, or more
than one. A session reaching only a margin still matters: a figure reported
for that margin may rest on it.

- ZeroActiveChargeTime - zero Active_Charge_Time, so the session delivered
  its energy in no time at all and has no finite average power; the
  estimating logic substitutes one, and the session is worth reviewing
  individually.
- ExcessiveAvgPower - average power above the Evolute breaker rating, which
  the hardware should not allow; the session still counts towards every
  estimate, but the breaker-spec figures assume no session draws more than
  that rating.

