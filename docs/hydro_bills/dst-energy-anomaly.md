# The DST energy anomaly, resolved

[`green-button-vs-bills.md`](green-button-vs-bills.md) flagged three billing periods whose energy
total missed the invoice by roughly one hour of consumption, all three at a clock change. This is
what causes them.

**It is not a daylight-saving bug, and it is not confined to those three periods.** The billing
period boundary in `green_button` is one hour later than Toronto Hydro's for half the year. The
clock-change periods are simply where that offset stops cancelling out and becomes visible.

## The finding

Toronto Hydro's billing period runs on **Eastern Standard Time year-round**. It does not shift with
daylight saving. `green_button` cuts its periods at **prevailing local midnight**, which is EST in
winter and EDT in summer.

The two agree from November to March and differ by one hour from March to November.

Re-summing the export's own hourly readings over an EST-fixed window reproduces **all 19 bills to
the milli-kWh**:

| Boundary rule | Periods matching exactly | Mean abs. error | Worst error | Total over 19 periods |
|---|---|---|---|---|
| Prevailing local midnight (current) | 6 of 19 | 20.343 kWh | 93.720 kWh | −104.277 kWh |
| **Fixed UTC−5, EST year-round** | **19 of 19** | **0.000 kWh** | **0.001 kWh** | **+0.003 kWh** |
| Fixed UTC−4, EDT year-round | 0 of 19 | 10.389 kWh | 38.520 kWh | −20.757 kWh |

The two residuals of 0.001 kWh are the bill's own three-decimal display, not a disagreement.

This was computed from `Interval_values` in the committed workbook, which re-sums to the
`Peak_values` totals exactly under the current rule — so the readings are sound and only the window
over them is in question.

## Which hour moves

For every period the two rules disagree on, the disputed reading is the **00:00–01:00 EDT hour of
the 24th** — the closing day's first hour.

Under EST-fixed, the period ends at 00:00 EST on the 24th, which is 01:00 EDT, so that hour is
inside the period. Under prevailing local midnight it ends at 00:00 EDT, so the hour falls to the
next period instead.

That single hour explains every difference in the report:

| Period ending | Hour dropped (kWh) | Hour added (kWh) | Net effect on total | Observed GB − Bill | Accounted for |
|---|---|---|---|---|---|
| 2024-12-23 | — | — | +0.000 | +0.000 | yes |
| 2025-01-23 | — | — | +0.000 | +0.000 | yes |
| 2025-02-23 | — | — | +0.000 | −0.000 | yes |
| 2025-03-23 | — | Mar 24 · 85.56 | +85.560 | −85.560 | yes |
| 2025-04-23 | Mar 24 · 85.56 | Apr 24 · 95.76 | +10.200 | −10.200 | yes |
| 2025-05-23 | Apr 24 · 95.76 | May 24 · 94.08 | −1.680 | +1.680 | yes |
| 2025-06-23 | May 24 · 94.08 | Jun 24 · 123.72 | +29.640 | −29.640 | yes |
| 2025-07-23 | Jun 24 · 123.72 | Jul 24 · 85.20 | −38.520 | +38.520 | yes |
| 2025-08-23 | Jul 24 · 85.20 | Aug 24 · 98.76 | +13.560 | −13.560 | yes |
| 2025-09-23 | Aug 24 · 98.76 | Sep 24 · 85.68 | −13.080 | +13.080 | yes |
| 2025-10-23 | Sep 24 · 85.68 | Oct 24 · 83.88 | −1.800 | +1.801 | yes |
| 2025-11-23 | Oct 24 · 83.88 | — | −83.880 | +83.880 | yes |
| 2025-12-23 | — | — | +0.000 | +0.001 | yes |
| 2026-01-23 | — | — | +0.000 | +0.000 | yes |
| 2026-02-23 | — | — | +0.000 | +0.000 | yes |
| 2026-03-23 | — | Mar 24 · 93.72 | +93.720 | −93.720 | yes |
| 2026-04-23 | Mar 24 · 93.72 | Apr 24 · 95.28 | +1.560 | −1.560 | yes |
| 2026-05-23 | Apr 24 · 95.28 | May 24 · 93.12 | −2.160 | +2.160 | yes |
| 2026-06-23 | May 24 · 93.12 | Jun 24 · 104.28 | +11.160 | −11.160 | yes |

## Why only the clock-change periods looked wrong

A summer period is shifted at **both** ends. It loses one 00:00 hour at the start and gains another
at the end, so the hour count is unchanged and only the *difference* between two midnight hours
shows up — a few kWh either way, small enough to look like meter-read noise.

A clock-change period is shifted at **one** end only, because the other end is already in EST and
the two rules coincide there:

- **March.** The period opens on Feb 24, in EST, where both rules agree — nothing is dropped. It
  closes on Mar 24, in EDT, where the EST rule reaches one hour further — an hour is added. Net: a
  whole hour missing from the export. Hence 671 hours against the bill's 672.
- **November.** The period opens on Oct 24, in EDT — an hour is dropped. It closes on Nov 24, in
  EST, where the rules agree — nothing added. Net: a whole hour too many. Hence 745 against 744.

So the three outliers are not a distinct fault. They are the same one-hour offset, failing to
cancel.

## The bill's own day count agrees

Every bill states a `Number of Days`. Multiplied by 24, it is the hour count of an EST-fixed window
in **19 of 19** periods. Under prevailing local midnight it holds in only 16 — the three
clock-change periods are exactly the failures, at 671 and 745 hours against a stated 28 and 31 days.

The invoice has been saying so all along.

## What this does *not* affect

**The demand figures are correct as they stand.** Recomputing `Demand kW` and `Demand kVA` over the
EST-fixed window gives the same peaks as the current window in all 19 periods — 0 mismatches out of
38, and all still agree with the bill.

**`Peak kW 7-7` cannot be affected**, and not merely as a matter of observation. The only hour the
shift ever moves is local hour 00:00, and the demand window covers 07:00 to 19:00. The two are
disjoint, so no hour that moves can be a candidate for the 7-7 peak.

This is why the day-23 reconciliation shows a perfect demand record beside an imperfect energy
record. The boundary error lands entirely in an off-peak midnight hour, which changes a monthly
energy sum and can never change a peak.

## Two corrections this forces

**1. The explanation in `docs/green_button/README.md` is superseded.** It records, for 2026-06-23,
that the −11.16 kWh difference "is not a period-boundary error" and is what "a meter read taken a
few minutes either side of local midnight would do". It is a period-boundary error, and it is not
minutes but exactly one hour: the 11.160 kWh is precisely 104.280 − 93.120, the two midnight hours
the shift swaps.

The supporting observation in that README stands and in fact corroborates this. It notes the whole
discrepancy falls in off-peak — which is what a misplaced 00:00 hour would do, off-peak being
19:00–07:00.

**2. `green-button-vs-bills.md` needs its "ordinary differences" section rewritten.** It repeats the
meter-read explanation and calls the clock-change periods an open question.

## Whether to change the code

Not a decision to take from this document alone, but the options are:

- **Change the boundary to EST-fixed.** Energy then reconciles exactly, and the demand figures are
  provably unchanged. Against it: `BillingPeriod` is currently expressed in local civil time, which
  reads naturally and is what the invoice's wording suggests; a fixed offset would need explaining.
- **Leave it and document the offset.** The demand charge is what this project exists to
  apportion, and it is already right. The energy error is 0.008% overall.

Worth confirming first that EST-fixed is Toronto Hydro's actual practice rather than a rule fitted
to 19 points. Interval meters commonly run on standard time year-round, so it is plausible on its
face, but 19 periods from one meter is the whole of the evidence here.

## Reproducing

Scripts used are in this session's scratchpad, not the repository. The method is short enough to
restate: read `Interval_values` from the workbook, key each reading by its UTC timestamp, and sum
over `[start, end)` where the bounds are the 24th of each month at 00:00 **UTC−5**, rather than at
local midnight. Compare against `hydro_bill_dump` output for the matching bill.
