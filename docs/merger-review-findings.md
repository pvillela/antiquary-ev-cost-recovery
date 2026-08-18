# Merger review — findings and plan

Review of the merged `ev-cost-recovery` crate following the consolidation of `ev-peak-contrib` and
`green-button`. Scope, method and every design decision below were settled in a grilling session;
this document is the record and the work list.

## Method

- **Baselines.** Tip of `history-ev-peak-contrib` (`d4da113`) and tip of `history-green-button`
  (`af8ffea`). Neither shares a merge-base with `main`, so comparison is tree-vs-tree through a path
  mapping.
- **Changelog.** `docs/Prompt_Update_code_and_docs_after_merger.md` explains what the merger was
  allowed to change. A difference from baseline that it does not explain is suspect.
- **Authority.** `docs/sessions/*.md` specifications are authoritative and code follows them.
  `README.md` and the maintenance manuals are descriptive and follow the code.
- **Out of scope.** `_todo/_todo.md`, by instruction.
- **Branch.** `.claude/allowed-branches` permits only `aicode`. All work lands there.

## Status

The dedicated `green_button` read and the baseline content diff are both complete and folded in
(findings 11 and 12).

**Phases 1 and 2 are complete.** `cargo test` is green: **144 passed, 0 failed, 1 ignored** across
all targets. `cargo clippy --all-targets` reports two warnings, both pre-existing and both owned by
Phase 3.

**Measured effect of the Phase 2 fix, on the real report.** Converting
`data/Session_Report_June_1_2026-June_30_2026.csv`, 238 sessions, through three builds:

| Build | `InconsistentDuration` | `ExcessiveAvgKw` |
| --- | --- | --- |
| `history-ev-peak-contrib` tip (`d4da113`, 11 Aug 2026) | **0** | 70 |
| `main` after `1d99e29` (16 Aug 2026) | **116** of 238 | 70 |
| After Phase 2 | **0** | 70 |

Read the first row first: the pre-merger software was **already correct** on this data. `1d99e29`
introduced the defect and Phase 2 reverses it, so the right description is a **five-day regression
restored**, not a long-standing fault found. The `ExcessiveAvgKw` count is identical in all three,
which is the control: nothing else about the conversion moved.

116 is the exact figure the derivation deleted in `8fafaa2` gave for the rule it was warning
against, still quoted at `excel.rs` as *"116 of the 238 rows in this project's `data` directory"*.
The commit that deleted the warning implemented the rule it warned about.

The defect never reached a release: `1d99e29`, `c568e4a`, `046d4ba`, `bfc1eab` and `919eb0f` are the
only commits between its introduction and this fix, and the CI release build has been broken across
all of them (finding 3). Any estimate produced from a `main` build in that window was computed on
half the data.

No genuine fault was masked in the trade: the count returns to zero, and to the same zero the
pre-merger build reports.

**The golden files did not change**, which is itself a finding — neither session fixture carries a
record in the affected region, so the suite could not have caught this and cannot guard it. The two
rewritten unit tests in `excel.rs` are now the only thing pinning the band.

Phase 1 closed with four failures: the two anticipated `InconsistentDuration` pins, and two in
`sessions::peak` that were new to the review and pre-existing (finding 13). Phase 2 resolved all
four. Of the two pins, `adj_conn_end_pads_the_reported_end` needed **no change at all** — it was
asserting soundness on a record the old predicate flagged, so correcting the predicate turned it
green on its own.

---

# Findings — fix now

## 1. The `InconsistentDuration` test is wrong — it changes results, and it panics

### 1a. The accepted band is shifted one step late

`src/sessions/excel.rs:398` accepts a range one whole `TIME_GRID_STEP` later than
`docs/sessions/time-reporting-uncertainty.md` derives, at **both** ends:

```
Document (checks 1 + 2):  rep_end − STEP  <  rep_start + conn_duration  <  rep_end + STEP + 1s
Code today:               rep_end        <=  rep_start + conn_duration  <  rep_end + 2·STEP + 1s
```

Consequences:

- **Sound sessions are flagged.** Any record whose implied end lands 1–59 s *before* the reported
  end is sound per the document and is flagged by the code.
- **Real faults are missed.** Overshoot of 61–120 s passes silently.

`InconsistentDuration` is the only anomaly that removes a session from every estimate
(`src/sessions/common.rs:577`, `excel.rs:779-782`), so this is a numbers-changing defect, not a
labelling one.

Commit `1d99e29` introduced this while claiming to implement the document. The **pre-`1d99e29` code
matched the document except for one second at the upper bound** — the change moved away from its own
specification. A `// TODO: Update comments based on docs/time-reporting-uncertainty.md` remains at
`excel.rs:394`, and the prose at `common.rs:530-537` still describes the deleted rule.

Three tests pin the old behaviour and contradict the current predicate:
`src/sessions/excel.rs:1130-1137`, `:1344-1347`, `:1355`.

**Scale.** The derivation deleted in `8fafaa2` stated that requiring equal minutes "rejected roughly
half of all real records". The new lower bound is that same rejection in weaker form, so the defect
is expected to touch a large fraction of every report.

**The suite cannot see it.** `src/sessions/excel.rs:1217-1220` —
`fold_resolves_when_start_plus_duration_falls_short_of_the_reported_minute` — has a fixture (start
`01:30`, end `03:30`, duration `2:59:31`, implied end `03:29:31`) that now acquires
`InconsistentDuration` under the current rule. The test still **passes**, because it only asserts
the absence of `DstUnresolvable` and the correct UTC start. Its name and doc comment describe
behaviour the code no longer has. A green test run is therefore not evidence that this is fixed.

**The spec still describes the old rule.** `docs/sessions/README.md:257-261` is unchanged from
baseline and states the abandoned band, including the remark that "the sample data reaches to within
3 seconds of its lower edge" — which is precisely the region the new rule rejects.

### 1b. There is no start-before-end test, and the gap is a reachable panic

Nothing checks `rep_start <= rep_end`. The document's own **check 3** (`adj_start <= adj_end`) is
meant to cover it but is too weak: with `rep_start = 10:01:00`, `rep_end = 10:00:00` both sides
evaluate to `10:01:00`, so a one-minute inversion passes. It only bites beyond roughly
2·`TIME_GRID_STEP`, and it is not implemented anywhere regardless.

Today inversion is caught only as a side effect of the overshoot clause, and only when
`conn_duration` is large enough to trip it. A worked case that escapes **both** current checks:

```
conn_start = 10:02:00, conn_end = 10:00:00, conn_duration = 0

lower clause:  implied_end <  conn_end                      →  10:02 <  10:00      false
upper clause:  implied_end >= conn_end + 2·STEP + 1s        →  10:02 >= 10:02:01   false
                                                            →  not flagged
```

The session is not excluded, so it reaches the estimating logic. There:

```
adj_conn_start() = truncate(10:02:00)      = 10:02:00
adj_conn_end()   = truncate(10:00:01) + 60s = 10:01:00
```

`Session::intersects` (`src/sessions/common.rs:100`) then calls
`Interval::from_start_end(10:02, 10:01)`, and `time::base.rs:25 duration()` **panics** —
*"interval ends at … before it starts at …"*.

This is not a latent risk that the code guards against. `Session::intersects` documents the
non-inverted span as a precondition (`common.rs:89-99`) and justifies the panic on the grounds that
`InconsistentDuration` will have excluded any inverted session first. That justification cites the
**pre-`1d99e29`** soundness test, whose form did imply it. The current test does not, so the
precondition the panic relies on is no longer established.

Fix: check `rep_start <= rep_end` explicitly, as step 1 of the Phase 2 predicate.

## 2. Tests are broken in both modules, and the golden discipline is unenforceable

Every site below points at `tests/fixtures/…` where the file now lives at
`tests/fixtures/{sessions,green_button}/…`. The merger updated `fixtures_golden.rs` to the shared
helper and left the others behind.

- `tests/sessions/segment_tiling.rs:66` and `tests/sessions/report_rendering.rs:52` — each defines a
  local `fixtures()` missing the `sessions/` segment.
- `src/bin/ev_cost_recovery/state.rs:680,709` — same break.
- **`tests/green_button/invoice.rs:19`** — a private `fixtures_dir()` missing the `green_button/`
  segment. Both tests `read_to_string(...).unwrap()` and therefore **panic**. This is not a
  duplication cleanup: it is a hard break, and it kills the most valuable test in the module — its
  own header (`invoice.rs:3-5`) says it is *"the only test whose expected values come from outside
  the software"*. It reconciles against a real Toronto Hydro invoice and currently cannot fail for
  the right reason.

The fixture-function pattern the merger prompt asks for (line 13) was never applied to `sessions`:
`tests/sessions/mod.rs` holds only `mod` declarations, while `tests/green_button/mod.rs:9,13`
provides `fixture()`/`fixtures_dir()` over `tests/common/mod.rs`.

**The documented maintenance commands no longer work.** Test binaries were consolidated into
`tests/integration.rs`, so per-file `--test` targets are gone:

- `tests/green_button/fixtures_golden.rs:18` — `UPDATE_GOLDEN=1 cargo test --test fixtures_golden`.
  This is the procedure for regenerating the goldens *and* the committed standard workbook. It now
  silently matches nothing, and an operator running it concludes the goldens are current.
- `tests/green_button/full_feed.rs:7` — `cargo test --test full_feed -- --ignored --nocapture`.
- `espi.rs:304`, `peaks.rs:131`, `billing.rs:92`, `common.rs:143`, `green_button/excel.rs:578` —
  `cargo test --package green-button --lib …`. The package is `ev_cost_recovery`.
- `examples/gb_trim_fixture.rs:22,31,34` — the `USAGE` string still says `trim_fixture`. Printed to
  the user, so it names a command that does not exist.

`tests/green_button/full_feed.rs:24` also expects the sample export *"is tracked in git"*. It never
has been — `data/` is untracked on `main` and absent from the baseline too. On a fresh clone the
panic message misdirects.

## 3. Every release build fails

`.github/workflows/release-build.yaml:54,60,66,72` copy `ev_peak_gui` / `ev_peak_gui.exe`. The
binary was renamed to `ev_cost_recovery` in `046d4ba`. The workflow fails at "Stage the download".

## 4. Two definitions of `adj_conn_end`

```
src/sessions/common.rs:82  (method)      truncate_to_time_grid(conn_end + 1s) + TIME_GRID_STEP
src/sessions/excel.rs:390  (write path)  truncate_to_time_grid(conn_end)      + TIME_GRID_STEP
```

The write path lacks the `+ 1s`. They agree for every current record, because every `conn_end` has
zero seconds; they diverge the moment one carries `:59` — for `12:00:59` the method gives `12:02`,
the write path `12:01`. The write path's value goes into the `adj_conn_end_utc` column; the method's
value is what every calculation uses. The sheet and the arithmetic can therefore disagree.

Compounding it: `adj_conn_end_utc` is a **required** sheet header (`excel.rs:793`) that is never
read — `excel.rs:861` is commented out — so nothing detects the divergence.

## 5. A third, unaligned tolerance

`src/sessions/excel.rs:25` `TRUNCATION_SLACK` is a symmetric ±60 s window driving DST-fold
resolution at `:439`. The document implies an asymmetric `(−STEP, +STEP + 1s)`. Three encodings of
one uncertainty model is how the drift in finding 1 happened.

## 6. Every `See README.md, "<section>"` citation is broken

~33 of them. Before the merger each project had one README, at its root, holding the cited sections.
The root README now holds licence text only; the sections live in `docs/sessions/README.md`.

**Seven are strings the program prints to users** — `sessions/ioi.rs:207,215`,
`sessions/excel.rs:708,727`, and the `USAGE` consts in `ev_peak_cli.rs`, `ev_csv_to_xlsx.rs`,
`gb_peak_values.rs`. A release-binary user has no repository, so no path can help them; the messages
need the rule stated inline.

`"Interval of interest boundaries"`, cited five times, has **never been a heading** — it is a bold
run-in label at `docs/sessions/README.md:11`.

## 7. Stale paths throughout the documentation

Worst first:

- `docs/green_button/maintenance-manual.md:182-186` — the fixture-regeneration procedure has the
  wrong example name (`trim_fixture`, now `gb_trim_fixture`) **and** the wrong output directory. Run
  as written it writes four files to the wrong place and leaves the real fixtures untouched.
- `docs/green_button/README.md:113-120` — the "Repository layout" table is stale in **all eight
  rows**.
- `docs/sessions/maintenance-manual.md` — every `src/*.rs` path, plus the `tests/fixtures/` golden
  directory. Two of them (`src/estimates.rs`) name a file that no longer exists under any name.
- `docs/sessions/README.md:62,212,238,241` — two broken Markdown links plus `src/interval.rs`, a
  module both moved and renamed (now `src/sessions/ioi.rs`).
- `src/time/tou.rs:29` and `src/time/holidays.rs:25` cite bare `docs/maintenance-manual.md`, which
  is now two files.
- `docs/green_button/Plan_Green_Button_conv_to_rust.md` — every path broken, and it sits outside
  `archive/`.

Archive directories hold roughly a hundred further broken references. These are **left as-is** by
decision: they are historical records.

## 8. Duplicated date/time logic

- **Excel serial dates, verbatim.** Same epoch constant, same value, same intent, and duplicate
  pinning tests: `green_button/common.rs:17,120,125,134,139,153` against
  `sessions/excel.rs:19,483,488,493,948,955,1034`. **Hazard:** `excel_serial` means two different
  things in the two modules — `Timestamp` in green_button, `civil::DateTime` in sessions. Keeping
  either name changes behaviour for one side.
- **`wall_clock_instant`** (`sessions/excel.rs:478`) is duplicated inline by
  `green_button/common.rs:125`.
- **3600 encoded four times** in green_button: `espi.rs:39`, `peaks.rs:13`, `billing.rs:11`, and
  inline in `common.rs:49`.
- **`TIME_ZONE_NAME` is private** (`time/base.rs:13`) while `sessions/ioi.rs:23` hard-codes the
  matching offsets. Three intra-doc links at `ioi.rs:21,65,117` are dead as a result.

## 9. Doc comments that contradict known facts

- `src/sessions/common.rs:56` gives *"a car that stays connected without drawing power"* as a reason
  the duration fields differ. Evolute stated otherwise on **22 Jul 2026**: *"All 3 will show as
  almost the same, with Active charging being off by maybe 1 second due to rounding as it is on a
  slightly different timer. These fields are here for grant reporting, but for our system we do not
  track them differently."*
- `src/time/base.rs:62-82` documents `TIME_GRID_STEP` as *"Resolution the session report states
  session boundaries at"* — that is Evolute's step. The constant is used as **ours**
  (`truncate_to_time_grid`, and it must divide `SEGMENT_DURATION`). Lines 64–66 then instruct a
  maintainer to set it to 1 second, which would break the `SEGMENT_DURATION` invariant.
- `src/sessions/excel.rs:773` justifies the `spikes` bucket as *"energy delivered in no time at
  all"*. Under Evolute's statement a zero `Active_Charge_Time` is more likely a reporting fault.

## 10. Smaller items

- `src/sessions/energy.rs` — `TouKkh` is a typo for `TouKwh`; division by `adj_duration()` is
  unguarded when that is zero.
- `src/hydro_bills/mod.rs` is 0 bytes. The merger prompt calls the module `hydro_bill` (singular);
  the directory is plural.
- `SignedDuration` is used for quantities that cannot be negative — `parse_duration`
  (`sessions/excel.rs:237`), `CsvSession` (`:263-264`), `excel_duration` (`:493`) — while `Session`'s
  own fields are unsigned. Only `:439`/`TRUNCATION_SLACK` genuinely needs a sign.
- `docs/sessions/time-reporting-uncertainty.md` — `multiles` → `multiples`, lines 58 and 118.
  `src/time/base.rs:68` — `truncaged` → `truncated`.
- `docs/Prompt_Update_code_and_docs_after_merger.md` — `meged` (3), `duplicaton` (9),
  `grenn_button` (21), `redudant` (25), `hydro_bill` → `hydro_bills` (11).

## 13. The `peak` test helper's `adj_conn_end` inverse is wrong — two tests fail

`sessions/peak.rs:258` takes its `end` argument as the **adjusted** end and back-computes
`conn_end = end − TIME_GRID_STEP`. That inverts the old `truncate(conn_end) + STEP` formula, not the
current `Session::adj_conn_end()`, which is `truncate(conn_end + 1s) + STEP`. The two agree only when
`end` sits on the 60 s grid.

Two tests pass an `end` that does not — `20:07:30`, chosen to be exactly half a segment. Measured:

```
end passed to helper  = 20:07:30
conn_end stored       = 20:06:30   (end − 60s)
adj_conn_end()        = 20:07:00   (truncate(20:06:31) + 60s)
```

30 s short, so a segment the test builds as half-covered is measured at `7/15 = 0.4667` where it
asserts `0.5`. The commented-out `// adj_conn_end,` at `peak.rs:267` is the field that used to carry
the value directly; the helper was never re-derived when it went.

**Not a production defect.** Every `conn_end` Evolute reports lands on the minute, where the two
formulas agree. It is finding 11's *"`truncate_to_time_grid` has no direct test"* showing up as the
first thing that depended on it silently.

Fix belongs in **Phase 2**, alongside the `adj_conn_end` unification: make the helper take the
reported `conn_end` and derive the adjusted end through `Session::adj_conn_end()`, so the two cannot
drift again. The two fixtures then need ends that survive the round trip.

## 11. What the merger lost

Verified against both baselines by inventory: **every** `fn`, `struct`, `enum`, `const` and
`#[test]` in both baselines has a counterpart on `main`, with one exception. Every `docs/**` file and
every fixture survived byte-identical. The merge commits themselves (`987a74f`, `199096e`) were
clean; almost all of the loss below happened in the follow-on commits `8fafaa2` and `650e959`.

What went is **prose, one public item, and one behavioural rule**:

- **`END_PADDING`** (`history-ev-peak-contrib:src/excel.rs:18-20`) — the only deleted item. Call
  sites use `TIME_GRID_STEP` directly now; the rationale for the signed form went with it.
- **The eight-line derivation of the soundness band** (baseline `src/excel.rs:383-394`), replaced by
  the bare TODO at `excel.rs:394`. That deletion is the same commit that moved the band — see
  finding 1a.
- **`TIME_ZONE_NAME` was `pub`** (`history-ev-peak-contrib:src/common.rs:12`) and is now private
  (`time/base.rs:13`). A silent public-surface removal; both binaries and the green_button crate
  consumed it.
- **Three user-facing messages lost the zone name** — `sessions/ioi.rs:148-181` now says *"in local
  time zone"* where the baseline interpolated `America/Toronto`. A consequence of the line above. No
  test covers message text, so nothing caught it.
- **`Session::conn_start`'s doc comment** (`common.rs:50`) lost its uncertainty-window statement. It
  migrated to `adj_conn_start()`, so a reader of `conn_start` no longer learns it is truncated.
- **`TIME_GRID_STEP`'s doc** lost *"which is what makes the DST fold inference possible"* and gained
  `truncaged`. **Not a real loss** — the substance survives, better placed and fuller, on
  `reproduces_reported_end` (`sessions/excel.rs:426-434`), which documents both the truncation
  asymmetry and its role in fold resolution at the point of use. Only the typo is actionable. The
  surviving sentence about `Conn_Duration` and `Active_Charge_Time` not being truncated describes
  *Evolute's* reporting — `EV_STEP` territory — and leaves `TIME_GRID_STEP` under the Phase 3
  rewrite that Q18 already schedules.
- ~~**`Cargo.toml` lost the `umya-spreadsheet` rationale**~~ — **restored.** Three lines explaining
  that `rust_xlsxwriter` models row heights and column widths as whole pixels and cannot represent
  the template's 13.8 pt rows or 1.39-wide spacers. Load-bearing: it is the reason a maintainer must
  not swap the crate. The baseline's closing sentence, "Also the crate ev-peak-contrib uses", was
  dropped as obsolete — they are one crate now. The `roxmltree` comment beside it had survived.
- **`.gitignore` lost the reason `Cargo.lock` is committed** — that a reproducible build is what lets
  a figure in a workbook be traced to the code that produced it.

Two consequences of the band change, beyond finding 1:

- **`docs/sessions/README.md:257-261`** still states the abandoned rule verbatim. Code and spec
  contradict each other.
- **`adj_conn_end_utc` is now decorative** (`excel.rs:861` commented out). Editing that cell in a
  workbook has no effect, where it was previously authoritative, and the cell comment at
  `excel.rs:702-708` still describes it as if it were.

Also: `Row.adj_start_utc` / `adj_start_local` (`excel.rs:280-282`) are computed at `:414-416` and
never used, because `COLUMNS` is byte-identical to baseline and has no `adj_conn_start` column. The
local one costs a time-zone conversion per row for nothing. Phase 3's column reorder consumes them.

`truncate_to_time_grid` (`time/base.rs:85`) is new logic underpinning both `adj_conn_start()` and
`adj_conn_end()` and has **no direct test**.

Confirmed non-losses: `src/sessions/peak.rs` is the former `src/estimates.rs` intact (10 insertions
/ 7 deletions, all imports); `src/sessions/ioi.rs` is a rename of the baseline `src/ioi.rs`, not new
code.

## 12. `green_button` defects

The merger changed nothing but `use` paths in all five library files, the binary and the example.
These are pre-existing, found by the dedicated read.

**Correctness**

- **Series are keyed on `uom` alone** (`espi.rs:145-149`). `power_of_ten` is taken from whichever
  `IntervalBlock` is visited first and every later block for that `uom` is folded in under that
  divisor. Two `ReadingType`s legally disagreeing on `powerOfTenMultiplier` — the check at `:109`
  constrains only `intervalLength` — put one meter's readings out by a power of ten, with no anomaly
  and no error. Two meters or two `UsagePoint`s also merge into one series. The module header
  (`espi.rs:15`) promises a foreign feed "either resolves or says which link is missing"; this is
  the case that resolves to a wrong number. Fix: error when `power_of_ten` disagrees, or key on
  `(uom, reading_type_href)`.
- **Gap-filling is unbounded** (`espi.rs:216-229`). Nothing bounds the hole, and `Timestamp` spans
  roughly year −9999 to 9999, so one corrupt `<espi:start>` pushes a `Reading` per hour of the gap —
  up to ~175 M rows — then tries to render them. The failure is an OOM or an apparent hang, not a
  diagnosable error.
- **`is_complete` compares a count, not a set** (`peaks.rs:47-49,76`). A period missing one hour but
  carrying one extra misaligned reading totals correctly and reports complete, suppressing the red
  `nbr_of_intervals` fill. `kwh_total` (`peaks.rs:77`) likewise sums misaligned rows, so an
  overlapping one double-counts.
- **A non-hourly `ReadingType` anywhere aborts the whole parse** (`espi.rs:106-115`), including for a
  `uom` the tool never reads. An export adding a 900-second voltage series is rejected outright.
- **Duplicate intervals: last value silently wins** (`espi.rs:170-172`). The anomaly is raised, but
  which value survives is document order — undocumented and unasserted.
- **Element text is never trimmed** (`espi.rs:102-104`). A pretty-printed feed fails the literal
  `uom` comparison and is reported as "carries no kWh series" — a misleading diagnosis. Fails loudly
  rather than wrongly, but `examples/gb_trim_fixture.rs:126-128` does trim, so the crate is
  inconsistent with itself.
- **`date(y, m, 23)` panics** (`billing.rs:67-74`) for a reading at the extreme of the timestamp
  range. Only reachable from a corrupt feed, but it is a panic where everything else returns an
  error.
- **An empty-but-valid feed produces a silent empty workbook** and exit 0 (`espi.rs:176-186`,
  `gb_peak_values.rs:114-121`). Nothing tells the user the export was empty.

**Coverage gaps**

`Anomaly::DuplicateInterval` is never exercised — no unit test creates one. None of the four
link-chain errors (`espi.rs:101,126,129,141`) has a test, despite link-following being the module's
headline design decision. Column widths are absent from the golden dump (`fixtures_golden.rs:177-218`)
even though the `1.39` spacer widths are the stated reason for choosing `umya-spreadsheet`. Freeze
panes (`excel.rs:469-486`) are unasserted — if umya maps the two splits the other way round, the
sheets freeze 3 columns and 1 row and nothing notices.

**Quality**

`excel.rs:509-513,551-554` writes an empty string plus format, alignment and font to ~14,000 cells
that `Out::blank()` already covers. `common.rs:57-59` says "add variants freely" but variant order is
the `Ord` order, which is the order `format_counts` emits into the goldens — the rule is
append-only. `fixtures_golden.rs:184-185` describes per-row heights on `Interval_values` that the
code does not set and the golden contradicts (`billed_period.golden.txt:117`). `espi.rs:139-141` is
the one link error that omits its `href`, and the one most likely to fire on a foreign feed.
`WriteReport::interval_rows` (`excel.rs:266`) counts gap placeholders while
`PeriodValues::interval_count` excludes them, and both are printed as "intervals".

---

# Findings — deferred

Recorded with reasons, so the deferral is a decision rather than an omission.

| Item | Why deferred |
| --- | --- |
| Wiring `sessions::energy` into a caller. It is re-exported `#[allow(unused)]` and nothing calls it. | Waits for the GUI that will use it. |
| `DedupedSessions::merge_sessions` returns `duplicates`, which nothing consumes. | Becomes warnings once the GUI calls it. |
| Extending `time-reporting-uncertainty.md` to the other six `AnomalyKind`s. | A separate project. A scope note is added under the title instead, so the narrow scope is visible. |
| Splitting `EV_STEP` from `OUR_STEP` in code. | `EV_STEP` is a conceptual device for the analysis only; `OUR_STEP` is `TIME_GRID_STEP`. |
| Unifying the two DST resolvers, `sessions/excel.rs:329` and `sessions/ioi.rs:83`. | They solve different problems with deliberately different tie-breaks. Documented instead, so nobody merges them. |
| Adding a push/PR CI job. | Releases are gated on `vx.y.z-rc` tags, which already match the `v*` trigger. |
| Committing an ignore rule for `data/`. | A cloner never sees a `data/` directory, and the present arrangement is required by an existing hook. |

---

# Plan

Ordered so that each phase is measurable when it starts.

## Phase 1 — Unblock the tests — **DONE**

Nothing later can be measured until `cargo test` runs.

- [x] `tests/sessions/mod.rs` — `fixture()`/`fixtures_dir()` added, mirroring
  `tests/green_button/mod.rs` exactly.
- [x] Local duplicates deleted from `tests/sessions/segment_tiling.rs`,
  `tests/sessions/report_rendering.rs` and `tests/green_button/invoice.rs`; each now imports the
  module helper. Inputs go through `fixture()`, which asserts existence, so a moved fixture fails by
  name. Goldens keep `fixtures_dir()`, since a golden may legitimately not exist yet under
  `UPDATE_*_GOLDEN`.
- [x] `src/bin/ev_cost_recovery/state.rs` — **four** sites, not the two recorded in finding 2.
  `:442` and `:726` build the same path from `CARGO_MANIFEST_DIR` and were equally broken; both
  tests panicked in `fs::copy`.
- [x] `.github/workflows/release-build.yaml` — `ev_peak_gui` → `ev_cost_recovery` at all four sites,
  including the two tarball names. `v*` tag trigger untouched.
- [x] Stale test commands — **eleven** sites, not the seven recorded. The extra four:
  `src/time/tou.rs:205` and `src/time/holidays.rs:181` also said `--package green-button`;
  `src/sessions/excel.rs:960` said `--package ev-peak-contrib`; and
  `tests/sessions/report_rendering.rs:25` had the same `--test <file>` break as the two green_button
  ones. Unit tests now read `cargo test --lib -- <module>::test --nocapture`; integration tests read
  `cargo test --test integration -- <module>::<file>`. All three integration forms were run and
  produce the intended selection.
- [x] `examples/gb_trim_fixture.rs` — `USAGE` names `gb_trim_fixture`, and both Usage and Example
  lines give the runnable `cargo run --example gb_trim_fixture -- …` form.
- [x] `full_feed.rs:24` — the message now says the export is not in the repository and names the path
  to put in place.
- [x] Also fixed, found by `cargo clippy`: the redundant `&'static` on `MODULE_NAME` in both test
  module files.

**Also in this phase, by decision: the `espi.rs:145` guard** (finding 12). It is the only silent
wrong-number path in the crate, and closing it is small and self-contained.

The defect is not really the `power_of_ten` divisor — it is that the code assumes **one series per
`uom`** throughout (`take()` at `:176-180` removes exactly one `Series` per unit), while `series` is
keyed on `uom` alone and `or_insert_with` fixes `power_of_ten` from whichever `IntervalBlock` is
visited first. Guarding only the multiplier would still silently merge two meters that agree on
scale. Detect the second `ReadingType` for a unit instead:

```rust
let mut series: HashMap<&str, (Series, &str)> = HashMap::new();

// pass 3, after resolving `reading_type`:
if let Some((_, seen)) = series.get(uom)
    && *seen != reading_type
{
    return Err(format!(
        "ReadingTypes {seen} and {reading_type} both carry uom {uom}; \
         this tool assumes one series per unit"
    ).into());
}
```

Compare the **href**, not the mere presence of an entry: many `IntervalBlock`s legitimately share one
`ReadingType`.

**Done.** `series` is now keyed `HashMap<&str, (Series, &str)>` and `take` drops the href on the way
out. Two tests added at `espi.rs`, the first error-path tests the `ReadingType` chain has ever had:
`a_second_reading_type_for_one_unit_is_rejected` and `many_blocks_may_share_one_reading_type` — the
second pins the href comparison, so a future simplification to "an entry already exists" fails
loudly. The real 18 MB export was parsed under the guard (`full_feed`, `--ignored`) and is
unaffected.

Not fixed by this, and still open: a second meter is rejected rather than summed, and `espi.rs:109`
still validates `intervalLength` for units the tool never reads.

**Result.** 140 passed, 4 failed, 1 ignored — the ignored one being the full-export test, run
separately and green. The `InconsistentDuration` pins are two of the four, as expected. The
other two were treated as a new finding and are written up as finding 13; they are pre-existing,
verified by stashing every change above and re-running.

`cargo clippy --all-targets` reports two warnings, both pre-existing and both owned by a later phase:
the dead `Row` fields (`excel.rs:280-282`, Phase 3's column reorder) and `let_and_return` at
`sessions/common.rs:299`.

`cargo fmt --check` fails on one file: `src/hydro_bills/mod.rs` is 0 bytes and rustfmt wants a
newline. Left alone — Phase 3 gives that file a doc comment, which settles it. It is the only thing
standing between the tree and a clean `fmt --check`.

## Phase 2 — Fix the arithmetic — **DONE**

Golden files needed **no regeneration**: neither session fixture carries a record the change
reclassifies. See Status for what it did move on the real report.

### The `InconsistentDuration` test — three checks

Replace the predicate at `src/sessions/excel.rs:396-402`. Any failure raises the anomaly:

```
1.  rep_start <= rep_end
2.  rep_start + conn_duration  <  rep_end + TIME_GRID_STEP + 1s     (document check 1)
3.  rep_end - TIME_GRID_STEP   <  rep_start + conn_duration         (document check 2, rearranged)
```

Check 1 is explicit because the document's own check 3 (`adj_start <= adj_end`) is too weak: with
`rep_start = 10:01:00, rep_end = 10:00:00` both sides evaluate to `10:01:00` and the inversion
passes. It only catches inversions beyond roughly 2·`TIME_GRID_STEP`, and an inverted record with
`conn_duration == 0` escapes all three checks as currently written.

- [x] Implemented as `duration_is_consistent` in `sessions/common.rs`, not inline in `excel.rs`, so
  the document has exactly one code counterpart. `CsvSession.conn_duration` is still a
  `SignedDuration`; it is converted at the call site and a negative value is inconsistent by
  definition. Phase 3 moves that conversion to the parse boundary.
- [x] `inconsistent_duration_is_reported` rewritten. It now pins `0:31:00` as **sound** where the
  old test pinned it as a fault — the one second the pre-`1d99e29` code was also missing — and adds
  the case that check 1 alone catches.
- [x] `adj_conn_end_pads_the_reported_end` needed no change. It was already asserting soundness on a
  record the old predicate flagged.

### One `adj_conn_end` — **DONE**

Better than derived from the methods: `adj_conn_start_of` and `adj_conn_end_of` are now free
functions in `sessions/common.rs`, and `Session`'s two methods defer to them. The write path holds a
CSV record and not yet a `Session`, so methods alone would have left it recomputing. There is now
one definition of each bound in the crate, reachable from both sides.

### `TRUNCATION_SLACK` — **DONE**

Replaced by `SLACK_EARLY` / `SLACK_LATE`, the asymmetric `(−STEP, +STEP + 1s)`, both **computed
from `TIME_GRID_STEP`** rather than written out, so a change to the grid carries them with it. The
symmetric ±60 s constant is gone.

### Document amendments — **DONE**

`docs/sessions/time-reporting-uncertainty.md`:

- [x] **Result** section added, stating all three checks together with notes on why each bound is
  strict, why the window is asymmetric, and why check 1 is not implied. It names
  `duration_is_consistent` as its one implementation.
- [x] Check 3 demoted to a derived remark, with the worked one-minute inversion that defeats it.
  New **check 4**, `rep_start <= rep_end`, derived and added — it is what the software tests
  instead.
- [x] Title kept; scope note added under it.
- [x] `multiles` → `multiples`.

`docs/sessions/README.md` — the "Other" section stated the abandoned rule verbatim (finding 11). It
now states the three checks, cites the derivation and the implementation, and explains why check 1
is not redundant. Corrected here rather than deferred to Phase 4 because Phase 2 is what made it
wrong.

### Smaller correctness items — **DONE**

- [x] `sessions/energy.rs` — `TouKkh` → `TouKwh`; a zero `adj_duration()` now skips the session
  rather than dividing by it. The attribution model is unchanged.
- [x] `sessions/common.rs` — `charge_time`'s comment corrected and the Evolute quote of 22 Jul 2026
  carried in it. `Questions_for_Evolute.md` gains an **Answers received** section with the quote,
  the date, and the two consequences for the software. Note the file is at the repository root, not
  under `docs/sessions/`.
- [x] `sessions/excel.rs` — the `spikes` justification revised: a zero `Active_Charge_Time` is a
  reporting fault, not energy delivered in no time. Bucket kept.
- [x] `time/base.rs` — `truncaged` → `truncated`.

### Finding 13, scheduled here — **DONE**

`peak.rs`'s test helper now derives `conn_end` and then **asserts the round trip** through
`Session::adj_conn_end`, so it cannot silently invert a formula the code no longer has. That assert
immediately rejected both fixtures: they asked for an adjusted end of `20:07:30`, which is off the
time grid and so a session the software cannot produce. Every adjusted end is truncated, so a
15-minute segment can only be covered in whole minutes and **half is not a reachable fraction**.
Both moved to five minutes of fifteen, and the first test was renamed accordingly.

## Phase 3 — Restructure

### `Row` embeds `Session`

`src/sessions/excel.rs:272` — replace the flat timestamp fields with `session: Session`, keeping
`record` for the CSV pass-through columns. Derived columns then come from the `Session` methods,
which is what removes the second `adj_conn_end`.

### Column order

Non-CSV fields move to the right end, in this order:

```
adj_conn_start, adj_conn_end, conn_start_utc, conn_end_utc,
adj_conn_start_utc, adj_conn_end_utc, adj_conn_duration, avg_kw, anomalies
```

`adj_conn_start` and `adj_conn_start_utc` are new. Reordering is safe: the read path locates columns
by header name (`excel.rs:838`), not position.

### Log files

Plain text beside the output, `<stem>.convert.log` / `<stem>.read.log`, overwritten each run. Each
either states everything was fine or lists what was found.

- **CSV → Excel** (`session_csv_to_xlsx`): anomalies only.
- **Excel → sessions** (`session_list`): stored column values against recomputed `Session` methods.
  Restore the read-back at `excel.rs:861`. On mismatch the **recomputed** value wins and the
  mismatch is logged. `adj_conn_duration` and `avg_kw` are compared only when Excel has stored a
  cached value — `set_formula` (`:605,614`) writes none — and logged as "formula not evaluated"
  otherwise.
- **XML → Excel** (`green_button::write_workbook`, `green_button/excel.rs:278`): same convention,
  building on the existing `WriteReport`.

Discrepancies are a **separate channel from `AnomalyKind`**: a stale sheet column must not silently
change which sessions feed an estimate.

### Off-grid warning

Warn when `conn_start`/`conn_end` do not land on the grid — once per file, with a count and the
first three offending rows, log only. The message states what it means: the reported resolution has
become finer than `TIME_GRID_STEP`. It points at the maintenance manual.

### `time` module

`time` holds date/time code that more than one module could use. Module-specific calculations stay
where they are.

- **New `src/time/excel.rs`** — the Excel serial epoch and helpers, named `serial_of_instant` /
  `serial_of_civil` / `serial_of_date` plus inverses, made infallible. The rename is required, not
  cosmetic: `excel_serial` currently means two different things.
- **Move into `time`** — `wall_clock_instant`, and `TZ_OFFSETS` from `sessions/ioi.rs:23`.
- **Make `TIME_ZONE_NAME` public**, fixing the three dead doc links.
- **Parameterise truncation** — `truncate_to(ts, step)` / `is_on_grid(ts, step)`, replacing
  `truncate_to_time_grid` at ~8 call sites.
- **Move `TIME_GRID_STEP` out to `sessions`**, beside `SEGMENT_DURATION` (`common.rs:24`) and
  `LEGAL_START_MINUTES` (`ioi.rs:19`), which must agree with it. Rewrite its doc comment.
- **`METER_INTERVAL: Duration = 1h` in `green_button`**, replacing the four encodings of 3600.
- **Leave both DST resolvers alone**, and document why.

### `SignedDuration`

Convert at the parse boundary so `parse_duration`, `CsvSession` and `excel_duration` use unsigned
`Duration`, matching `Session`. `SignedDuration` remains only at `:439`/`TRUNCATION_SLACK`.

### `hydro_bills`

Keep the plural name; give the empty `mod.rs` a doc comment stating its intended scope.

## Phase 4 — Documentation

Last, because Phases 2 and 3 change what it must describe.

- **Root `README.md`** — short project overview, then links to the module docs, then the licensing
  text that is currently all it holds.
- **New `docs/time/README.md`** — the `Time zone` section moves here. `Boundaries and the time grid`
  stays in `docs/sessions/README.md`, since `TIME_GRID_STEP` moves to `sessions`.
- **One `docs/maintenance-manual.md`** — consolidate the sessions manual (§1–§6) and the
  green_button manual (§1–§10) into Shared / Sessions / Green Button parts. Cite sections **by title,
  not number** thereafter. Add the `TIME_GRID_STEP` note tied to the Phase 3 off-grid warning.
- **Reference fixes** — the ~33 `README.md` citations get explicit paths, or point at
  `docs/time/README.md` where the content moved there, plus every stale path in finding 7.
- **The seven user-facing strings** — state the rule inline, drop the path.
- **Promote `Interval of interest boundaries`** to a real heading.
- **Move `docs/green_button/Plan_Green_Button_conv_to_rust.md` into `archive/`.**
- **Leave archive references broken**, with a line at the top of each archive directory saying so.
- **`docs/Prompt_Update_code_and_docs_after_merger.md`** — fix the typos, leave the wording.

---

# Open scope decision — `green_button` correctness

Finding 12 is **pre-existing**, not merger damage: the merger changed only `use` paths in that
module. The review scope agreed at the outset gave `green_button` a lighter pass, so these are
reported rather than scheduled, and need a call before any of them enters a phase.

Ranked by what I would fix first:

| Item | Why it ranks here |
| --- | --- |
| ~~`espi.rs:145` — guard against a second `ReadingType` per unit~~ — **scheduled into Phase 1** | The only defect that produces a **wrong number silently**. Everything else fails loudly or not at all. |
| `espi.rs:218` — bound the gap fill | Turns a corrupt timestamp from an error into an OOM. Cheap to fix. |
| `peaks.rs:47` — make `is_complete` a set check, or soften its doc comment | The red completeness fill is a signal you rely on when reconciling against invoices. |
| Tests for `DuplicateInterval` and the four link-chain errors | Untested paths in the module's headline design decision. |
| The rest of finding 12 | Quality and hardening; no impact on a well-formed Toronto Hydro export. |

The remaining items are hardening against feeds this tool has never seen. If the only input is ever
Toronto Hydro's own export, deferring all of them is defensible — the note is here so that stays a
decision rather than an oversight.

---

# Verification

1. `cargo test` passes. It cannot run at all before Phase 1.
2. `cargo clippy --all-targets` and `cargo fmt --check` are clean.
3. Golden files change **once**, in Phase 2, and the diff is explainable by the
   `InconsistentDuration` change alone: sessions whose implied end falls 1–59 s before the reported
   end leave `excluded`; sessions overshooting by 61–120 s enter it.
4. `cargo run --example sessions`, `--example site_load_report`, `--example gb_trim_fixture`.
5. Convert `data/Session_Report_June_1_2026-June_30_2026.csv`, then read the workbook back. Both
   logs appear, and the read log reports no discrepancies for a workbook this crate just wrote.
6. Run `gb_peak_values` on `data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML` and reconcile
   against `docs/green_button/reference/Green_Button_Peak_Values-python-2026-07-16.xlsx`.
7. Every path in the consolidated manual's fixture-regeneration procedure resolves.
