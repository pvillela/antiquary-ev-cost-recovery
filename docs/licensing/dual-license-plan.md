# Dual-license under MIT OR Apache-2.0

**Status: implemented.** Kept as the record of why the licensing is shaped this
way. Follow-on work is in [third-party-notices-plan.md](third-party-notices-plan.md).

## Context

The project ships one license file. The working tree shows the first step already
taken by hand: `LICENSE` is deleted (staged) and an identical `LICENSE-MIT` is
untracked. The remaining work is to add the Apache-2.0 half and to make the dual
license visible where consumers look — crate metadata and the README.

Target end state: the standard Rust-ecosystem dual license. A user may take the
code under either license, at their option.

## Current state

- `LICENSE-MIT` — MIT text, "Copyright (c) 2026 Paulo Villela". Content is
  unchanged from the old `LICENSE`. Untracked.
- `LICENSE` — deleted, staged.
- `Cargo.toml` — no `license` field.
- `README.md` — 3 lines, no license section.
- No source file carries a copyright header, and nothing in `src/`, `build.rs`,
  `.github/`, or `docs/` refers to a license. So no headers need updating.

## Changes

### 1. Add `LICENSE-APACHE` (new file, repo root)

Copy the canonical Apache 2.0 text already on this machine:

```
cp /usr/local/rustup/toolchains/stable-x86_64-unknown-linux-gnu/share/doc/cargo/LICENSE-APACHE ./LICENSE-APACHE
```

This is the exact file the Rust project ships (201 lines, appendix boilerplate
left as the unfilled `[yyyy] [name of copyright owner]` template, which is how
the appendix is meant to appear in a `LICENSE-APACHE` file). Verify afterwards
that it starts with `Apache License / Version 2.0, January 2004`.

### 2. `Cargo.toml` — add license metadata

In the `[package]` table, after `edition`:

```toml
license = "MIT OR Apache-2.0"
```

SPDX expression, the conventional spelling for this pairing. No `license-file`
key — the two are mutually exclusive, and `license` is the one tools read.

### 3. `README.md` — add a License section

Append:

```markdown
## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](../../LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
```

The Contribution paragraph is the standard Rust wording. It is what makes the
dual license hold for code other people send in; without it, contributions
arrive under no stated terms.

### 4. Stage the file moves

`git add LICENSE-MIT LICENSE-APACHE` so the rename of `LICENSE` -> `LICENSE-MIT`
and the new file are both tracked. The deletion of `LICENSE` is already staged.
No commit unless asked.

## Assumptions

- No per-file SPDX headers. Rust projects normally rely on the root license
  files plus `Cargo.toml`; adding headers to every source file is a different,
  larger decision. Say so if you want them.
- Copyright holder and year stay "2026 Paulo Villela", matching `LICENSE-MIT`.

## Verification

1. `cargo metadata --no-deps --format-version 1 | grep -o '"license":"[^"]*"'`
   -> prints `"license":"MIT OR Apache-2.0"`.
2. `cargo build` — confirms `Cargo.toml` still parses.
3. `head -3 LICENSE-APACHE` and `head -3 LICENSE-MIT` — both files present with
   the right text.
4. `git status` — shows `LICENSE` deleted, `LICENSE-MIT` and `LICENSE-APACHE`
   added, `Cargo.toml` and `README.md` modified.

## Decisions taken during review

Two questions came up after the plan was approved, recorded here because the
answers are not obvious from the result.

**Why `LICENSE-APACHE` carries no copyright year or holder.** The Apache 2.0
text is copied verbatim and has no slot for a name. Its grant refers to
"Licensor", defined by the terms themselves, so no name is needed for the
license to operate — unlike MIT, whose copyright line sits inside the grant. The
`Copyright [yyyy] [name of copyright owner]` line at the end belongs to the
appendix, which is an instruction block for pasting into source file headers,
not part of the license. Leaving it unfilled is what rust-lang, tokio, and serde
all do.

**Why no `NOTICE` file.** Apache 2.0 section 4(d) is conditional — "If the Work
includes a NOTICE text file" — so omitting one is not a gap in coverage. A
NOTICE would make attribution binding on downstream redistributors of derivative
works; the copyright is already stated in `LICENSE-MIT` and retained under
section 4(c) in source form. Judged not worth the downstream obligation at this
project's size.
