# Segment tiling, worked through

How charging sessions land on the 15-minute segments that an interval of interest is divided
into, walked through on the seven-session example in `tests/fixtures/Session_Report_Diagram.csv`.

The same example is asserted, session by session, in `tests/segment_tiling.rs`, and rendered in
full in `tests/fixtures/Session_Report_Diagram.report.md`. This document is the prose; those two
are the machine-checked versions of the same claims.

There is no diagram beyond the sketch below, and deliberately so. A uniform 15-minute partition is
simple enough to read as a table — what needed a drawing was the old variable-length grouping,
where the group boundaries were themselves derived from the data.

## The interval and its segments

The interval of interest is 16:00–17:00 local on 2026-06-15, a date with no DST transition. It is
one hour, so it divides into four segments:

| Segment | From  | To    |
|---------|-------|-------|
| `16:00` | 16:00 | 16:15 |
| `16:15` | 16:15 | 16:30 |
| `16:30` | 16:30 | 16:45 |
| `16:45` | 16:45 | 17:00 |

Segments are **half-open**: each runs from its own start up to but *not including* the next one's.
No instant falls in two of them, and together they cover the interval exactly. A 15-minute interval
of interest yields a single segment by the same rule.

Segments are named by the minute they start on, and that name is the join key across the report:
the Estimates section names the winning segment by it, the Segments table lists it, and the Segment
membership section keys its lists on it.

## The seven sessions

Every session occupies the half-open span `[Conn_start, Adj_conn_end)`, where `Adj_conn_end` is the
*reported* end plus one `TIME_GRID_STEP` — currently one minute. That padding is not slack: a time
stated to the minute is the true time truncated down, so a session reported to end at 16:34 truly
ended somewhere in `[16:34:00, 16:35:00)`, and 16:35:00 exclusive is the tightest bound that
contains it wherever in that minute it fell.

```text
           16:00      16:15      16:30      16:45      17:00
             |          |          |          |          |
  A   15:54 =|==========|==========|==========|==========|===== 17:04
  B     15:59|=====|16:16                                        overruns the left edge
  C          |  16:08 =====|16:43                                nested, spans two segments
  E          |         16:20 ==|16:35                            staggered start with D
  D          |          16:24 =|16:35                            ends the same minute as E
  F          |            16:34 =====|16:43                      starts the minute D and E end
  G          |                       16:48 ==|16:56              alone in the last segment
```

| Session | Reported start | Reported end | Span used            | Notes                          |
|---------|----------------|--------------|----------------------|--------------------------------|
| `A`     | 15:54          | 17:03        | 15:54 – 17:04        | Outruns the interval both ends |
| `B`     | 15:59          | 16:15        | 15:59 – 16:16        | Starts before the interval     |
| `C`     | 16:08          | 16:42        | 16:08 – 16:43        | Wholly inside                  |
| `E`     | 16:20          | 16:34        | 16:20 – 16:35        | Staggered against `D`          |
| `D`     | 16:24          | 16:34        | 16:24 – 16:35        | Ends the same minute as `E`    |
| `F`     | 16:34          | 16:42        | 16:34 – 16:43        | Starts the minute `D`/`E` end  |
| `G`     | 16:48          | 16:55        | 16:48 – 16:56        | Alone but for `A`              |

## Which sessions each segment holds

A session belongs to a segment when the two **overlap** — share at least one instant. Abutting is
not overlapping, and the half-open convention is what keeps the two distinguishable.

| Segment | Sessions              | Why                                                        |
|---------|-----------------------|------------------------------------------------------------|
| `16:00` | `A`, `B`, `C`         | `D`, `E`, `F`, `G` all begin after 16:15                    |
| `16:15` | `A`, `B`, `C`, `D`, `E` | `B` reaches in by one minute; `D` and `E` start inside    |
| `16:30` | `A`, `C`, `D`, `E`, `F` | `B` has ended; `F` starts inside                          |
| `16:45` | `A`, `G`              | `C`, `D`, `E`, `F` have all ended by 16:43                  |

Two entries are worth dwelling on.

**`B` in the 16:15 segment.** `B` is *reported* to end at 16:15, which looks like it stops exactly
where the segment starts. But its true end lies anywhere in `[16:15, 16:16)`, so it may still have
been drawing power when the segment opened. It is therefore counted — as a fraction, and a small
one, which is what the bracket below expresses.

**`D`, `E` and `F` in the 16:30 segment.** `D` and `E` are reported to end at 16:34 and `F` is
reported to start at 16:34. Whether `F` overlapped them or merely took over from them, the reported
times cannot say. All three are in the segment, and the doubt is carried into the figures rather
than resolved by fiat.

## What each segment contributes

Two aggregates are computed per segment, and every estimate is derived from them:

- **`agg_count`** — each session's *overlap ratio*, the fraction of the segment its span covers,
  summed over the segment's sessions. A session covering the whole segment contributes 1; one
  covering half contributes 0.5. It is a session count weighted by presence, so it is fractional.
- **`agg_kw`** — the same ratios, each multiplied by its session's own average power
  (`Energy_Use` over `Active_Charge_Time`), summed the same way.

Both are **brackets** rather than numbers. Where a session's own reported edge falls inside the
segment, the minute of truncation is a minute of genuine doubt, and the ratio runs from what the
reported times least support to what they most support. Where neither edge falls inside — as with
`A`, which crosses the whole interval — the ratio is exactly 1 and the bracket is a point.

| Segment | `agg_count`   | `agg_kw`        |
|---------|---------------|-----------------|
| `16:00` | 2.400 – 2.467 | 14.880 – 15.293 |
| `16:15` | 2.933 – 3.133 | 17.940 – 19.200 |
| `16:30` | 2.800 – 3.133 | 17.420 – 19.560 |
| `16:45` | 1.400 – 1.533 | 8.440 – 9.253   |

The 16:00 segment is nearly exact — only `C` has an edge inside it — while 16:30 has the widest
bracket of the four, because `C`, `D`, `E` and `F` all begin or end within it.

## Which segment is reported

The estimates are reported for the **maximal** segment: the one where the derivation peaks. The
two derivations are ranked separately and need not agree, so each names its own segment.

Here they do agree, and narrowly. Ranked on the midpoint of its bracket, 16:15 leads 16:30 by
0.067 on `agg_count` (3.033 against 2.966) and by 0.08 kW on `agg_kw` (18.570 against 18.490).
Both maxima are 16:15, and that is the segment the report names.

The narrowness is the point rather than an accident of the fixture. The two segments' brackets
overlap heavily — 16:30's runs wider in both directions — so which one is "the peak" rests on a
choice of ranking statistic. The midpoint is what the code uses, and a reader quoting one figure
should look at the Segments table rather than treat the winner as decisive.

Ties go to the earliest segment. That is not a rare case to have decided: in an interval no session
reached at all, every segment sits at the same standing block, and without a rule the report would
name an arbitrary one.

## The empty case

An interval that no session intersects still has its four segments, and they are not zero. The
transformer is energised whether or not a vehicle is plugged in, and its core loss and magnetizing
current are part of the building's demand. Every segment reports `site_load(0)` — the standing
block — and the report says in prose why the figures are not zero.
