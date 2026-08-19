# green-button

Turns a Toronto Hydro Green Button export into a spreadsheet of billing-period peak values.

Toronto Hydro bills an interval-metered general-service account partly on **demand**: the highest
kilowatt draw in the month, and separately the highest within a 07:00–19:00 window. Those two
figures appear on the invoice as `Demand kW` and `Peak kW 7-7`, and they are what this tool
recovers from the raw meter data, along with the kVA equivalents and the energy total — so a bill
can be checked rather than taken on faith.

```
gb_peak_values data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML
```

The workbook is written beside the input, with the same name and an `.xlsx` extension. An existing
file is **never overwritten** — move or delete it first. Figures in these workbooks get reconciled
against real invoices, and a silent overwrite is how that work gets lost.

## What it produces

**`Peak_values`** — one row per billing period, newest first:

| | |
|---|---|
| `billing_period_ending` | the 23rd the period is labelled by |
| `nbr_of_intervals` | hours carrying data; highlighted light red if not what a complete period holds |
| `kwh` | energy used |
| `max_kw` … | highest kW over the whole period, when it occurred in local and UTC time, the kVA at that same interval, and its Time-of-Use period |
| `max_kw_nop` … | the same, restricted to the 07:00–19:00 demand window |
| `max_kva` … / `max_kva_nop` … | the same two, for kVA |
| `anomalies` | what went wrong in this period's hours, with counts; highlighted when non-empty |

`nop` means "no off-peak" — the value did not come from an off-peak interval. The `_tou` column
beside it says which of `OnPeak` or `MidPeak` it did come from.

**`Interval_values`** — every hour of the export, newest first: local time, UTC, kWh, kW, kVA, and
that hour's anomalies.

## The rules it applies

**Billing period.** From the start of the 24th of one month to the end of the 23rd of the next, in
**Eastern Standard Time**, labelled by that 23rd. Confirmed against an invoice stating its period as
`MAY 23 2026 TO JUN 23 2026` over 31 days.

Standard time, not prevailing local time — the boundary sits at 00:00 EST all year and does not move
when the clocks do. From March to November that is 01:00 by the wall clock. Getting this wrong is a
one-hour error at each end of every summer period, and
[`../hydro_bills/archive/dst-energy-anomaly-pre-fix.md`](../hydro_bills/archive/dst-energy-anomaly-pre-fix.md) is what it cost: a
prevailing-local boundary reproduced 6 of 19 invoices, a standard-time one reproduces all 19 to the
milli-kWh. The feed agrees — its own `IntervalBlock`s start at 05:00 UTC year-round, "a permanent
midnight-EST day boundary", as [`Toronto_Hydro_Object_Model.md`](Toronto_Hydro_Object_Model.md)
records.

Only the boundary is on standard time. Time-of-Use periods, the 07:00–19:00 demand window and the
holiday calendar are all on prevailing local time, because those are stated in the clock a customer
reads. The invoice's own on-peak and mid-peak energy is what confirms the split: it reproduces
exactly under prevailing-time TOU rules and would not under standard-time ones.

**A complete period** holds days × 24 hours, clock changes included, because a standard-time day is
always 24 hours long. The count is still derived from the two boundary instants rather than from the
day count, so it stays right whatever the boundary is; the two now simply agree. Anything else is
highlighted.

Periods of 671 and 745 hours were reported until the boundary was corrected, for the February–March
and October–November periods, and were treated as complete. They were the symptom: the invoices for
those periods state 28 and 31 days, meaning 672 and 744 hours.

**Time-of-Use periods**, from the Ontario Energy Board. Off-peak does not move between seasons; only
the on-peak and mid-peak labels swap between the midday block and the two shoulders:

| | winter (Nov 1 – Apr 30) | summer (May 1 – Oct 31) |
|---|---|---|
| off-peak | 19:00–07:00, and weekends and holidays all day | same |
| on-peak | 07:00–11:00 and 17:00–19:00 | 11:00–17:00 |
| mid-peak | 11:00–17:00 | 07:00–11:00 and 17:00–19:00 |

**The demand window** is 07:00–19:00 on business days — exactly the complement of off-peak, which is
why one predicate serves both.

**Holidays** are the OEB's Time-of-Use schedule, computed as rules rather than looked up: New Year's
Day, Family Day, Good Friday, Victoria Day, Canada Day, **Civic Holiday**, Labour Day, Thanksgiving,
Christmas and Boxing Day, plus the OEB's substitution rule — a holiday falling on a weekend also
makes the next free weekday off-peak. The Civic Holiday is on the OEB's list although it is not an
Employment Standards Act public holiday; omitting it would change the demand figures. The calendar
actually applied is printed on every run.

**Arithmetic.** Every sum and maximum runs on the raw source integers; the division that turns them
into kWh, kW and kVA happens once, at cell-write time. Ties go to the earliest interval.

**Not modelled:** the distribution loss factor. Toronto Hydro multiplies metered energy by it (1.0295
on the sample invoice) to get the `Adj.` figures it bills. This workbook reports raw meter values,
which is what the invoice's unadjusted columns state. The factor is not in the Green Button data,
varies by rate class and changes between rate applications.

**Scope limit:** the current OEB schedule, with no historical variation. A feed from 2020, when
emergency flat pricing was in force, would be silently mispriced.

## How far it has been checked

Against a Toronto Hydro invoice for the period ending 2026-06-23:

| | computed | invoice |
|---|---|---|
| Demand kW | 153.119996 | 153.119 |
| Peak kW 7-7 | 152.639996 | 152.639 |
| Demand kVA | 183.359995 | 183.359 |
| kWh | 77,292.718 | 77,292.718 |

The three demand figures agree to the invoice's truncation, and the energy total agrees to the
milli-kWh. So does each of the three Time-of-Use buckets, once the loss factor is divided out.

The energy total used to read 77,281.558, 11.16 kWh under the invoice, and the whole of that gap
fell in off-peak while on-peak and mid-peak were already exact. That was the boundary error, not a
meter read a few minutes off midnight: the missing hour was 00:00–01:00 EDT on the closing day, and
midnight is off-peak, which is why it hid there.

The reconciliation now covers all 19 billing periods the export and the bills share, not this one
invoice alone — see [`../hydro_bills/green-button-vs-bills.md`](../hydro_bills/green-button-vs-bills.md).

Output was also compared against the workbook the previous Python implementation produced, over all
21 billing periods and every shared column: no differences. See `../maintenance-manual.md`,
"The port gate — run once, recorded here, then removed".

## Building and testing

```
cargo build --release
cargo test                              # unit tests, four fixture feeds, the invoice check
cargo test -- --ignored                 # adds the full 18 MB export
UPDATE_GOLDEN=1 cargo test --test fixtures_golden   # regenerate goldens, then read the diff
```

## Repository layout

| | |
|---|---|
| `src/green_button/espi.rs` | reads the ESPI feed, following its links |
| `src/time/holidays.rs`, `src/time/tou.rs` | the Ontario calendar and price periods — shared with `sessions`, hence `time` |
| `src/green_button/billing.rs`, `src/green_button/peaks.rs` | periods, expected interval counts, the four maxima |
| `src/green_button/excel.rs` | the workbook, driven by two column tables |
| `src/green_button/common.rs` | the domain types, and `METER_INTERVAL` |
| `docs/maintenance-manual.md` | invariants, procedures, and what would force a re-check — one manual for the whole crate, with a Green Button part |
| `tests/fixtures/green_button/billed_period.xlsx` | the current formatting standard — open this to see what the output should look like |
| `docs/green_button/reference/` | the workbook this replaced, kept as provenance and never read by code |
| `docs/green_button/archive/python/` | the previous implementation; `explore_model.py` is still current |
