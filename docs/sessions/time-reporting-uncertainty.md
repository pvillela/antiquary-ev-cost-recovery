# Time reporting uncertainty

## Terminology

- `rep_*`: reported values.
- `real_*`: real values.
- `adj_*`: as in our code.

## Givens

```
// `evolute_truncate` function truncates to `EV_STEP`
// `our_truncate` function truncates to `OUR_STEP`

rep_start == evolute_truncate(real_start)

// due to truncation AND uncertainty about last second inclusion/exclusion
rep_end == evolute_truncate(real_end) || rep_end == evolute_truncate(real_end - 1s)

adj_start = our_truncate(rep_start)
adj_end = our_truncate(rep_end)
```

## Inconsistent duration anomaly

For a normal session:

```
real_start + conn_duration == real_end

==> rep_end <= real_start + conn_duration < rep_end + EV_GRID + 1s
==> rep_end <= rep_start + conn_duration < rep_end + 2*EV_GRID + 1s
```

Assuming EV_GRID <= OUR_GRID:

```
rep_end <= rep_start + conn_duration < rep_end + 2*OUR_GRID + 1s
```