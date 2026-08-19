# Third-party attribution for shipped binaries

**Status: implemented.** Prior work is recorded in
[dual-license-plan.md](dual-license-plan.md).

## Context

The dual-license work is **done and applied**: `LICENSE-MIT` + `LICENSE-APACHE` at
the root, `license = "MIT OR Apache-2.0"` in `Cargo.toml`, License and
Contribution sections in `README.md`. Nothing below revisits that.

What remains is a real gap it exposed. `.github/workflows/release-build.yaml`
ships standalone `ev_cost_recovery` binaries that statically link **494 packages**,
several with attribution obligations that survive into the executable:

- ~105 crates under bare `MIT` — the notice must accompany "all copies or
  substantial portions".
- `epaint_default_fonts` — `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`.
  The `AND` is binding: egui embeds Ubuntu Light in the binary.
- `unicode-ident` — `(MIT OR Apache-2.0) AND Unicode-3.0`.
- `encoding_rs` — `(Apache-2.0 OR MIT) AND BSD-3-Clause`.

Today the release archives contain a bare binary and nothing else.

No forced copyleft exists in the graph. The only GPL/LGPL appearances
(`self_cell`, `r-efi`) are `OR` alternatives with permissive options.

**Decided against:** a rust-style `COPYRIGHT` file (its licensing-statement and
in-tree-exception roles are already covered, and this tree has no exceptions),
per-file SPDX headers, and a root `NOTICE` file.

**Outcome:** a generated notices file, embedded in the GUI *and* shipped in both
release archives, kept honest by CI.

## Changes

### 1. `about.toml` (new, repo root) — cargo-about config

```toml
# Only what actually ships. Filtering here drops graph entries that never reach a
# release binary, such as r-efi, which is only pulled in for UEFI targets.
targets = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]

ignore-build-dependencies = true
ignore-dev-dependencies = true

# cargo-about resolves an OR expression to the first accepted entry, so the order
# is the preference order. GPL-2.0-only and LGPL-2.1-or-later are deliberately
# absent: excluding them is what makes self_cell resolve to Apache-2.0 and r-efi
# to MIT, rather than a choice we would have to state anywhere.
accepted = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "0BSD",
    "Unlicense",
    "BSL-1.0",
    # AND-ed onto other terms rather than offered as an alternative, so these are
    # not optional: the run fails if they are not accepted.
    "Unicode-3.0",
    "OFL-1.1",
    "Ubuntu-font-1.0",
]
```

That accepted list is the exact distinct set present in the graph today, minus
the two copyleft alternatives. Verify option names against the installed
cargo-about version — they have moved between releases.

### 2. `about.md.hbs` (new, repo root) — output template

Handlebars template producing Markdown: a short header, then one section per
distinct license (`{{#each licenses}}`) with the crates using it
(`{{#each used_by}}`) followed by the full license text.

`cargo about init` scaffolds a starting template; trim it to this shape. Use
triple-stache (`{{{name}}}`, `{{{text}}}`) — handlebars HTML-escapes by default,
which turns the quotes in `BSD 3-Clause "New" or "Revised" License` into
`&quot;` in a file nobody will render as HTML.

**Size, as built: 265 KB, not the ~60–90 KB first estimated.** That estimate
assumed one MIT text shared by all its crates. cargo-about groups by license
*text*, not by identity, and each MIT crate carries its own copyright line, so
the file holds 135 distinct MIT sections. That is what MIT requires — every
copyright notice, not one representative — so the size is correct rather than a
template fault.

The same grouping repeats the Apache 2.0 terms **8 times**, which is pure noise:
the eight sections are byte-identical apart from the appendix, where some crates
filled in their own copyright (`Copyright 2011 Google Inc.`) and others left the
`[yyyy] [name of copyright owner]` template, plus one 75-line short form. Left
as cargo-about emits it. Collapsing them would mean a generator that discards a
crate's actual license file, and the duplication is nearly free once compressed.

**Measured cost of embedding** (release profile, `strip = "symbols"`, same code
with only the embedded file swapped):

| | Bytes | Delta |
|---|---|---|
| Binary with notices | 25,984,824 | |
| Binary without | 25,714,232 | **+270,592 (+1.05%)** |
| tar.gz with | 10,238,713 | |
| tar.gz without | 10,218,771 | **+19,942 (+0.20%)** |

License text is repetitive, so it compresses about 14:1 and the download grows
by roughly 20 KB. Size is not a reason to avoid embedding, and it is not a reason
to deduplicate.

### 3. `THIRD-PARTY-NOTICES.md` — generated, stamped, never committed

`scripts/gen-notices.sh` cds to the repository root, installs cargo-about if it
is absent, runs it, and appends a stamp naming the inputs the file was made from:

```
<!-- inputs-sha256: 5009646a... -->
```

Gitignored. A committed copy is stale the moment a dependency moves, and a stale
notice is worse than none: it names crates the binary no longer carries and omits
ones it does.

**The stamp is what makes freshness enforceable rather than procedural.**
`Cargo.lock` fixes every crate version, and crates.io versions are immutable, so
`Cargo.lock` + `about.toml` + `about.md.hbs` determine the output completely.
`build.rs` recomputes that hash on release builds and refuses to compile on a
mismatch, a missing stamp, or a missing file. Ordering the CI steps correctly is
no longer what keeps releases honest — a local `cargo build --release` is held to
the same standard.

Rejected alternative: having `build.rs` invoke cargo-about itself on release
builds. Verified to work — no deadlock, despite invoking cargo from a build
script — but cargo-about takes **16–30 s** per run, which every release build
would pay, and it would require the tool installed wherever a release is built.
The stamp check costs microseconds.

### 3a. `build.rs` — profile decides

| | Debug | Release |
|---|---|---|
| File absent | placeholder | **hard error** |
| File present, stamp matches | placeholder | embeds it |
| File present, stamp stale | placeholder | **hard error** |
| File present, no stamp | placeholder | **hard error** |

Debug never reads the tree's copy at all, so a leftover file cannot quietly
change what a development build reports. `sha2` goes in `[build-dependencies]`;
it is already in the graph transitively, so it compiles nothing new, and
`ignore-build-dependencies` keeps it out of the notices it guards.

### 4. `src/bin/ev_cost_recovery/about.rs` (new) — the About modal

- `const NOTICES: &str = include_str!(concat!(env!("OUT_DIR"), "/third-party-notices.md"));`
  Reaching through `OUT_DIR` is what allows the source file to be absent.
- `pub fn modal(ctx: &egui::Context, open: &mut bool)` — an `egui::Modal`
  (available since egui 0.31; you are on 0.36) holding:
  - `app::APP_NAME`, `env!("CARGO_PKG_VERSION")`, `Copyright 2026 Paulo Villela`
  - the dual-license sentence, matching `README.md`
  - a `ScrollArea::vertical` with `NOTICES` in monospace.

Display it as plain monospace text, not rendered Markdown — rendering would mean
adding `egui_commonmark`, and the file's value here is legal completeness rather
than typography. It stays `.md` so it reads well in the archive and on GitHub.

### 5. `src/bin/ev_cost_recovery/app.rs` + `main.rs` — wire it up

- `mod about;` in `main.rs` alongside the existing module list.
- `App` gains `about_open: bool` (derives `Default`, so no constructor change).
  Keep it on `App` next to `logo`, not on `AppState` — it is window chrome, not
  workflow state.
- In the tab strip closure (`app.rs:40-56`), after the `for` loop over tabs, add
  a right-aligned About button via
  `ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), ...)`.
- Call `about::modal(root_ui.ctx(), &mut self.about_open)` after the
  `CentralPanel`.
- `Tab` (`state.rs:21`) is unchanged — About is a modal, not a third tab, so the
  landing-screen invariant that `tab: Option<Tab>` encodes stays intact.

### 6. `.github/workflows/release-build.yaml` — generate, then package

Add a `taiki-e/install-action` step for cargo-about and a `bash
scripts/gen-notices.sh` step to the existing `build` job, before
`cargo build --release`. `shell: bash` on both runners; Git Bash on
windows-latest supplies `sha256sum`.

The install step is redundant with the script's own check, and kept anyway: it
fetches a prebuilt binary in seconds where the script's fallback compiles
cargo-about from source. The script finds it already present and skips.

**The install command lives in exactly one place**, `scripts/gen-notices.sh`.
cargo-about's binary is gated behind `cli`, a non-default feature — without
`--features cli`, `cargo install cargo-about` builds the library and installs no
executable, warning rather than failing. That is easy to get wrong, so nothing
else restates it: the README, the `build.rs` placeholder, and its build-failure
message all say only "run `bash scripts/gen-notices.sh`".

No separate staleness job, and no reliance on step order: if the generation step
were removed or moved after the build, `build.rs` would fail the release rather
than let it through.

**Packaging** — replace the two per-platform copy steps with a `dist/` staging
directory on both. This also removes the current asymmetry, where the tarball
exists only to carry the executable bit:

- Linux: `mkdir -p dist`, copy the binary and the three text files in, then
  `tar -czf ev_cost_recovery-${{ matrix.target }}.tar.gz -C dist .`
- Windows: same staging in pwsh, then `upload-artifact` with `path: dist/*`.

`upload-artifact@v4` roots the archive at the least common ancestor of its
inputs. Staging is what keeps that root at `dist/`; passing the `target/...` exe
path and a root-level `LICENSE-MIT` together would nest everything under
`target/x86_64-pc-windows-msvc/release/`.

Both archives end up with `ev_cost_recovery[.exe]`, `LICENSE-MIT`, `LICENSE-APACHE`,
`THIRD-PARTY-NOTICES.md`.

### 7. `README.md`

Under the License section, note that binaries bundle third-party code, that the
notices are generated at release time rather than committed, and give the two
commands that produce a local copy. Deliberately not a link: the file is not in
the repository, so a link would be broken on GitHub.

## Out of scope (flagged, not doing)

- The workflow uploads *workflow artifacts* only. There is no `gh release create`
  step, so a `v*` tag attaches nothing to the GitHub Releases page. Say the word
  if downloads are meant to come from there.
- `ev_peak_cli`, `ev_csv_to_xlsx`, `gb_peak_values` get no `--licenses` flag.
  None of them ship in releases today.

## Verification

1. `bash scripts/gen-notices.sh` — succeeds with no "unaccepted license" errors,
   and reports the byte count and stamp. Confirm `epaint_default_fonts` appears
   under OFL-1.1 *and* Ubuntu-font-1.0, and that `self_cell` resolved to
   Apache-2.0.
2. The four `build.rs` cases, each confirmed by building:
   - debug with the file present → placeholder embedded, file ignored
   - release with no file → build fails, "they have not been generated"
   - release with an unstamped file → build fails, "no inputs-sha256 stamp"
   - release freshly generated → the real 270,796 bytes embedded
3. Staleness: append a line to `about.toml` and build release → fails with "they
   are older than the dependency graph". Revert and it builds again. This is the
   case that a CI-only guard cannot catch.
4. Run `target/release/ev_cost_recovery` under `scripts/run-gui.sh`: click About,
   confirm the window opens, the text scrolls, the version line is right, and
   that both Close and Escape dismiss it. Confirm the tabs still switch after.
5. `cargo test` — no library code is touched.
6. Dry-run packaging locally: build the `dist/` directory by hand, `tar` it, and
   list the archive to confirm the four entries sit at the archive root with no
   `target/...` nesting, and that the binary keeps its executable bit.
