# EV Cost Recovery

Works out how much of a building's electricity cost is attributable to EV charging.

A demand charge is levied on the highest 15-minute average the building reached in a billing
period. That figure is metered for the building as a whole, and nothing measures how much of it the
chargers were responsible for. This software estimates that share from the two records that do
exist: the utility's interval data, and the charging network's session report.

## The four modules

Three read a source of data. The fourth is the calendar they all run on.

| Module | Reads | Answers |
|---|---|---|
| [`green_button`](docs/green_button/README.md) | Toronto Hydro's Green Button export, an ESPI XML feed of hourly meter readings | When did the building peak, and at what kW and kVA? Which billing period was that in? |
| [`sessions`](docs/sessions/README.md) | The Evolute monthly session report, a CSV of charging sessions | Over a chosen interval, how much of the demand was EV charging — as a bracket, not a point? |
| [`hydro_bills`](docs/hydro_bills/) | The PDF invoices Toronto Hydro issues | What did the utility charge, and on what terms? |
| [`time`](docs/time/README.md) | Nothing of its own — the other three call it | What does a timestamp mean here, and what does the Ontario calendar say about it? |

`time` is a module rather than a scattering of helpers because the same calendar has to be applied
identically by all three. Getting it subtly different in two places is a class of error nothing else
here would catch.

`sessions` reports a **bracket** rather than a single number, and that is deliberate. Session start
and end times are reported only to the minute, so a session's true extent is known to within a
minute at each end. Every figure therefore runs from what the reported times least support to what
they most support. See [`docs/sessions/time-reporting-uncertainty.md`](docs/sessions/time-reporting-uncertainty.md)
for the derivation.

`hydro_bills` is the least complete of the four. It reads a bill and it defines the billing period
boundary, since that is a fact about the bill; the step from *how much power* to *how much money* is
still to be written.

## Documentation

- [`docs/sessions/README.md`](docs/sessions/README.md) — the estimation logic, the workbook, the
  interval-of-interest rules
- [`docs/green_button/README.md`](docs/green_button/README.md) — the ESPI feed, the peak values, the
  workbook layout
- [`docs/hydro_bills/green-button-vs-bills.md`](docs/hydro_bills/green-button-vs-bills.md) — the
  export checked against every invoice, period by period
- [`docs/time/README.md`](docs/time/README.md) — the two clocks and which is for what, the DST fold
  and how it is resolved, the time grid
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
over an interval), `gb_peak_values` (Green Button feed to workbook) and `hydro_bill_dump` (a bill
PDF's figures). Each prints its usage when run with no arguments.

Six more report on one billing period. `peak_power_cli` gives the kW and kVA peaks, estimated from
a Green Button export and the two session reports spanning the period. `energy_cli` gives the
kilowatt-hours drawn, split by time-of-use band. Two of them price those against a Toronto Hydro
bill: `energy_cost_cli` for the consumption lines and `peak_power_cost_cli` for the three
demand-priced delivery lines. Every rate they use is read off the bill; no tariff is assumed.

The two costing tools ask for no closing date, because the bill states which period it covers.

`cost_recovery_cli` is the other side of the ledger: it prices the same kilowatt-hours at EV
cost-recovery rates you give on the command line, and reports what they recover. No bill is read
and no tax is added — the rates are yours, and what they have to cover is your decision. A schedule
is written `EFFECTIVE_DATE:ON_PEAK,MID_PEAK,OFF_PEAK`; give a second one when the rates changed
during the period, and the energy is split at local midnight on its effective date.

`cost_recovery_surplus_cli` puts the two sides together: what the rates recover, less the delivery
and energy costs, and the difference. A positive surplus means the rates covered the chargers'
share of the bill; a negative one means they fell short. It prints all three reports beneath the
summary, so every figure in the subtraction can be checked. Only the delivery and energy sides are
counted as EV cost — the customer charge, the standard supply administration charge and the
wholesale market service charge are in neither — so a surplus of zero is not breaking even on the
whole invoice.

These six need two adjacent months' session reports, since a billing period runs from the
24th to the 23rd. Only June's is real, so `scripts/make-may-mock.py` builds the May half from it —
shifted dates, fresh ids, and two deliberate quirks that exercise the merge: a session at the end
of May that both reports carry, and a reused `Charge_Session_ID` of May's own. The script explains
both, and refuses to overwrite an output that already exists.

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
