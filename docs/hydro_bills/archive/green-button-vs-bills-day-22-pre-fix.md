# Green Button against the bills, with the period closing on the 22nd

> **These figures predate a fix.** Both workbooks compared here were generated while the period
> boundary was at prevailing local midnight; it is now at 00:00 EST, for the reasons in
> [`dst-energy-anomaly-pre-fix.md`](dst-energy-anomaly-pre-fix.md). That change does not touch this document's
> conclusion — the closing day and the clock the boundary runs on are separate questions, and the
> 23rd is the right closing day under either.

The same comparison as [`green-button-vs-bills-pre-fix.md`](green-button-vs-bills-pre-fix.md), with one thing
changed: the meter export was regenerated with `BILL_END_DAY = 22` instead of 23, so a billing
period runs from the start of the 23rd of one month to the end of the 22nd of the next.

This is a control, not a proposal. It asks what the reconciliation looks like if the boundary is
put one day earlier, and the answer is unambiguous: **it gets much worse.** The 23rd is right.

## How the day-22 export was made

`BILL_END_DAY` in `src/hydro_bills/common.rs` was temporarily set to 22, `gb_peak_values` was
rebuilt and run, and the constant was then restored to 23. The working tree is back to where it
started; `cargo test` passes.

The workbook was written outside the repository, so the committed
`data/TH_Electric_Usage_23-11-2024_to_24-06-2026.xlsx` was never touched. `gb_peak_values` would
have refused to overwrite it in any case — it writes beside its input and never clobbers.

The day-22 workbook is a scratch artefact and is not kept. To reproduce it, make the same one-line
change and run the tool against a copy of the XML feed placed outside `data/`.

## How periods were paired with bills

Every bill states its meter reading period as the 23rd of one month to the 23rd of the next, and
that does not move. So a day-22 period ending on the 22nd of month *M* was paired with the bill
whose period ends on the 23rd of month *M* — the two cover substantially the same month, offset by
a day at each end.

19 complete day-22 periods paired with a bill, the same count as the day-23 comparison. Dropped:
`2026-07-22`, which holds 49 of its 720 hours because the export ends inside it.

Completeness was judged against the hours that actually elapse between the two local midnights
bounding each period, not against a fixed 720 or 744. Under this closing day the clock-change
periods hold 671 and 745 hours and are complete, exactly as they are under the 23rd.

## The comparison

| GB period ending | Bill period ending | GB kWh used | Bill kWh used | GB Demand kW | Bill Demand kW | GB Peak kW 7-7 | Bill Peak kW 7-7 | GB Demand kVA | Bill Demand kVA |
|---|---|---|---|---|---|---|---|---|---|
| 2024-12-22 | 2024-12-23 | 58,820.158 | 58,993.558 | 111.359997 | 111.359 | **95.039997** | **102.239** | 138.719996 | 138.719 |
| 2025-01-22 | 2025-01-23 | 66,485.878 | 66,715.198 | 117.599997 | 117.599 | 113.759997 | 113.759 | 146.879996 | 146.879 |
| 2025-02-22 | 2025-02-23 | 69,879.718 | 69,509.398 | 126.239996 | 126.239 | 117.599997 | 117.599 | 157.439996 | 157.439 |
| 2025-03-22 | 2025-03-23 | 61,594.438 | 61,739.998 | 116.159997 | 116.159 | 112.799997 | 112.799 | 142.559996 | 142.559 |
| 2025-04-22 | 2025-04-23 | 60,884.878 | 61,041.598 | **103.199997** | **106.079** | **99.839997** | **106.079** | **133.439996** | **143.519** |
| 2025-05-22 | 2025-05-23 | 66,880.678 | 66,879.358 | 121.439997 | 121.439 | 121.439997 | 121.439 | 155.999996 | 155.999 |
| 2025-06-22 | 2025-06-23 | 69,908.158 | 70,767.238 | 145.919996 | 145.919 | **131.999996** | **141.119** | 176.639996 | 176.639 |
| 2025-07-22 | 2025-07-23 | 70,209.718 | 69,175.078 | **142.559996** | **140.639** | **141.119996** | **140.639** | **171.839996** | **170.879** |
| 2025-08-22 | 2025-08-23 | 74,045.878 | 74,301.358 | 131.519996 | 131.519 | 131.519996 | 131.519 | 158.879996 | 158.879 |
| 2025-09-22 | 2025-09-23 | 57,642.718 | 57,442.198 | **110.879997** | **105.599** | 102.239997 | 102.239 | **137.759996** | **129.119** |
| 2025-10-22 | 2025-10-23 | 56,950.198 | 56,802.478 | 105.599997 | 105.599 | 98.399997 | 98.399 | 123.839997 | 123.839 |
| 2025-11-22 | 2025-11-23 | 59,208.958 | 58,993.438 | 94.559997 | 94.559 | 91.679997 | 91.679 | 118.559997 | 118.559 |
| 2025-12-22 | 2025-12-23 | 58,372.199 | 58,428.718 | 96.479997 | 96.479 | 96.479997 | 96.479 | 119.519997 | 119.519 |
| 2026-01-22 | 2026-01-23 | 60,944.758 | 61,313.518 | **101.279997** | **116.639** | **101.279997** | **116.639** | **121.919997** | **140.159** |
| 2026-02-22 | 2026-02-23 | 68,238.958 | 68,008.678 | 125.279996 | 125.279 | 125.279996 | 125.279 | 154.079996 | 154.079 |
| 2026-03-22 | 2026-03-23 | 60,950.038 | 61,188.358 | 122.879997 | 122.879 | 122.879997 | 122.879 | 147.839996 | 147.839 |
| 2026-04-22 | 2026-04-23 | 71,824.918 | 72,006.838 | 131.519996 | 131.519 | 131.039996 | 131.039 | 158.399996 | 158.399 |
| 2026-05-22 | 2026-05-23 | 71,518.678 | 71,475.838 | 130.559996 | 130.559 | 130.559996 | 130.559 | 161.759996 | 161.759 |
| 2026-06-22 | 2026-06-23 | 77,215.078 | 77,292.718 | 153.119996 | 153.119 | 152.639996 | 152.639 | 183.359995 | 183.359 |

Bold marks a demand figure that does not match the bill.

## The demand figures stop agreeing

**13 of the 57 demand comparisons fail**, across 6 of the 19 periods. Under the 23rd, all 57 match.

| Period (bill) | Figure | Day-22 GB | Bill | Miss |
|---|---|---|---|---|
| 2024-12-23 | Peak kW 7-7 | 95.039997 | 102.239 | −7.20 |
| 2025-04-23 | Demand kW | 103.199997 | 106.079 | −2.88 |
| 2025-04-23 | Peak kW 7-7 | 99.839997 | 106.079 | −6.24 |
| 2025-04-23 | Demand kVA | 133.439996 | 143.519 | −10.08 |
| 2025-06-23 | Peak kW 7-7 | 131.999996 | 141.119 | −9.12 |
| 2025-07-23 | Demand kW | 142.559996 | 140.639 | +1.92 |
| 2025-07-23 | Peak kW 7-7 | 141.119996 | 140.639 | +0.48 |
| 2025-07-23 | Demand kVA | 171.839996 | 170.879 | +0.96 |
| 2025-09-23 | Demand kW | 110.879997 | 105.599 | +5.28 |
| 2025-09-23 | Demand kVA | 137.759996 | 129.119 | +8.64 |
| 2026-01-23 | Demand kW | 101.279997 | 116.639 | −15.36 |
| 2026-01-23 | Peak kW 7-7 | 101.279997 | 116.639 | −15.36 |
| 2026-01-23 | Demand kVA | 121.919997 | 140.159 | −18.24 |

This is the decisive evidence, and it is the kind a demand figure gives that an energy total cannot.
A month's peak is a single interval. Move the boundary by one day and that interval either stays
inside the period or falls out of it — there is no partial effect. Six periods peaked in one of the
two days the boundary shift moves, and for those the day-22 window reports a different hour
entirely.

`2026-01-23` is the clearest: the real peak of 116.639 kW is excluded outright, and the window
falls back on a 101.279 kW second-best. A demand charge computed that way would understate the
month by 15 kW.

The misses go both ways, which rules out a simple systematic offset. Day-22 sometimes drops the
true peak (2026-01-23, 2025-04-23) and sometimes picks up a larger one from a day the bill does not
cover (2025-07-23, 2025-09-23).

## The energy totals get worse by an order of magnitude

| GB period ending | Bill period ending | Hours | GB − Bill (kWh) | as % of bill |
|---|---|---|---|---|
| 2024-12-22 | 2024-12-23 | 720 | −173.400 | −0.2939% |
| 2025-01-22 | 2025-01-23 | 744 | −229.320 | −0.3437% |
| 2025-02-22 | 2025-02-23 | 744 | +370.320 | +0.5328% |
| 2025-03-22 | 2025-03-23 | 671 | −145.560 | −0.2358% |
| 2025-04-22 | 2025-04-23 | 744 | −156.720 | −0.2567% |
| 2025-05-22 | 2025-05-23 | 720 | +1.320 | +0.0020% |
| 2025-06-22 | 2025-06-23 | 744 | −859.080 | −1.2140% |
| 2025-07-22 | 2025-07-23 | 720 | +1,034.640 | +1.4957% |
| 2025-08-22 | 2025-08-23 | 744 | −255.480 | −0.3438% |
| 2025-09-22 | 2025-09-23 | 744 | +200.520 | +0.3491% |
| 2025-10-22 | 2025-10-23 | 720 | +147.720 | +0.2601% |
| 2025-11-22 | 2025-11-23 | 745 | +215.520 | +0.3653% |
| 2025-12-22 | 2025-12-23 | 720 | −56.519 | −0.0967% |
| 2026-01-22 | 2026-01-23 | 744 | −368.760 | −0.6014% |
| 2026-02-22 | 2026-02-23 | 744 | +230.280 | +0.3386% |
| 2026-03-22 | 2026-03-23 | 671 | −238.320 | −0.3895% |
| 2026-04-22 | 2026-04-23 | 744 | −181.920 | −0.2526% |
| 2026-05-22 | 2026-05-23 | 720 | +42.840 | +0.0599% |
| 2026-06-22 | 2026-06-23 | 744 | −77.640 | −0.1004% |

## Side by side

| | closing on the 23rd | closing on the 22nd |
|---|---|---|
| Demand figures matching the bill | **57 of 57** | 44 of 57 |
| Periods with every demand figure right | **19 of 19** | 13 of 19 |
| Periods matching on kWh to under 0.01 | **6** | 0 |
| Mean absolute kWh difference | **20.3** | 262.4 |
| Largest absolute kWh difference | **93.7** | 1,034.6 |
| Total over 19 periods, GB − Bill | **−104.3 kWh** (−0.0084%) | −499.6 kWh (−0.0402%) |

Per-period, GB − Bill in kWh:

| Period (bill) | closing on the 23rd | closing on the 22nd |
|---|---|---|
| 2024-12-23 | +0.000 | −173.400 |
| 2025-01-23 | +0.000 | −229.320 |
| 2025-02-23 | −0.000 | +370.320 |
| 2025-03-23 | −85.560 | −145.560 |
| 2025-04-23 | −10.200 | −156.720 |
| 2025-05-23 | +1.680 | +1.320 |
| 2025-06-23 | −29.640 | −859.080 |
| 2025-07-23 | +38.520 | +1,034.640 |
| 2025-08-23 | −13.560 | −255.480 |
| 2025-09-23 | +13.080 | +200.520 |
| 2025-10-23 | +1.801 | +147.720 |
| 2025-11-23 | +83.880 | +215.520 |
| 2025-12-23 | +0.001 | −56.519 |
| 2026-01-23 | +0.000 | −368.760 |
| 2026-02-23 | +0.000 | +230.280 |
| 2026-03-23 | −93.720 | −238.320 |
| 2026-04-23 | −1.560 | −181.920 |
| 2026-05-23 | +2.160 | +42.840 |
| 2026-06-23 | −11.160 | −77.640 |

The day-23 column contains six exact agreements and its worst case is 93.7 kWh. The day-22 column
contains none, and its worst case is eleven times larger. The one period where day-22 does
marginally better on energy — 2025-05-23, 1.320 against 1.680 — is noise at 0.002%, and its demand
figures are right under both.

## What this settles

`BILL_END_DAY = 23` is confirmed by the meter data itself, not only by the wording printed on the
invoice. Moving it a day earlier breaks 13 demand figures that currently match exactly and enlarges
the mean energy error thirteenfold. Nothing here suggests the boundary is anywhere but where the
bill says it is.

It also put a bound on the clock-change anomaly the day-23 report flags. Those three outliers
(−85.6, +83.9, −93.7 kWh) survive here and are, if anything, larger, so they were never an artefact
of the closing day. That pointed the investigation at the clock the boundary runs on rather than the
date it falls on, which is where the answer was: see
[`dst-energy-anomaly-pre-fix.md`](dst-energy-anomaly-pre-fix.md).
