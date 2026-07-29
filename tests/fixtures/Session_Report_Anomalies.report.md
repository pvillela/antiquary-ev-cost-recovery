EV Peak Power Contribution
==========================

Source     Session_Report_Anomalies.xlsx
Interval   2026-06-15 16:00 - 17:00 ET  (1 hour)


Estimates
---------

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 18.700 | 19.684 |     2 |
| Breaker-spec-based | 20.100 | 22.500 |     2 |

The likely kW values are in the range from 18.700 kW (consumption-based) to
20.100 kW (breaker-spec-based). The likely kVA values are in the range from
19.684 kVA (consumption-based) to 22.500 kVA (breaker-spec-based).


Session groups
--------------

| # | From     | To       | Length | Count |     kW |
|--:|:---------|:---------|-------:|------:|-------:|
| 0 | 16:10:00 | 16:15:00 |   5:00 |     1 |  6.000 |
| 1 | 16:15:00 | 16:22:00 |   7:00 |     2 | 12.000 |
| 2 | 16:22:00 | 16:23:00 |   1:00 |     3 | 18.700 |
| 3 | 16:23:00 | 16:36:00 |  13:00 |     2 | 12.000 |
| 4 | 16:36:00 | 16:41:00 |   5:00 |     1 |  6.000 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.


Group membership
----------------

- Group 0 - N1
- Group 1 - N1, N2
- Group 2 - N1, N2, SPIKE
- Group 3 - N1, N2
- Group 4 - N1


Anomalies
---------

| Row | Session  | Anomaly                      | Excluded |
|----:|:---------|:-----------------------------|:---------|
|   4 | BAD      | InconsistentDuration         | yes      |
|   5 | MARGIN   | IntersectsBoundaryMarginOnly | yes      |
|   6 | SPIKE    | ZeroActiveChargeTime         | no       |
|   8 | REVERSED | InconsistentDuration         | yes      |

Row numbers are workbook rows, so each one can be looked up directly.

- InconsistentDuration - Conn_start + Conn_Duration misses Conn_DateTime_End
  by a minute or more; start, end and duration are inconsistent.
- IntersectsBoundaryMarginOnly - session overlaps the interval of interest
  only within a minute of a boundary, which is the precision the report's
  session times are stated to.
- ZeroActiveChargeTime - zero Active_Charge_Time, so the session delivered
  its energy in no time at all and has no finite average power; the
  estimating logic substitutes one, and the session is worth reviewing
  individually.

