# Green Button Peak Values — script manual

`build_peak_values.py` reads a Toronto Hydro **Green Button (ESPI)** XML export and
populates the two-sheet Excel workbook `out/Green_Button_Peak_Values.xlsx` **in place**.

It edits the workbook's XML **surgically** — it regenerates only the data rows of the two
sheets and reuses the template's own cell-style indices, leaving `styles.xml`, the theme
and every header byte-for-byte unchanged. This is why the workbook's existing formatting
(fonts, bold, alignment, number formats, colours, widths) is preserved exactly; the script
deliberately avoids re-saving the file through a spreadsheet library, which would rewrite
the styles and strip the original formatting.

## Prerequisites

- [`uv`](https://docs.astral.sh/uv/) (Python version + package manager). The only third-party
  dependency is `holidays` (declared in `pyproject.toml`); everything else is the Python
  standard library. `uv run` installs it automatically.
- **Close the workbook first.** If `out/Green_Button_Peak_Values.xlsx` is open in
  Excel/LibreOffice the save will fail or be overwritten. A leftover `.~lock….xlsx#` file
  from a crashed session is harmless once no app is actually holding the file.

## Usage

```bash
uv run build_peak_values.py [INPUT_XML] [WORKBOOK_XLSX]
```

Defaults:

| Argument        | Default                                             |
|-----------------|-----------------------------------------------------|
| `INPUT_XML`     | `data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML` |
| `WORKBOOK_XLSX` | `out/Green_Button_Peak_Values.xlsx` (read and re-saved in place) |

To process a different date range, put the new export in `data/`, **make a copy of the
formatted workbook** for the output, and run e.g.:

```bash
cp out/Green_Button_Peak_Values.xlsx out/Green_Button_Peak_Values_2027.xlsx
uv run build_peak_values.py data/TH_Electric_Usage_new.XML out/Green_Button_Peak_Values_2027.xlsx
```

The output workbook must already contain the two sheets with their header rows
(`Peak_values` machine names in row 4, `Interval_values` names in row 3). A blank copy of
the existing workbook is the simplest template.

## What it produces

**`Interval_values`** — one row per hourly interval, newest first (descending
`Interval_utc`): local `Interval`, `Interval_utc`, and `kWh` / `kW` / `kVA`.

**`Peak_values`** — one row per monthly billing period, newest first:

- `Billing_period_ending` — the 23rd that ends the period.
- `Nbr_of_intervals`, `kWH` (period total).
- `Max_kW…` / `Max_kVA…` — the first (earliest) interval where kW / kVA peaks, with its
  local and UTC start and the companion metric at that interval.
- `…_nop…` — the same restricted to **on-peak** intervals (blank if the period has none).

## Definitions

- **Billing period** for a month = start of the 24th of the previous month → end of the
  23rd of the month, in **local** time (`America/Toronto`).
- **Off-peak** = weekends, Ontario statutory holidays incl. the August Civic Holiday, and
  weekday local times before 07:00 or at/after 19:00. On-peak is the complement.
- **Units / precision** — the raw ESPI integers are summed/maximised as integers; the
  conversion to kilo units (kWh/kW/kVA) and any division happen only when a cell is written,
  avoiding floating-point drift.

## Incremental & non-destructive behaviour

Designed to be re-run as new data arrives, without disturbing finalized figures:

- **`Peak_values`** — new billing periods are **inserted** as new rows; the most-recent
  period that already existed (the previously in-progress one) is **fully recomputed**;
  every older existing period is only **backfilled in empty cells** — populated cells of
  already-complete periods are never overwritten.
- **`Interval_values`** — on an empty sheet every interval is written; on later runs only
  intervals **newer** than those already present are inserted at the top. Existing interval
  rows are never changed.

Because complete historical rows are preserved, values you have reconciled against actual
bills stay put; only the current period and genuinely new rows change.
