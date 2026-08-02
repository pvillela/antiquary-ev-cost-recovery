# EV Peak Power Contribution — desktop GUI

## Context

`Prompt_Estimates_gui.md` asks for a GUI version of the `estimates` binary: a polished,
self-contained, double-clickable desktop app for the major platforms.

The two CLI tools are a pipeline. `csv_to_xlsx` turns Evolute's monthly session report into a
workbook; `estimates` reads that workbook and prints the peak-contribution report for one interval
of interest. A user who double-clicks an app has the CSV, not the workbook — so a GUI covering
only `estimates` would strand them at step one. The app therefore covers both steps, in two tabs.

Facts established while planning:

- A full estimate over the real June workbook takes **24 ms**, process startup included. No
  background thread, no spinner, no async anywhere.
- `PowerEstimatesReport` exposes both public struct fields and `to_markdown()` (the ~76-column text
  in `tests/fixtures/*.report.md`, pinned by golden-file tests). The GUI can use either, per part.
- The interval boundary rules live *inside* `src/bin/ev_estimates_cli.rs` and are unreachable from
  a second binary. This is the one piece of restructuring the GUI forces.
- Errors are `Box<dyn Error>` throughout, so the GUI can display message text but cannot branch on
  error kind.

## Decisions settled

| Question | Decision |
|:---|:---|
| Scope | One app, both pipeline steps |
| Layout | Two tabs, **neither selected at launch**; free flipping afterwards |
| Landing | App name, one line of purpose, two large labelled buttons that select a tab; not reachable again |
| Batch convert | Not supported (CLI keeps it) |
| Toolkit | `egui` + `eframe` 0.35, already in `Cargo.toml` |
| Interval input | Constrained pickers — illegal states unrepresentable, no boundary-rule error text |
| DST | Gap hour absent from the dropdown; fold hour reveals an explicit EDT/EST radio, Estimate disabled until answered |
| Report display | Headline rendered natively; rest verbatim from `to_markdown()` in the same window |
| Export | Copy to clipboard **and** Save… via native dialog |
| Convert output | Beside the CSV, extension replaced; confirm before overwriting |
| Handoff | Convert tab offers "Estimate with this workbook"; never switches by itself |
| Persistence | **None** — fresh every launch |
| Targets | Linux x86_64 and Windows x86_64 only (the matrix CI already has) |
| Double-click | Windows: no console window + embedded icon. Linux: plain binary, documented |
| Naming | New `ev_peak_gui`; rename `ev_estimates_cli` → `ev_estimate_cli` |
| Appearance | Follow OS light/dark, zoom 1.15, 900×700 resizable |
| Errors | Inline red block below the button pressed, library message verbatim |
| Running | Explicit Estimate button, no live recompute |
| Default date | First day the workbook covers; covered range shown beside the filename |
| Tests | State layer unit-tested; widgets checked by eye |

## Work

### 1. Move the interval rules into the library

New `src/interval.rs`, re-exported from `src/lib.rs` alongside the other modules. It holds what
`src/bin/ev_estimates_cli.rs` has today — `LEGAL_START_MINUTES`, `OFFSETS`, the length rule, and
`resolve_local` — reshaped so both front-ends can use it:

- `checked_interval(start: civil::DateTime, length, designator) -> Result<(Timestamp, Timestamp), String>`
  — the CLI's `parse_interval` minus the text parsing.
- `local_hours(date) -> Vec<HourEntry>` and a fold/gap classifier — what the GUI needs to know
  which hour to omit and when to show the EDT/EST radio. `resolve_local`'s existing
  "ask each offset, check the zone agrees" approach already answers this; it just needs exposing.

The library core stays permissive: `groups_for_interval` is untouched, and this is an additional
checked entry point, not a restriction. `parse_interval`'s and `resolve_local`'s existing tests move
with the code; the CLI keeps only `split_designator` and its own text parsing.

### 2. Rename the CLI

`src/bin/ev_estimates_cli.rs` → `src/bin/ev_estimate_cli.rs`. Three places refer to the old name and
must agree afterwards:

- `.github/workflows/release-build.yaml` — artifact paths name `ev_estimates_cli`.
- The binary's own `USAGE` string, which says `estimates`.
- `README.md`'s Tools table, which also says `estimates`.

`ev_csv_to_xlsx` keeps its name (assumption — say if you want it renamed to match).

### 3. The GUI binary

`src/bin/ev_peak_gui/`, as `main.rs` plus modules, with `#![windows_subsystem = "windows"]` on the
GUI binary only. Split so that logic is testable without egui:

- `state.rs` — the app state and every decision about it as plain functions: which hour entries a
  date offers, whether the fold radio is needed, whether Estimate is enabled, the covered date
  range of a workbook, the default save filename. No egui types. Unit-tested inline, as the CLI
  tests itself today.
- `app.rs` — `eframe::App`, tab bar, landing screen, theme and zoom.
- `convert.rs`, `estimate.rs` — the two tabs' widgets, thin over `state.rs`.

Reused as-is: `session_csv_to_xlsx` and `session_list` (`src/excel.rs`),
`max_power_estimates_for_interval` (`src/estimates.rs`), `to_markdown()` (`src/report.rs`), the new
`src/interval.rs`, and `rfd` for native file dialogs.

**Landing.** Tab bar with neither tab active; heading, one line of purpose, two buttons —
"Convert a session report (CSV → Excel)" and "Estimate peak contribution".

**Convert tab.** One file picker (`.csv`). On Convert: if the target `.xlsx` exists, a confirmation
first; then `session_csv_to_xlsx`, showing the written path, the anomaly list (what the CLI puts on
stderr) in a scrollable area, and an "Estimate with this workbook" button that switches tabs with
the file selected.

**Estimate tab.** File picker (`.xlsx`). On selection, read once via `session_list` to show
`Covers <first> to <last>` and set the date picker to the first day covered. Then: date picker
(`egui_extras::DatePickerButton`, which takes a `jiff::civil::Date` directly, per the prototype),
hour dropdown built from `local_hours(date)`, minute as four buttons `:00 :15 :30 :45`, length as
two buttons with `1 hour` disabled unless the minute is `:00`. The fold radio appears only on the
DST-end date's ambiguous hour, with Estimate disabled until it is answered.

**Results.** Headline rendered natively from `EstimateSet` fields — the four figures and the likely
range. Note `PowerEstimatesReport::estimates` is `Option`; `report.rs:221` has prose for the `None`
case but the native block does not get it for free, so the GUI renders "No charging sessions fall
in this interval" itself. Below it, `to_markdown()` output in collapsible monospace sections
(skew margins, session groups, membership, anomalies, excluded sessions), starting expanded. Then
**Copy** (full report text to clipboard) and **Save…** (native dialog, pre-filled
`<workbook>_<YYYY-MM-DD>_<HHMM>-<HHMM>.report.md`, contents byte-identical to CLI stdout).

**Errors.** Red-bordered block below the button pressed, library message verbatim, cleared on the
next attempt.

### 4. Packaging

- `Cargo.toml`: `[[bin]]` entry for `ev_peak_gui`; consider `eframe` with `default-features = false`
  plus `glow` to keep `wgpu` out of the build.
- A `.ico` app icon asset embedded on Windows via `winresource` in `build.rs`. **An icon file has to
  be created — none exists in the repo.**
- `.github/workflows/release-build.yaml`: add the GUI binary to the artifact paths for the two
  existing targets. No new runners.
- `README.md`: a GUI section covering where to get the binary, the one-time SmartScreen "More info →
  Run anyway" on Windows, and `chmod +x` on Linux.

### 5. Loose end

`examples/google_eframe.rs` is superseded once the real app exists. Recommend deleting it; say if
you would rather keep it as a scratchpad.

## Verification

- `cargo test` — the moved interval rules keep their tests; golden-file report tests must still
  pass untouched, since nothing in `report.rs` changes.
- `cargo run --bin ev_estimate_cli -- data/Session_Report_June_1_2026-June_30_2026-what-if.xlsx "2026-06-15 16:00" 1h`
  still prints what it prints today, after the rename and the rules move.
- `cargo run --bin ev_peak_gui` and, against
  `data/Session_Report_June_1_2026-June_30_2026-what-if.xlsx` and the fixtures in `tests/fixtures/`:
  - Launch shows no tab selected; each button lands in its tab; flipping preserves state.
  - `Session_Report_Diagram` fixture — dubious groups, both estimate sets, asterisked rows.
  - `Session_Report_SkewMargin` fixture — skew-margin section present.
  - `Session_Report_Anomalies` fixture — excluded sessions and anomaly sections present.
  - Save the report and `diff` it against the CLI's stdout for the same interval — must be identical.
  - Set the date to 2026-11-01, hour 01: the EDT/EST radio appears and Estimate stays disabled
    until answered; both answers give reports an hour apart. Set it to 2026-03-08: hour 02 is
    absent from the dropdown.
  - Convert `tests/fixtures/Session_Report_Anomalies.csv` twice: the second run asks before
    overwriting; the anomaly list matches `ev_csv_to_xlsx`'s stderr.
  - Point at a non-workbook and at a deleted file: inline red message, no panic.
- Windows: confirm no console window appears behind the GUI and the icon shows in Explorer.
