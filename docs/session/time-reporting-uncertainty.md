# Time reporting uncertainty

**Scope.** *Terminology*, *Givens* and *Key properties* define `adj_start` and `adj_end` generally;
they underpin `Session::adj_conn_start` and `Session::adj_conn_end` and every calculation built on
them. From *Inconsistent duration anomaly* onward the document derives one thing only, the
`InconsistentDuration` anomaly. The other six `AnomalyKind`s are out of scope.

`EV_STEP` is Evolute's reporting resolution. It is a device for this analysis and appears nowhere in
the code; only `OUR_STEP`, which is `TIME_GRID_STEP`, does.

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
                >= real_end  // because real_end and OUR_STEP are multiples of 1s
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

Also implied, though too weak to be worth testing:

```
By above Key properties of adj_start and adj_end:

adj_start <= real_start && adj_end >= real_end
==> adj_start <= adj_end
```

Both sides are truncated, so an inversion smaller than roughly `2 * OUR_STEP` survives it. With
`rep_start == 10:01:00` and `rep_end == 10:00:00`, `adj_start` and `adj_end` both come to
`10:01:00` and the check passes on a record that is plainly inverted. Check 4 below is what the
software tests instead.

### Consistency check 4

```
rep_start <= rep_end
```

Not derived from the definition of a normal session — it is the same fact stated at full strength,
before truncation throws precision away. `real_start + conn_duration == real_end` with
`conn_duration >= 0` gives `real_start <= real_end`, and truncation is monotonic, so
`rep_start <= rep_end`.

## Result

The three checks the software applies. Any failure raises `InconsistentDuration`, which is the only
anomaly that removes a session from every estimate.

```
1.  rep_start <= rep_end                                          // check 4
2.  rep_start + conn_duration  <  rep_end + OUR_STEP + 1s         // check 1
3.  rep_end - OUR_STEP         <  rep_start + conn_duration       // check 2, rearranged
```

Notes on reading these:

- Every bound is **strict**. Checks 2 and 3 come from two half-open windows meeting, not from an
  interval anyone chose.
- The window checks 2 and 3 draw is **asymmetric** — one second wider late than early. That second
  is the `+ 1s` of `adj_end`: the reported end is truncated *and* it is not known whether the last
  second of the named minute is inside the session or outside it.
- Check 3 is check 2 with `rep_start + conn_duration` isolated, so all three read against the same
  quantity.
- Check 1 is not implied by the other two. A record with `rep_start` one `OUR_STEP` after `rep_end`
  and `conn_duration == 0` satisfies both of them.

Implemented by `duration_is_consistent` in `src/session/common.rs`, which is the single place the
three appear in code and the only caller-facing statement of them.

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
                >= real_end  // because real_end and OUR_STEP are multiples of 1s
```
