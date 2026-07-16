# Notes on Green Button data

- It is not clear whether the Antiquary bill includes all of the 23rd of each month or just part of it.  In some months, the Green Button kWh totals differ by a few kWh from the amount on the bill, so it is possible that the cut-off used by Toronto Hydro may not be midnight on the 23rd as I assumed for the Green Button analysis.
- Green Button measurements are in 1-hour intervals, not 15-minute intervals. Assume that the KW and KVA values are from the highest 15-minute interval in the hour.
- Green Button data are time-stamped with UTC, so DST transitions are not a problem.
- However, PowerLens shows bogus entries for the missing hour (2:00 to 3:00) on March 3, 2026 and doesn't include entries for the extra hour on November 2, 2025.

