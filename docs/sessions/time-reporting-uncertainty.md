# Time reporting uncertainty

## Terminology

- `rep_*`: reported values.
- `real_*`: real values.
- `adj_*`: as defined below.

## Givens

```
// `evolute_truncate` function truncates to `EV_STEP`
// `our_truncate` function truncates to `OUR_STEP`

x = evolute_truncate(y)
==> x <= y && y < x + EV_STEP

x = our_truncate(y)
==> x <= y && y < x + OUR_STEP

// Assumptions:
OUR_STEP and EV_STEP are both multiples of 1s
OUR_STEP is a multiple of EV_STEP

// Given the above assumption:
our_truncate(x) == our_truncate(evolute_truncate(x))

rep_start == evolute_truncate(real_start)

// due to truncation AND uncertainty about last second inclusion/exclusion
rep_end == evolute_truncate(real_end) || rep_end == evolute_truncate(real_end - 1s)

// define:
adj_start = our_truncate(rep_start)
adj_end = our_truncate(rep_end + 1s) + OUR_STEP // see previous comment
adj_end_delta = our_truncate(rep_end + 1s) - our_truncate(rep_end) // == 0 or OUR_STEP; usually 0
```

## Key properties of `adj_start` and `adj_end`

```
adj_start == our_truncate(rep_start)
===> adj_start <= real_start

adj_end == our_truncate(rep_end + 1s) + OUR_STEP
==> if rep_end == evolute_truncate(real_end)
    ==> adj_end == our_truncate(rep_end + 1s) + OUR_STEP
                == our_truncate(evolute_truncate(real_end) + 1s) + OUR_STEP
                >= our_truncate(evolute_truncate(real_end)) + OUR_STEP
                == our_truncate(real_end) + OUR_STEP
                > real_end

    else: rep_end == evolute_truncate(real_end - 1s)
    ==> adj_end == our_truncate(rep_end + 1s) + OUR_STEP
                == our_truncate(evolute_truncate(real_end - 1s) + 1s) + OUR_STEP
                >= our_truncate(evolute_truncate(real_end - 1s)) + OUR_STEP
                == our_truncate(real_end - 1s) + OUR_STEP
                >= real_end  // because real_end and OUR_STEP are multiles of 1s
```

## Inconsistent duration anomaly

### Definition of normal session

```
real_start + conn_duration == real_end
```

For a normal session, we have the following consistency checks.

### Consistency check 1

```
real_start + conn_duration == real_end

==> real_start + conn_duration < rep_end + EV_STEP + 1s
==> rep_start + conn_duration < rep_end + EV_STEP + 1s
==> rep_start + conn_duration < rep_end + OUR_STEP + 1s
```

### Consistency check 2

```
real_start + conn_duration == real_end

==> rep_end <= real_start + conn_duration
==> rep_end < rep_start + EV_STEP + conn_duration
==> rep_end < rep_start + OUR_STEP + conn_duration
```

### Consistency check 3

By above **Key properties of `adj_start` and `adj_end`**:

adj_start <= real_start && adj_end >= real_end
==> adj_start <= adj_end


## Appendix: Other bounds on `adj_end`

```
if rep_end == evolute_truncate(real_end)
==> adj_end == our_truncate(rep_end + 1s) + OUR_STEP
            == our_truncate(rep_end) + (our_truncate(rep_end + 1s) - our_truncate(rep_end)) + OUR_STEP
            == our_truncate(rep_end) + adj_end_delta + OUR_STEP
            == our_truncate(evolute_truncate(real_end)) + adj_end_delta + OUR_STEP
            == our_truncate(real_end) + adj_end_delta + OUR_STEP

else: rep_end == evolute_truncate(real_end - 1s)
    if evolute_truncate(real_end - 1s) == evolute_truncate(real_end)
    ==> adj_end == our_truncate(real_end) + adj_end_delta + OUR_STEP // same as above
    
    else: evolute_truncate(real_end - 1s) == evolute_truncate(real_end) - EV_STEP
    ==> adj_end == our_truncate(rep_end + 1s) + OUR_STEP
                == our_truncate(evolute_truncate(real_end - 1s) + 1s) + OUR_STEP
                >= our_truncate(evolute_truncate(real_end - 1s)) + OUR_STEP
                == our_truncate(real_end - 1s) + OUR_STEP
                >= real_end  // because real_end and OUR_STEP are multiles of 1s
```
