# Green Button peak values

## Background

The file @reference/Toronto_Hydro_Object_Model.md was created using the greenbutton-objects library (see https://pypi.org/project/greenbutton-objects/) to explain the object model in the Toronto Hydro Green Button download file @TH_Electric_Usage_23-11-2024_to_24-06-2026.XML.

## General instructions and definitions

### Formatting

- Respect and preserve the existing cell formatting in the Excel spreadsheet.
- Populate date values as numbers that will be formatted in Excel as "YYYY-MM-DD NN".
- Populate date-time values as numbers that will be formatted in Excel as either "YYYY-MM-DD HH:MM NN" or "YYYY-MM-DD HH:MM".

### Off-peak

- Off-peak times consist of:
  - Anytime on weekends
  - Times on weekdays < 07:00 in local time, not UTC
  - Times on weekdays >= 19:00 in local time, not UTC
  - Anytime on Toronto statutory holidays, including Civic Holiday.

### Billing period

- The billing period for a month runs from the beginning of the 24th day of the previous month to the end of the 23rd day of the month. For billing purposes, days and months are defined in local time, not UTC.

### Python

- Use the `uv` Python version and package manager (https://docs.astral.sh/uv/concepts/projects/init/) for the installation of any required Python versions and packages.
- Use @explore_model.py as needed.
- Perform all mathematical operations (sum, max) on the original integers obtained from the source data without performing any division operations. Just before populating the data in the spreadsheet, convert values to floating point and perform any required division operations to convert to the spreadsheet column units.
- Create or update Python script(s) that I can use to process a similarly formatted data file for a different data range and produce a similar Excel spreadsheet.
- Write/update a brief instruction manual in markdown format for use of the Python script(s). Save the manual as file `out/manual.md`.

## Spreadsheet update

Use the data in file @data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML to populate cells in Excel file @out/Green_Button_Peak_Values.xslx. The spreadsheet file contains 2 tabs.

### Interval_values tab

-   Contains a table with the following columns defined in row 3 of the sheet:
    -   Interval
    -   Interval_utc
    -   kWH
    -   kW
    -   kVA

-   Populate data in the columns:
    -   Interval -- data measurement interval's start in local time.
    -   Interval_utc -- data measurement interval's start in UTC.
    -   kWh -- kWh value in the interval.
    -   kW -- kW value in the interval.
    -   kVA -- kVA value in the interval.

-   The rows should be in descending order of Interval_utc.

### Peak_values tab

-   Contains a table with the following columns defined in row 4 of the sheet:
    -   Billing_period_ending
    -   Nbr_of_intervals
    -   kWh
    -   Max_kW
    -   Max_kW_Interval
    -   Max_kW_Interval_utc
    -   Max_kW_kVA
    -   Max_kW_nop
    -   Max_kW_nop_interval
    -   Max_kW_nop_interval_utc
    -   Max_kW_nop_kVA
    -   Max_kVA
    -   Max_kVA_Interval
    -   Max_kVA_Interval_utc
    -   Max_kVA_kW
    -   Max_kVA_nop
    -   Max_kVA_nop_interval
    -   Max_kVA_nop_interval_utc
    -   Max_kVA_nop_kW

-   Populate data in the columns:
    -   Billing_period_ending -- There will be one row for each month occurring in the data. The value in this column is the date of the last day of the billing period corresponding to the month (see definition provided previously). 
    -   Nbr_of_intervals -- Number of intervals during the billing period.
    -   kWh -- The sum of kWh interval data values for the billing period.
    -   Max_kW, Max_kW_Interval, Max_kW_Interval_utc, Max_kW_kVA -- find the first interval in the billing period where the kW value is maximized. Max_kW_Interval is that interval's start in local time, Max_kW_Interval_utc is that interval's start in UTC. Max_kW is the kW value in that interval and Max_kW_kVA is the kVA value in that interval.
    -   Max_kW_nop, Max_kW_nop_Interval, Max_kW_nop_Interval_utc, Max_kW_nop_kVA -- Similar to above but exclude intervals that fall in off-peak periods.
    -   Max_kVA, Max_kVA_Interval, Max_kVA_Interval_utc, Max_kVA_kW -- find the first interval in the billing period where the kVA value is maximized. Max_kVA_Interval is that interval's start in local time, Max_kVA_Interval_utc is that interval's start in UTC. Max_kVA is the kVA value in that interval and Max_kVA_kW is kW value in that interval.
    -   Max_kVA_nop, Max_kVA_nop_Interval, Max_kVA_nop_Interval_utc, Max_kVA_nop_kW -- Similar to above but exclude intervals that fall in off-peak periods.

-   The rows should be in descending order of Billing_period.

# Ask for clarification

- Ask any questions you may need me to answer so you can do a high-quality job.

