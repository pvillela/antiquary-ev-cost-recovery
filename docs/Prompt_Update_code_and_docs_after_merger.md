# Update code and docs after merger of `ev-peak-contrib` and `green-button` projects

Two projects, `ev-peak-contrib` and `green-button`, were merged into one and are now modules `sessions` and `green_button`, respectively, and  here. Their histories are in branches `history-ev-peak-contrib` and `history-green-button`.

In addition to the directory structure adjustments, several changes have been introduced:
- New method `Session::adj_conn_start`; change of `Session::adj_conn_end` from field to method.
- New columns `adj_conn_start_utc` and `adj_conn_start` to be added to the Excel conversion of the CSV Session Report. With the introduction of these new fields, I would like to have all fields not coming from the CSV placed at the right end of the sheet.
- Updated determination of `InconsistentDuration` anomaly -- see `docs/sessions/time-reporting-uncertainty.md`. The implementation of it that I started in `sessions::excel` was based on a previous version of the document and needs to be revised to align with the document.
- I suggest that the `sessions::excel::Row` struct be modified to include a `session: Session` field to avoid duplication of logic.
- New `time` module that combines and attempts to rationalize time logic from both original modules.
- New placeholder `hydro_bills` module that will be developed in the future.
- `data` folder that was version-controlled in original `green-button` module has been stripped out of history and is now gitignored for the sake of privacy in what is currently a public repo.
- Example of revised fixture function pattern in `tests::common` and `tests::green_button`. This pattern should be used for all fixtures in all modules.
- New module `sessions::energy` to be used to compute the energy consumption of EV charging during a billing period. As a billing period overlaps two session report months, the sessions from 2 session reports will be merged by new function `sessions::DedupedSessions::merge_sessions` to feed into `sessions::energy::tou_kwh`.
- GUI binary has been renamed to `ev_cost_recovery` in anticipation of addition of more functionality.

There are probably other changes I left out of the above list.

Additional changes to be made by you:
- `sessions` functionality: produce log files for conversion from CSV to Excel and reading of Excel.
- `green_button` functionality: produce log file for conversion from XML to Excel.
- Warning to be reported is session start/end times don't fall on the time grid defined by TIME_GRID_STEP.
- Maintenance manual: consider changing TIME_GRID_STEP if Evolute changes time resolution of session start/end times (which would have generated a warning according to the previous point). 
- In crate-level README.md, add links to module-level README.md files located under `docs`.
- Consolidate in crate module `time` any redundant date and time functionality currently in crate modules `sessions` and `green_button`.
- Fix any broken inter-document references or links under `docs`.
- Fix .github/workflows if broken.


Review the merged code and docs to identify gaps, errors, and other areas for improvement in code, comments, and docs.