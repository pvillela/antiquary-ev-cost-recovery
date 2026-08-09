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
`America/Toronto` local time, labelled by that 23rd. Confirmed against an invoice stating its period
as `MAY 23 2026 TO JUN 23 2026` over 31 days.

**A complete period** holds as many hours as elapse between those two local midnights — *not* days
× 24. That distinction matters: a February period spans 671 hours because the clocks go forward
inside it, and an October–November one spans 745 because they go back. Both are complete. Anything
else is highlighted.

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
| kWh | 77,281.558 | 77,292.718 |

The three demand figures agree to the invoice's truncation. The energy total is 11.16 kWh under it —
0.014% — and that difference is not a period-boundary error: on-peak and mid-peak energy reproduce
the invoice **to the milli-kWh** once the loss factor is divided out, and the entire discrepancy
falls in off-peak, which is what a meter read taken a few minutes either side of local midnight
would do.

Output was also compared against the workbook the previous Python implementation produced, over all
21 billing periods and every shared column: no differences. See `docs/maintenance-manual.md`.

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
| `src/espi.rs` | reads the ESPI feed, following its links |
| `src/holidays.rs`, `src/tou.rs` | the Ontario calendar and price periods |
| `src/billing.rs`, `src/peaks.rs` | periods, expected interval counts, the four maxima |
| `src/excel.rs` | the workbook, driven by two column tables |
| `docs/maintenance-manual.md` | invariants, procedures, and what would force a re-check |
| `docs/reference/` | the workbook this replaced, kept as provenance |
| `docs/archived/python/` | the previous implementation; `explore_model.py` is still current |
