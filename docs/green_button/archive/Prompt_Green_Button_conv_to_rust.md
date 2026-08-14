# Prompt: Conversion from Python to Rust

The main objective is to create a Rust implementation of the existing Python code, with some changes.

## Dependencies and conventions

- See `../ev-peak-contrib` for the Rust crate used to create and write to Excel files and other conventions. I want this project's patterns and conventions to be aligned with those of `ev_peak_contrib`.

## Formatting

- Preserve the formatting currently in `bak/Green_Button_Peak_Values.xlsx`, except for functional changes described below.

## Functional changes

- For now, there is no need to be able to update an existing spreadsheet. Just need to create a new one from the source XML data.
- The output spreadsheet must be created in the same directory as the input XML file and its name must be the same as that of the input XML data file but with the `.xlsx` suffix instead of `.XML`.
- Any value in the `Nbr_of_intervals` column of the `Peak_values` tab that is not the exact number of intervals corresponding to a full billing period must be highlighted with a light red background.

## Housekeeping

- Move the existing Python code to the `history/python` directory.

## Updates to requirements

- Add the ev_peak_contrib crate as a path dependency. I plan to eventually consolidate these projects or maybe put them all under one workspace.
- The "Diff to Demand kVA" column in the sample spreadsheet's Peak_values tab was temporary only and should not have been there. I have removed it and saved the updated spreadsheet.
- Also on the Peak_values tab, add a TOU column at the end of each of the 4 value groups (Demand kW, Peak kW 7-7, Demand kVA, Peak kVA 7-7). That column indicates what TOU type the interval falls in, i.e., `OnPeak`, `MidPeak`, or `OffPeak`. The programmatic column names must be unique because later we will want to be able to read the spreadsheet relying on column names rather than column numbers. Define an `enum Tou` that has those variants.
- To implement the above item, create a `pub fn tou_partition(interval: &ev_peak_contrib::Interval) -> Vec<(Tou, ev_peak_contrib::Interval)>` such that the second components of the vector form a maximal partition of `interval` such that each element of the partition is in the TOU corresponding to the first component of the tuple. I will need this function for future functionality elsewhere.
