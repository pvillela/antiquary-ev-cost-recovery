# Questions for Evolute

- Where is the panel ID reported in the session report? If not currently reported, can it be added to the report? (We can work around this by maintaining a separate table that maps stations to panels, but that adds complexity to our software and work for the building admin.)
- Session start and end times as currently reported are truncated to minutes.
  - Can the reporting of session start and end times be modified to include the seconds? (This would allow us to have a clear view of whether sessions overlapping over a period of 1 minute really overlap or abut each other.)
  - If so, would the session end time represent the last second during which the EV was drawing power or would it be the first second during which the EV was not drawing power? We would prefer the latter because then `Conn_DateTime_Start + Conn_Duration = Conn_DateTime_End`, but we can work with either option.


