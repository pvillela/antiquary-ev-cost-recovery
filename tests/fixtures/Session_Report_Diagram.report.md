EV Peak Power Contribution
==========================

Source     Session_Report_Diagram.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)


Estimates
---------

More than one reading of the data is defensible here, because some group
holds two sessions that need not have overlapped each other - one is
reported as ending in the same minute the other is reported as starting, and
the report states those times only to the minute. Both readings are given
below. The first counts every session and assumes nothing, so it is the one
to quote if only one figure is wanted.

"Nominal" - every group's membership taken at face value:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 31.400 | 33.053 |     5 |
| Breaker-spec-based | 33.500 | 37.500 |     5 |

"Minimum overlap" - assuming the sessions in each dubious group overlapped
  as little as their reported times allow:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 24.700 | 26.000 |     4 |
| Breaker-spec-based | 26.800 | 30.000 |     4 |

The likely kW values are in the range from 24.700 kW ("Minimum overlap",
consumption-based) to 33.500 kW ("Nominal", breaker-spec-based). The likely
kVA values are in the range from 26.000 kVA ("Minimum overlap",
consumption-based) to 37.500 kVA ("Nominal", breaker-spec-based).


Session groups
--------------

|  # | From     | To       |   Len | Count |     kW | Min Count | Min kW |
|---:|:---------|:---------|------:|------:|-------:|----------:|-------:|
| 0  | 16:00:00 | 16:08:00 |  8:00 |     2 | 12.400 |         2 | 12.400 |
| 1  | 16:08:00 | 16:16:00 |  8:00 |     3 | 18.600 |         3 | 18.600 |
| 2  | 16:16:00 | 16:20:00 |  4:00 |     2 | 12.200 |         2 | 12.200 |
| 3  | 16:20:00 | 16:24:00 |  4:00 |     3 | 18.100 |         3 | 18.100 |
| 4  | 16:24:00 | 16:34:00 | 10:00 |     4 | 24.700 |         4 | 24.700 |
| 5* | 16:34:00 | 16:35:00 |  1:00 |     5 | 31.400 |         4 | 24.700 |
| 6  | 16:35:00 | 16:43:00 |  8:00 |     3 | 18.900 |         3 | 18.900 |
| 7  | 16:43:00 | 16:48:00 |  5:00 |     1 |  6.000 |         1 |  6.000 |
| 8  | 16:48:00 | 16:56:00 |  8:00 |     2 | 12.100 |         2 | 12.100 |
| 9  | 16:56:00 | 17:00:00 |  4:00 |     1 |  6.000 |         1 |  6.000 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.

An asterisk marks a dubious group: one holding two sessions that need not
have overlapped each other, because one is reported as ending in the same
minute the other is reported as starting. "Count" and "kW" take its
membership at face value; "Min Count" and "Min kW" assume as little overlap
as the reported times allow. They differ on exactly the marked rows.


Group membership
----------------

- Group 0 - A, B
- Group 1 - A, B, C
- Group 2 - A, C
- Group 3 - A, C, E
- Group 4 - A, C, D, E
- Group 5 - A, C, D, E, F
- Group 6 - A, C, F
- Group 7 - A
- Group 8 - A, G
- Group 9 - A


Anomalies
---------

None. Every session considered for this interval was well formed.

