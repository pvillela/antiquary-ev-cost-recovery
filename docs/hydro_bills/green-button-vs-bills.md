# Green Button against the bills

What the Green Button meter export says a billing period held, beside what Toronto Hydro invoiced
for that same period, over every period the two share.

**They agree throughout.** All 19 periods, all four figures: energy to the milli-kWh, and every
demand figure to the invoice's own truncation.

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

The 19 that are complete in the export and also have a bill.

Dropped:

- `2024-11-23` and `2026-07-23` — present in the export but partial, holding 24 hours each against
  a full month, because the feed begins and ends inside them. Bills for both exist.
- `2024-07-23`, `2024-08-23`, `2024-09-23`, `2024-10-23` — bills exist, but the export starts on
  2024-11-23 and does not reach them.

Every remaining period matched exactly one bill; none is billed twice.

## The comparison

| Billing period ending | GB kWh used | Bill kWh used | GB Demand kW | Bill Demand kW | GB Peak kW 7-7 | Bill Peak kW 7-7 | GB Demand kVA | Bill Demand kVA |
|---|---|---|---|---|---|---|---|---|
| 2024-12-23 | 58,993.558 | 58,993.558 | 111.359997 | 111.359 | 102.239997 | 102.239 | 138.719996 | 138.719 |
| 2025-01-23 | 66,715.198 | 66,715.198 | 117.599997 | 117.599 | 113.759997 | 113.759 | 146.879996 | 146.879 |
| 2025-02-23 | 69,509.398 | 69,509.398 | 126.239996 | 126.239 | 117.599997 | 117.599 | 157.439996 | 157.439 |
| 2025-03-23 | 61,739.998 | 61,739.998 | 116.159997 | 116.159 | 112.799997 | 112.799 | 142.559996 | 142.559 |
| 2025-04-23 | 61,041.598 | 61,041.598 | 106.079997 | 106.079 | 106.079997 | 106.079 | 143.519996 | 143.519 |
| 2025-05-23 | 66,879.358 | 66,879.358 | 121.439997 | 121.439 | 121.439997 | 121.439 | 155.999996 | 155.999 |
| 2025-06-23 | 70,767.238 | 70,767.238 | 145.919996 | 145.919 | 141.119996 | 141.119 | 176.639996 | 176.639 |
| 2025-07-23 | 69,175.078 | 69,175.078 | 140.639996 | 140.639 | 140.639996 | 140.639 | 170.879996 | 170.879 |
| 2025-08-23 | 74,301.358 | 74,301.358 | 131.519996 | 131.519 | 131.519996 | 131.519 | 158.879996 | 158.879 |
| 2025-09-23 | 57,442.198 | 57,442.198 | 105.599997 | 105.599 | 102.239997 | 102.239 | 129.119996 | 129.119 |
| 2025-10-23 | 56,802.479 | 56,802.478 | 105.599997 | 105.599 | 98.399997 | 98.399 | 123.839997 | 123.839 |
| 2025-11-23 | 58,993.438 | 58,993.438 | 94.559997 | 94.559 | 91.679997 | 91.679 | 118.559997 | 118.559 |
| 2025-12-23 | 58,428.719 | 58,428.718 | 96.479997 | 96.479 | 96.479997 | 96.479 | 119.519997 | 119.519 |
| 2026-01-23 | 61,313.518 | 61,313.518 | 116.639997 | 116.639 | 116.639997 | 116.639 | 140.159996 | 140.159 |
| 2026-02-23 | 68,008.678 | 68,008.678 | 125.279996 | 125.279 | 125.279996 | 125.279 | 154.079996 | 154.079 |
| 2026-03-23 | 61,188.358 | 61,188.358 | 122.879997 | 122.879 | 122.879997 | 122.879 | 147.839996 | 147.839 |
| 2026-04-23 | 72,006.838 | 72,006.838 | 131.519996 | 131.519 | 131.039996 | 131.039 | 158.399996 | 158.399 |
| 2026-05-23 | 71,475.838 | 71,475.838 | 130.559996 | 130.559 | 130.559996 | 130.559 | 161.759996 | 161.759 |
| 2026-06-23 | 77,292.718 | 77,292.718 | 153.119996 | 153.119 | 152.639996 | 152.639 | 183.359995 | 183.359 |

## The demand figures

All 57 comparisons match — 19 periods × `Demand kW`, `Peak kW 7-7`, `Demand kVA`.

In every case the bill's figure is the export's **truncated** to three decimals, never rounded:
`153.119996` is billed as `153.119`, not `153.120`. Truncating each GB value and comparing gives
zero mismatches.

This is the result that matters most. The demand charge is levied on these three numbers, and this
says the export reproduces all three from raw meter data, over 19 periods, without a miss.

The residual `…9996` / `…9997` tails are the meter's raw integers divided at cell-write time — an
artefact of the division, not a disagreement.

## The energy totals

| Billing period ending | Days billed | Hours in period | GB − Bill (kWh) |
|---|---|---|---|
| 2024-12-23 | 30 | 720 | +0.000 |
| 2025-01-23 | 31 | 744 | +0.000 |
| 2025-02-23 | 31 | 744 | −0.000 |
| 2025-03-23 | 28 | 672 | +0.000 |
| 2025-04-23 | 31 | 744 | +0.000 |
| 2025-05-23 | 30 | 720 | −0.000 |
| 2025-06-23 | 31 | 744 | −0.000 |
| 2025-07-23 | 30 | 720 | +0.000 |
| 2025-08-23 | 31 | 744 | −0.000 |
| 2025-09-23 | 31 | 744 | +0.000 |
| 2025-10-23 | 30 | 720 | +0.001 |
| 2025-11-23 | 31 | 744 | +0.000 |
| 2025-12-23 | 30 | 720 | +0.001 |
| 2026-01-23 | 31 | 744 | +0.000 |
| 2026-02-23 | 31 | 744 | +0.000 |
| 2026-03-23 | 28 | 672 | +0.000 |
| 2026-04-23 | 31 | 744 | −0.000 |
| 2026-05-23 | 30 | 720 | −0.000 |
| 2026-06-23 | 31 | 744 | −0.000 |

Over all 19 periods: GB 1,242,075.565 kWh against 1,242,075.562 kWh billed — **+0.003 kWh**, on
1.24 million. No single period is out by more than 0.001, and the two that are sit at the bill's own
three-decimal display rather than at a real difference.

`Hours in period` is `Days billed` × 24 in **all 19** periods, the clock-change periods included.
That is a property of the boundary rather than a coincidence: a standard-time day is always 24 hours
long, so a billing period is always a whole number of days.

## What this took

An earlier version of this comparison had six exact energy matches out of nineteen, a mean error of
20 kWh and a worst case of 94. The demand figures were already perfect, which is what made the
energy gap puzzling.

The cause was the billing period boundary. `green_button` cut periods at prevailing local midnight;
Toronto Hydro cuts at 00:00 EST year-round and does not move when the clocks do, so from March to
November the export's period was an hour out at each end. Summer periods shifted at both ends and
mostly cancelled — which is why the gaps looked like meter-read noise — while a period containing a
clock change shifted at one end only and lost or gained a whole hour.

The investigation is
[`archive/dst-energy-anomaly-pre-fix.md`](archive/dst-energy-anomaly-pre-fix.md), and the reports it
supersedes are beside it. `docs/time/README.md` states the two-clock rule the fix rests on: the
boundary on standard time, Time-of-Use periods and the 07:00–19:00 demand window on prevailing local
time.

## Reproducing this

```sh
cargo build --release
./target/release/gb_peak_values data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML
for f in data/hydro_bills/*.pdf; do ./target/release/hydro_bill_dump "$f"; done
```

then read sheet `Peak_values` against the `Your Electricity Usage` block of each bill.
`gb_peak_values` never overwrites an existing workbook — move the old one aside first.
`hydro_bill_dump --lines <PDF>` shows the positioned text if a bill ever stops parsing.
