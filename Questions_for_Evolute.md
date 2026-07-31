# Questions for Evolute

- How are sessions that start in one month and end in the next reported? In such cases, do the duration and energy use fields contain only the amounts that land on month the report is for?
- Session start and end times as currently reported are truncated to minutes.
  - Can the reporting of session start and end times be modified to include the seconds? (This would allow us to have a clear view of whether sessions overlapping over a period of 1 minute really overlap or abut each other.)
  - Either way: does the reported session end time denote the last second during which the EV was drawing power, or the first second during which it was not? We currently assume the former, and pad the reported end accordingly; under the latter, no padding would be needed. We can work with either, but the two call for different arithmetic, so we would rather not guess.
