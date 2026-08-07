# Green Button peak values

## Background

The file @Toronto_Hydro_Object_Model.md was created using the greenbutton-objects library (see https://pypi.org/project/greenbutton-objects/) to explain the object model in the Toronto Hydro Green Button download file @TH_Electric_Usage_23-11-2024_to_24-06-2026.XML.

## General instructions and definitions

- Format date values as "YYYY-MM-DD"
- Format date-time values as "YYYY-MM-DD HH:MM"
- Off-peak times consist of:
  - Anytime on weekends
  - Times on weekdays < 07:00 in local time, not UTC
  - Times on weekdays >= 19:00 in local time, not UTC
  - Anytime on Toronto statutory holidays, including Civic Holiday.

- The billing period for a month runs from the beginning of the 24th day of the previous month to the end of the 23rd day of the month. For billing purposes, days and months are defined in local time, not UTC.
- Use the `uv` Python version and package manager (https://docs.astral.sh/uv/concepts/projects/init/) for the installation of any required Python versions and packages.
- Use @explore_model.py as needed.

## File creation instructions

Use the data in file @TH_Electric_Usage_23-11-2024_to_24-06-2026.XML to create an Excel file Green_Button_Peak_Values.xslx containing a table with the following information:

-   There will be 14 columns, named as follows:

    -   Billing_period_ending,
    -   kWH, 
    -   Max_kW, Max_kW_Interval, Max_kW_Interval_utc,
    -   Max_kW_nop, Max_kW_nop_interval, Max_kW_nop_interval_utc,
    -   Max_kVA, Max_kVA_Interval, Max_kVA_Interval_utc,
    -   Max_kVA_nop, Max_kVA_nop_interval, Max_kVA_nop_interval_utc.

-   Populate the columns:

    -   Billing_period_ending -- There will be one row for each month occurring in the data. The value in this column is the date of the last day of the billing period corresponding to the month (see definition provided previously). 
    -   kWH -- The total of kWH interval data values for the billing period.
    -   Max_kW, Max_kW_Interval, Max_kW_Interval_utc -- find the first interval in the billing period where the kW value is maximized. Max_kW_Interval is that interval's start in local time, Max_kW_Interval_utc is that interval's start in UTC, and Max_kW is the value in that interval.
    -   Max_kW_nop, Max_kW_nop_Interval, Max_kW_nop_Interval_utc -- Similar to above but exclude intervals that fall in off-peak periods.

    -   Max_kVA, Max_kVA_Interval, Max_kVA_Interval_utc -- find the first interval in the billing period where the kVA value is maximized. Max_kVA_Interval is that interval's start in local time, Max_kVA_Interval_utc is that interval's start in UTC, and Max_kVA is the value in that interval.
    -   Max_kVA_nop, Max_kVA_nop_Interval, Max_kVA_nop_Interval_utc -- Similar to above but exclude intervals that fall in off-peak periods.

-   The rows should be in descending order of Billing_period.

Also create a Python script that I can use to process a similarly formatted data file for a different data range and produce a similar Excel spreadsheet.



