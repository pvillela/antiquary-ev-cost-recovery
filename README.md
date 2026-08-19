# EV Cost Recovery

Works out how much of a building's electricity cost is attributable to EV charging.

A demand charge is levied on the highest 15-minute average the building reached in a billing
period. That figure is metered for the building as a whole, and nothing measures how much of it the
chargers were responsible for. This software estimates that share from the two records that do
exist: the utility's interval data, and the charging network's session report.

## The two modules

| Module | Reads | Answers |
|---|---|---|
| [`green_button`](docs/green_button/README.md) | Toronto Hydro's Green Button export, an ESPI XML feed of hourly meter readings | When did the building peak, and at what kW and kVA? Which billing period was that in? |
| [`sessions`](docs/sessions/README.md) | The Evolute monthly session report, a CSV of charging sessions | Over a chosen interval, how much of the demand was EV charging — as a bracket, not a point? |

Both write Excel workbooks, because a workbook is what a figure gets checked in.

`sessions` reports a **bracket** rather than a single number, and that is deliberate. Session start
and end times are reported only to the minute, so a session's true extent is known to within a
minute at each end. Every figure therefore runs from what the reported times least support to what
they most support. See [`docs/sessions/time-reporting-uncertainty.md`](docs/sessions/time-reporting-uncertainty.md)
for the derivation.

`hydro_bills` is the third module, and is empty. It is where the tariff arithmetic will go — the
step from *how much power* to *how much money*.

## Documentation

- [`docs/sessions/README.md`](docs/sessions/README.md) — the estimation logic, the workbook, the
  interval-of-interest rules
- [`docs/green_button/README.md`](docs/green_button/README.md) — the ESPI feed, the peak values, the
  workbook layout
- [`docs/time/README.md`](docs/time/README.md) — the time zone, the DST fold and how it is resolved,
  the time grid
- [`docs/maintenance-manual.md`](docs/maintenance-manual.md) — what to check before changing a
  constant, how to regenerate the golden files, the invariants nothing enforces
- [`docs/sessions/time-reporting-uncertainty.md`](docs/sessions/time-reporting-uncertainty.md) — the
  specification the consistency checks are derived from

## Building and running

```sh
cargo build --release      # the desktop app, ev_cost_recovery
cargo test                 # everything
cargo run --example sessions -- <workbook.xlsx>
```

The command-line tools are `ev_csv_to_xlsx` (session report to workbook), `ev_peak_cli` (estimate
over an interval) and `gb_peak_values` (Green Button feed to workbook). Each prints its usage when
run with no arguments.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

Released binaries link third-party crates. Their licences and copyright notices
are generated at release time as `THIRD-PARTY-NOTICES.md`, which ships in each
release archive and is readable from the app's About window. It is not committed
here, since it goes stale as soon as a dependency moves. To produce a copy:

```
bash scripts/gen-notices.sh
```

A release build will not compile without it: `build.rs` checks that the notices
were generated from the current `Cargo.lock`, so a release binary cannot carry a
list that has fallen behind what is linked into it. Debug builds do not need it.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
