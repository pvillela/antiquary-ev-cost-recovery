# Green Button against the bills

> **These figures predate a fix.** The energy differences reported here were a period-boundary
> error, traced in [`dst-energy-anomaly-pre-fix.md`](dst-energy-anomaly-pre-fix.md) and since corrected:
> `green_button` now cuts periods on standard time and reproduces all 19 invoices to the milli-kWh.
> The workbook compared here was generated before that change. The demand figures are unaffected and
> were correct throughout.

What the Green Button meter export says a billing period held, beside what Toronto Hydro invoiced
for that same period. Every demand figure agrees. The energy totals differ slightly, and the
differences have a shape.

## Sources

| | |
|---|---|
| `GB` | `data/TH_Electric_Usage_23-11-2024_to_24-06-2026.xlsx`, sheet `Peak_values` — columns `kwh`, `max_kw`, `max_kw_nop`, `max_kva` |
| `Bill` | `data/hydro_bills/*.pdf`, the `Your Electricity Usage` table — `kWh Used`, `Demand kW`, `Peak kW 7-7`, `Demand kVA` |

Bill figures were read with `hydro_bill_dump`, not by eye:

```sh
cargo run --release --bin hydro_bill_dump -- data/hydro_bills/TH_5728140000_2026_06_29.pdf
```

## Which periods are here

The 19 periods that are **complete** in the export and also have a bill.

Dropped:

- `2024-11-23` and `2026-07-23` — present in the export, but partial (24 and 25 hours against a
  full month), so the export's totals for them are not comparable to a bill. Bills for both exist.
- `2024-07-23`, `2024-08-23`, `2024-09-23`, `2024-10-23` — bills exist, but the export starts on
  2024-11-23 and does not cover them.

Every remaining period matched exactly one bill; no period is billed twice.

## The comparison

| Billing period ending | GB kWh used | Bill kWh used | GB Demand kW | Bill Demand kW | GB Peak kW 7-7 | Bill Peak kW 7-7 | GB Demand kVA | Bill Demand kVA |
|---|---|---|---|---|---|---|---|---|
| 2024-12-23 | 58,993.558 | 58,993.558 | 111.359997 | 111.359 | 102.239997 | 102.239 | 138.719996 | 138.719 |
| 2025-01-23 | 66,715.198 | 66,715.198 | 117.599997 | 117.599 | 113.759997 | 113.759 | 146.879996 | 146.879 |
| 2025-02-23 | 69,509.398 | 69,509.398 | 126.239996 | 126.239 | 117.599997 | 117.599 | 157.439996 | 157.439 |
| 2025-03-23 | 61,654.438 | 61,739.998 | 116.159997 | 116.159 | 112.799997 | 112.799 | 142.559996 | 142.559 |
| 2025-04-23 | 61,031.398 | 61,041.598 | 106.079997 | 106.079 | 106.079997 | 106.079 | 143.519996 | 143.519 |
| 2025-05-23 | 66,881.038 | 66,879.358 | 121.439997 | 121.439 | 121.439997 | 121.439 | 155.999996 | 155.999 |
| 2025-06-23 | 70,737.598 | 70,767.238 | 145.919996 | 145.919 | 141.119996 | 141.119 | 176.639996 | 176.639 |
| 2025-07-23 | 69,213.598 | 69,175.078 | 140.639996 | 140.639 | 140.639996 | 140.639 | 170.879996 | 170.879 |
| 2025-08-23 | 74,287.798 | 74,301.358 | 131.519996 | 131.519 | 131.519996 | 131.519 | 158.879996 | 158.879 |
| 2025-09-23 | 57,455.278 | 57,442.198 | 105.599997 | 105.599 | 102.239997 | 102.239 | 129.119996 | 129.119 |
| 2025-10-23 | 56,804.279 | 56,802.478 | 105.599997 | 105.599 | 98.399997 | 98.399 | 123.839997 | 123.839 |
| 2025-11-23 | 59,077.318 | 58,993.438 | 94.559997 | 94.559 | 91.679997 | 91.679 | 118.559997 | 118.559 |
| 2025-12-23 | 58,428.719 | 58,428.718 | 96.479997 | 96.479 | 96.479997 | 96.479 | 119.519997 | 119.519 |
| 2026-01-23 | 61,313.518 | 61,313.518 | 116.639997 | 116.639 | 116.639997 | 116.639 | 140.159996 | 140.159 |
| 2026-02-23 | 68,008.678 | 68,008.678 | 125.279996 | 125.279 | 125.279996 | 125.279 | 154.079996 | 154.079 |
| 2026-03-23 | 61,094.638 | 61,188.358 | 122.879997 | 122.879 | 122.879997 | 122.879 | 147.839996 | 147.839 |
| 2026-04-23 | 72,005.278 | 72,006.838 | 131.519996 | 131.519 | 131.039996 | 131.039 | 158.399996 | 158.399 |
| 2026-05-23 | 71,477.998 | 71,475.838 | 130.559996 | 130.559 | 130.559996 | 130.559 | 161.759996 | 161.759 |
| 2026-06-23 | 77,281.558 | 77,292.718 | 153.119996 | 153.119 | 152.639996 | 152.639 | 183.359995 | 183.359 |

## The demand figures agree everywhere

All 57 demand comparisons — 19 periods × `Demand kW`, `Peak kW 7-7`, `Demand kVA` — match. In every
case the bill's figure is the export's figure truncated to three decimals, never rounded:
`153.119996` is billed as `153.119`, not `153.120`. Truncating each GB value to three decimals and
comparing gives zero mismatches.

That is the result that matters most. The demand charge is levied on these three numbers, and this
says the export reproduces all three from raw meter data, over 19 periods, without a single miss.

The residual `…9996` / `…9997` tails are the meter's raw integers divided at cell-write time; they
are an artefact of the division, not a disagreement.

## The energy totals differ slightly

| Billing period ending | Hours in period | GB − Bill (kWh) | as % of bill |
|---|---|---|---|
| 2024-12-23 | 720 | +0.000 | +0.0000% |
| 2025-01-23 | 744 | +0.000 | +0.0000% |
| 2025-02-23 | 744 | −0.000 | −0.0000% |
| 2025-03-23 | **671** | **−85.560** | −0.1386% |
| 2025-04-23 | 744 | −10.200 | −0.0167% |
| 2025-05-23 | 720 | +1.680 | +0.0025% |
| 2025-06-23 | 744 | −29.640 | −0.0419% |
| 2025-07-23 | 720 | +38.520 | +0.0557% |
| 2025-08-23 | 744 | −13.560 | −0.0183% |
| 2025-09-23 | 744 | +13.080 | +0.0228% |
| 2025-10-23 | 720 | +1.801 | +0.0032% |
| 2025-11-23 | **745** | **+83.880** | +0.1422% |
| 2025-12-23 | 720 | +0.001 | +0.0000% |
| 2026-01-23 | 744 | +0.000 | +0.0000% |
| 2026-02-23 | 744 | +0.000 | +0.0000% |
| 2026-03-23 | **671** | **−93.720** | −0.1532% |
| 2026-04-23 | 744 | −1.560 | −0.0022% |
| 2026-05-23 | 720 | +2.160 | +0.0030% |
| 2026-06-23 | 744 | −11.160 | −0.0144% |

Over all 19 periods: GB 1,241,971.285 kWh against 1,242,075.562 kWh billed — 104.277 kWh under,
0.0084%.

Five periods agree to the milli-kWh. The rest are within ±0.06%, except three.

### The three outliers are the three clock-change periods

The largest discrepancy in each direction falls on a period whose hour count is not 720 or 744:

| Period | Hours | What happens inside it | GB − Bill | One hour at that period's average |
|---|---|---|---|---|
| 2025-03-23 | 671 | clocks go forward | −85.560 | 91.9 |
| 2026-03-23 | 671 | clocks go forward | −93.720 | 91.1 |
| 2025-11-23 | 745 | clocks go back | +83.880 | 79.3 |

In all three the gap is close to **one hour of that period's average consumption**, and its sign
follows the direction the clocks moved: short in the spring-forward periods, long in the
fall-back one. The three non-DST periods with the next largest gaps sit at 38.5, 29.6 and 13.6 kWh,
so these stand out at roughly two to three times the ordinary noise.

[`dst-energy-anomaly-pre-fix.md`](dst-energy-anomaly-pre-fix.md) resolves this. The period boundary was at
prevailing local midnight where the meter's is at 00:00 EST, so from March to November the export's
period was an hour out. The hour counts 671 and 745 were part of the symptom rather than a fact
about the calendar: the invoices for those periods state 28 and 31 days, meaning 672 and 744 hours.

### The ordinary differences

They have the same cause. Away from the clock changes the boundary is shifted at *both* ends, so the
hour count survives and only the difference between two midnight hours shows — a few kWh either way,
small enough to pass for meter-read noise. It is not noise: every one of these gaps is accounted for
to the milli-kWh by the two 00:00 hours the shift swaps.

The check `docs/green_button/README.md` recorded for 2026-06-23 pointed at this all along. On-peak
and mid-peak energy reproduced that invoice exactly while the whole −11.16 kWh difference fell in
off-peak — which is where a misplaced midnight hour has to land, off-peak running 19:00–07:00.

## Reproducing this

```sh
cargo build --release --bin hydro_bill_dump
for f in data/hydro_bills/*.pdf; do ./target/release/hydro_bill_dump "$f"; done
```

against sheet `Peak_values` of the export. `hydro_bill_dump --lines <PDF>` shows the positioned
text if a bill ever stops parsing.
