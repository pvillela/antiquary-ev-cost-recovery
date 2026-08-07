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
