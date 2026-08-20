EV Peak Power Contribution
==========================

Source     Session_Report_Diagram.xlsx
Interval   2026-06-15 16:00 - 17:00 EDT  (1 hour)


Estimates
---------

| Estimate     | Unit |    Min |    Max | Segment |
|:-------------|:-----|-------:|-------:|:--------|
| Energy-based | kW   | 18.383 | 19.657 | 16:15   |
| Energy-based | kVA  | 18.852 | 20.139 | 16:15   |
| Count-based  | kW   | 19.787 | 21.121 | 16:15   |
| Count-based  | kVA  | 20.271 | 21.619 | 16:15   |

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


Segments
--------

| Segment |       Count |            kW |
|:--------|------------:|--------------:|
| 16:00   | 2.400-2.467 | 14.880-15.293 |
| 16:15   | 2.933-3.133 | 17.940-19.200 |
| 16:30   | 2.800-3.133 | 17.420-19.560 |
| 16:45   | 1.400-1.533 |   8.440-9.253 |

Times are local (ET), and each segment is 15 minutes long, named by the
minute it starts on. Segments are half-open: each runs from its own start up
to but not including the next one's, so no instant falls in two of them and
they tile the interval exactly. "Count" is a session count weighted by how
much of the segment each session covered, so it is fractional; "kW" weights
each session's average power the same way.


Segment membership
------------------

- 16:00 - A, B, C
- 16:15 - A, B, C, D, E
- 16:30 - A, C, D, E, F
- 16:45 - A, G


Anomalies
---------

| Row | Session | Anomaly               |
|----:|:--------|:----------------------|
|   5 | D       | ExcessiveAvgKw(6.600) |
|   7 | F       | ExcessiveAvgKw(6.700) |

Row numbers are rows of the source data file named above, so each one can be
looked up directly. Only sessions reaching the interval of interest are
listed here; a session anomalous elsewhere in the workbook is not this
interval's concern.

- ExcessiveAvgKw - average kilowatts above the Evolute breaker rating, which
  the hardware should not allow; the session still counts towards every
  estimate, but the breaker-spec figures assume no session draws more than
  that rating.

