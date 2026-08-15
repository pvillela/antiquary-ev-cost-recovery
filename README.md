# EV Cost Recovery

Programs to facilitate the calculation of the net impact of EV charging activity on the building's electrical costs.

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
