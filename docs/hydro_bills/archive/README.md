# Archive

Superseded reports, kept as provenance. **The figures in these files are wrong for the current
code** — they were produced while `green_button` cut billing periods at prevailing local midnight,
which is an hour out from March to November.

They are here because they are the evidence the fix rests on, and because a reconciliation that
found a real error is worth being able to re-read.

| | |
|---|---|
| `green-button-vs-bills-pre-fix.md` | the first reconciliation: 57 of 57 demand figures right, 6 of 19 energy totals, and three clock-change periods out by about an hour |
| `green-button-vs-bills-day-22-pre-fix.md` | a control, regenerating the export with `BILL_END_DAY = 22`. It is worse everywhere, which confirmed the 23rd and ruled the closing day out as the cause of the clock-change gaps |
| `dst-energy-anomaly-pre-fix.md` | the investigation that found the cause: the boundary belongs on standard time, not prevailing local time |

The current reconciliation is [`../green-button-vs-bills.md`](../green-button-vs-bills.md), where
all 19 periods agree.
