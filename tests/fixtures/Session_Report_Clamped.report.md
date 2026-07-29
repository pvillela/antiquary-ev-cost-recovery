EV Peak Power Contribution
==========================

Source     Session_Report_Clamped.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)


Estimates
---------

More than one reading of the data is defensible here, because some group was
reported with more concurrent sessions than a single panel can run, so
either a second panel is installed or the report is wrong. Each is given
below. The first counts every session and constrains nothing, so it is the
one to quote if only one figure is wanted.

"Direct" - every session counted, no panel constraint:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 66.600 | 70.105 |     0 |
| Breaker-spec-based | 80.400 | 90.000 |     0 |

"Clamped" - assuming one panel, capped at 10 concurrent sessions:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 56.500 | 59.474 |     0 |
| Breaker-spec-based | 67.000 | 75.000 |     0 |

The likely kW values are in the range from 56.500 kW ("Clamped",
consumption-based) to 80.400 kW ("Direct", breaker-spec-based). The likely
kVA values are in the range from 59.474 kVA ("Clamped", consumption-based)
to 90.000 kVA ("Direct", breaker-spec-based).


Session groups
--------------

|  # | From     | To       | Length | Count |     kW |
|---:|:---------|:---------|-------:|------:|-------:|
| 0* | 16:20:00 | 16:51:00 |  31:00 |    12 | 66.600 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.

An asterisk marks a group holding more sessions than a single panel can run
at once. "Clamped" estimates were computed over a subset of them; the
figures are under Anomalies


Group membership
----------------

- Group 0 - C00, C01, C02, C03, C04, C05, C06, C07, C08, C09, C10, C11


Anomalies
---------

No session was anomalous, but one group was:

| Group | From     | To       | Reported | Included | Anomaly             |
|------:|:---------|:---------|---------:|---------:|:--------------------|
|     0 | 16:20:00 | 16:51:00 |       12 |       10 | ClampedSessionGroup |

- ClampedSessionGroup - the report claims more sessions were charging at
  once than a single panel should allow. The "Clamped" estimates use only
  the 10 the panel could have run; the "Direct" estimates use all of them.

