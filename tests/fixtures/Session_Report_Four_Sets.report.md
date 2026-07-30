EV Peak Power Contribution
==========================

Source     Session_Report_Four_Sets.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)


Estimates
---------

More than one reading of the data is defensible here, because some group was
reported with more concurrent sessions than a single panel can run, so
either a second panel is installed or the report is wrong, and some session
overlaps the interval by less than the precision its times are reported to,
so it may not have been running in this window at all. Each is given below.
The first counts every session and constrains nothing, so it is the one to
quote if only one figure is wanted.

"Direct" - every session counted, no panel constraint:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 13.500 | 14.211 |     0 |
| Breaker-spec-based | 80.400 | 90.000 |     2 |

"Clamped" - assuming one panel, capped at 10 concurrent sessions:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 13.500 | 14.211 |     0 |
| Breaker-spec-based | 67.000 | 75.000 |     2 |

"Direct, narrow" - counting only sessions whose overlap is certain:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 11.500 | 12.105 |     2 |
| Breaker-spec-based | 80.400 | 90.000 |     2 |

"Clamped, narrow" - both restrictions at once:

| Estimate           |     kW |    kVA | Group |
|:-------------------|-------:|-------:|------:|
| Consumption-based  | 10.000 | 10.526 |     2 |
| Breaker-spec-based | 67.000 | 75.000 |     2 |

The likely kW values are in the range from 10.000 kW ("Clamped, narrow",
consumption-based) to 80.400 kW ("Direct", breaker-spec-based). The likely
kVA values are in the range from 10.526 kVA ("Clamped, narrow",
consumption-based) to 90.000 kVA ("Direct", breaker-spec-based).


Session groups
--------------

|  # | From     | To       |   Len | Direct Count | Direct kW | Narrow Count | Narrow kW |
|---:|:---------|:---------|------:|-------------:|----------:|-------------:|----------:|
| 0  | 16:00:00 | 16:01:00 |  1:00 |            2 |    13.500 |            1 |     0.500 |
| 1  | 16:01:00 | 16:10:00 |  9:00 |            1 |     0.500 |            1 |     0.500 |
| 2* | 16:10:00 | 17:00:00 | 50:00 |           12 |    11.500 |           12 |    11.500 |

Times are local (ET). Groups are half-open: each runs from its From up to
but not including its To, so no instant falls in two groups and no session
is counted twice.

"Direct" counts every member, "Narrow" only those whose overlap with the
interval is certain; the two differ exactly where a member is daggered under
Group membership.

An asterisk marks a group holding more sessions than a single panel can run
at once. "Clamped" estimates were computed over a subset of them; the
figures are under Anomalies


Group membership
----------------

- Group 0 - MARGIN†, SPAN
- Group 1 - SPAN
- Group 2 - A01, A02, A03, A04, A05, A06, A07, A08, A09, A10, A11, SPAN

† marks a session whose overlap with the interval is not certain, its
reported times leaving it undecidable. Such a session is counted in the main
estimates and left out of the "Narrow" ones; the reason is under Anomalies.


Anomalies
---------

| Row | Session | Anomaly                      |
|----:|:--------|:-----------------------------|
|   2 | MARGIN  | IntersectsBoundaryMarginOnly |

Row numbers are workbook rows, so each one can be looked up directly.

- IntersectsBoundaryMarginOnly - session overlaps the interval of interest
  only within a minute of a boundary, which is the precision the report's
  session times are stated to, so it may not have been running in this
  window at all. It is counted in the main estimates; the "Narrow" ones
  leave it out.

| Group | From     | To       | Reported | Included | Anomaly             |
|------:|:---------|:---------|---------:|---------:|:--------------------|
|     2 | 16:10:00 | 17:00:00 |       12 |       10 | ClampedSessionGroup |

- ClampedSessionGroup - the report claims more sessions were charging at
  once than a single panel should allow. The "Clamped" estimates use only
  the 10 the panel could have run; the "Direct" estimates use all of them.

