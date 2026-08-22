# Questions for Evolute

- How are sessions that start in one month and end in the next reported? In such cases, do the duration and energy use fields contain only the amounts that land on month the report is for?
- Session start and end times as currently reported are truncated to minutes.
  - Can the reporting of session start and end times be modified to include the seconds? (This would allow us to have a clear view of whether sessions overlapping over a period of 1 minute really overlap or abut each other.)
  - Either way: does the reported session end time denote the last second during which the EV was drawing power, or the first second during which it was not? We currently assume the former, and pad the reported end accordingly; under the latter, no padding would be needed. We can work with either, but the two call for different arithmetic, so we would rather not guess.
- Can you provide `Energy_Use` with 3 decimal places instead of just 1?


## Answers received

### 22 Jul 2026 — the three duration fields

> **Q:** What is the difference between Conn_Duration, Charge_Duration, and Active_Charge_Time?
>
> **A:** All 3 will show as almost the same, with Active charging being off by maybe 1 second due
> to rounding as it is on a slightly different timer. These fields are here for grant reporting,
> but for our system we do not track them differently.

Consequences for the software:

- The three fields do **not** distinguish connected time from charging time. A car that stays
  plugged in without drawing power is not why they differ; the difference is a rounding artefact of
  roughly a second. `Session::charge_time`'s doc comment carried the wrong reason and has been
  corrected.
- A zero `Active_Charge_Time` on a session that reports energy is therefore not "energy delivered
  in no time at all" but a reporting fault. Such rows are still surfaced rather than dropped — see
  `Sessions::spikes`.
