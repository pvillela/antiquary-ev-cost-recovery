#!/usr/bin/env python3
# scripts/make-may-mock.py
#
# Builds a mock May 2026 session report from the real June one.
#
# There is only one real Evolute session report in `data`, and it covers June. A billing period
# runs from the 24th to the 23rd, so estimating one takes the reports for two adjacent months --
# and with only June there is nothing to pair it with. This produces the missing May half.
#
# May rather than an arbitrary month because the pair has to line up with the Green Button export,
# which reaches 2026-06-24: May plus June covers the period ending 2026-06-23 and no later period
# is available.
#
# The three populations in the output are each there for a reason:
#
# - Records dated 1 June to 1 July are shifted back 31 days, landing on 1 to 31 May, and are given
#   fresh ids. They are May's own sessions.
# - The two June records that already fall on 31 May are carried over *verbatim*, ids included. A
#   session at the end of a month appears in both months' reports, and merging the two must
#   recognise the copies as one session rather than counting it twice. This is the case
#   `MergedSessions::merge_sessions` collapses.
# - June's reused `Charge_Session_ID` is mirrored: the two records carrying `S37487` in June take
#   one shared new id here, so May has a reused id of its own. Its own, not June's -- merging the
#   two months then shows two independent collisions rather than one four-way one, which is the
#   likelier shape in practice. This is the case that raises `AnomalyKind::DuplicateId`.
#
# Everything else about a record -- station, vehicle, durations, energy -- is the June figure
# unchanged, so the mock has the shape of real data rather than of a generator's idea of it. Ids
# come from a seeded PRNG, so a rerun reproduces the same file.
#
# The output is never overwritten: move or delete it first. Same rule as `gb_peak_values`, and for
# the same reason -- `data` is gitignored, so a file there has no history to recover from.
#
# Usage:
#     python3 scripts/make-may-mock.py

import csv
import random
import sys
from datetime import datetime, timedelta
from pathlib import Path

DATA = Path(__file__).resolve().parent.parent / "data"
SOURCE = DATA / "Session_Report_June_1_2026-June_30_2026.csv"
TARGET = DATA / "Session_Report_May_1_2026-May_31_2026-mock.csv"

# June 1 lands on May 1, and July 1 -- the day June's report overruns into -- on May 31.
SHIFT = timedelta(days=31)
# The report states wall times to the minute, with the hour not zero-padded.
STAMP = "%Y-%m-%d %-H:%M"
# The date whose sessions both months' reports carry.
SHARED_DAY = "2026-05-31"
# The id June reuses across two unrelated sessions.
REUSED_IN_JUNE = "S37487"
# Fixed, so the file is reproducible.
SEED = 20260523


def shift(stamp: str) -> str:
    """The reported wall time moved back a month, in the format the report writes."""
    return (datetime.strptime(stamp.strip(), "%Y-%m-%d %H:%M") - SHIFT).strftime(STAMP)


def main() -> int:
    if TARGET.exists():
        print(
            f"{TARGET} already exists. Move or delete it first -- this script never overwrites "
            f"its output.",
            file=sys.stderr,
        )
        return 1
    if not SOURCE.exists():
        print(f"no such report: {SOURCE}", file=sys.stderr)
        return 1

    with SOURCE.open(newline="") as f:
        reader = csv.DictReader(f)
        fieldnames = reader.fieldnames
        rows = list(reader)

    taken = {r["Charge_Session_ID"] for r in rows}
    rng = random.Random(SEED)

    def fresh_id() -> str:
        """An id no record in either report carries."""
        while True:
            candidate = f"S{rng.randrange(10000, 100000)}"
            if candidate not in taken:
                taken.add(candidate)
                return candidate

    # Drawn once and given to both records carrying the reused id, so the collision survives the
    # renumbering instead of being quietly resolved by it.
    reused_in_may = fresh_id()

    for row in rows:
        if row["Conn_DateTime_Start"].startswith(SHARED_DAY):
            continue  # Carried over as June states it: same id, same figures, same day.
        if row["Charge_Session_ID"] == REUSED_IN_JUNE:
            row["Charge_Session_ID"] = reused_in_may
        else:
            row["Charge_Session_ID"] = fresh_id()
        row["Conn_DateTime_Start"] = shift(row["Conn_DateTime_Start"])
        row["Conn_DateTime_End"] = shift(row["Conn_DateTime_End"])

    # CRLF, which is what Evolute's own exports use.
    with TARGET.open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames, lineterminator="\r\n")
        writer.writeheader()
        writer.writerows(rows)

    print(f"{TARGET}")
    print(f"{len(rows)} rows; reused id {reused_in_may}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
