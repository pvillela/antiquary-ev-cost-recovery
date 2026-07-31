EV Peak Power Contribution
==========================

Source     Session_Report_SkewMargin.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)
Covered    2026-06-15 15:59 - 17:01 EDT  (62 minutes)


Estimates
---------

| Estimate           |    kW |   kVA | Group |
|:-------------------|------:|------:|------:|
| Consumption-based  | 2.000 | 2.105 |     0 |
| Breaker-spec-based | 6.700 | 7.500 |     0 |

The likely kW values are in the range from 2.000 kW (consumption-based) to
6.700 kW (breaker-spec-based). The likely kVA values are in the range from
2.105 kVA (consumption-based) to 7.500 kVA (breaker-spec-based).


Skew margins
------------

The interval of interest comes from Toronto Hydro's metering data and the
session times from Evolute, and nothing reconciles the two clocks. The
windows just before and just after the interval are therefore estimated too,
and one of them is shown below because its figures come out higher than the
interval's own. Read it as what the EV load could have been had the two
clocks disagreed - not as a second estimate of the metered window.

"Before" - 2026-06-15 15:59 - 16:00 EDT (1 minute):

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 12.000 | 12.632 |     0 |
| Breaker-spec-based | 13.400 | 15.000 |     0 |

The likely kW values are in the range from 12.000 kW (consumption-based) to
13.400 kW (breaker-spec-based). The likely kVA values are in the range from
12.632 kVA (consumption-based) to 15.000 kVA (breaker-spec-based).

| # | From     | To       |  Len | Count |     kW |
|--:|:---------|:---------|-----:|------:|-------:|
| 0 | 15:59:00 | 16:00:00 | 1:00 |     2 | 12.000 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.

- Group 0 - BEFORE1, BEFORE2


Session groups
--------------

| # | From     | To       |   Len | Count |    kW |
|--:|:---------|:---------|------:|------:|------:|
| 0 | 16:10:00 | 16:41:00 | 31:00 |     1 | 2.000 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.


Group membership
----------------

- Group 0 - INSIDE


Anomalies
---------

None. Every session considered for this interval was well formed.

