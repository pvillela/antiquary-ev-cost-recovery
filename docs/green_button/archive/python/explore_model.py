"""
Explore the Toronto Hydro Green Button XML file using greenbutton_objects
and print a structured summary of the object model present in the file.

Library API (greenbutton-objects==2024.7.11):
  parse_feed(filename) -> list[UsagePoint]

Classes (greenbutton_objects.resources):  UsagePoint, MeterReading, ReadingType, IntervalBlock
Classes (greenbutton_objects.objects):    DateTimeInterval, IntervalReading, ReadingQuality
Enums  (greenbutton_objects.enums):       ServiceKind, AccumulationBehaviourType, CommodityType,
                                          CurrencyCode, DataQualifierType, FlowDirectionType,
                                          KindType, PhaseCode, UomType, QualityOfReading, ...
"""

from __future__ import annotations

import datetime as dt
import xml.etree.ElementTree as ET

from greenbutton_objects.parse import parse_feed


XML_FILE = "TH_Electric_Usage_23-11-2024_to_24-06-2026.XML"

ATOM = "{http://www.w3.org/2005/Atom}"
ESPI = "{http://naesb.org/espi}"


def decode_epoch(seconds: int) -> str:
    """Decode a Green Button epoch-seconds timestamp to a UTC / EST(-5) string."""
    utc = dt.datetime.fromtimestamp(seconds, tz=dt.timezone.utc)
    est = utc.astimezone(dt.timezone(dt.timedelta(hours=-5)))
    return f"UTC {utc:%Y-%m-%d %H:%M} / EST {est:%Y-%m-%d %H:%M}"


def raw_xml_diagnostics() -> None:
    """Inspect the raw XML for facts the object model does not expose directly:

    - the LocalTimeParameters resource (DST / timezone metadata), and
    - the reading counts on the two DST-transition days, showing that the file
      uses a fixed 05:00-UTC daily grid (24 readings every day, including DST days).
    """
    tree = ET.parse(XML_FILE)
    root = tree.getroot()

    print("\n=== LocalTimeParameters (raw XML, first entry) ===")
    ltp = root.find(f".//{ESPI}LocalTimeParameters")
    if ltp is not None:
        for child in ltp:
            tag = child.tag.replace(ESPI, "")
            print(f"  {tag:14}: {child.text}")
        tz = ltp.findtext(f"{ESPI}tzOffset")
        if tz is not None:
            print(f"  -> tzOffset {tz}s = {int(tz) / 3600:+.0f}h (EST base)")

    # Reading counts per day-block for the two DST-transition days of the KWH series.
    # DST in this file: spring-forward 2025-03-09, fall-back 2025-11-02.
    print("\n=== DST-transition day blocks (identical across all 3 series) ===")
    dst_days = {"2025-03-09": "spring-forward", "2025-11-02": "fall-back"}
    seen: set[str] = set()
    for content in root.iter(f"{ATOM}content"):
        ib = content.find(f"{ESPI}IntervalBlock")
        if ib is None:
            continue
        start = ib.findtext(f"{ESPI}interval/{ESPI}start")
        if start is None:
            continue
        day = dt.datetime.fromtimestamp(int(start), tz=dt.timezone.utc).astimezone(
            dt.timezone(dt.timedelta(hours=-5))
        ).strftime("%Y-%m-%d")
        if day in dst_days and day not in seen:
            seen.add(day)
            reads = ib.findall(f"{ESPI}IntervalReading")
            firsts = [int(r.findtext(f"{ESPI}timePeriod/{ESPI}start")) for r in reads]
            print(f"  {day} ({dst_days[day]}): {len(reads)} readings")
            print(f"    interval.start : {decode_epoch(int(start))}")
            print(f"    first reading  : {decode_epoch(firsts[0])}")
            print(f"    last reading   : {decode_epoch(firsts[-1])}")

    # Fall-back "duplicate 01:00": two consecutive readings that render as the same
    # DST-aware local wall-clock hour but are distinct epoch integers.
    print("\n=== Fall-back duplicate local hour (2025-11-02) ===")
    try:
        from zoneinfo import ZoneInfo

        toronto = ZoneInfo("America/Toronto")
    except Exception:  # pragma: no cover - tzdata may be absent
        toronto = None
    for ep in (1762059600, 1762063200):
        utc = dt.datetime.fromtimestamp(ep, tz=dt.timezone.utc)
        local = (
            utc.astimezone(toronto).strftime("%Y-%m-%d %H:%M %Z")
            if toronto is not None
            else "(tzdata unavailable)"
        )
        print(f"  epoch {ep}: UTC {utc:%Y-%m-%d %H:%M} -> local {local}")


def main() -> None:
    print(f"Parsing {XML_FILE} ...\n")
    usage_points = parse_feed(XML_FILE)      # returns list[UsagePoint]

    print(f"=== Feed ===")
    print(f"  UsagePoint count: {len(usage_points)}\n")

    for up in usage_points:
        print(f"=== UsagePoint ===")
        print(f"  title         : {up.title!r}")
        print(f"  link_self     : {up.link_self}")
        print(f"  serviceCategory: {up.serviceCategory}")
        print(f"  roleFlags     : {up.roleFlags}")
        print(f"  status        : {up.status}")
        print(f"  link_related  : {up.link_related}")
        print(f"  MeterReading count: {len(up.meterReadings)}\n")

        for i, mr in enumerate(up.meterReadings):
            print(f"  === MeterReading[{i}] ===")
            print(f"    title    : {mr.title!r}")
            print(f"    link_self: {mr.link_self}")

            rt = mr.readingType
            print(f"    === ReadingType ===")
            print(f"      title                : {rt.title!r}")
            print(f"      accumulationBehaviour: {rt.accumulationBehaviour}")
            print(f"      commodity            : {rt.commodity}")
            print(f"      currency             : {rt.currency}")
            print(f"      dataQualifier        : {rt.dataQualifier}")
            print(f"      flowDirection        : {rt.flowDirection}")
            print(f"      intervalLength       : {rt.intervalLength} s")
            print(f"      kind                 : {rt.kind}")
            print(f"      phase                : {rt.phase}")
            print(f"      powerOfTenMultiplier : {rt.powerOfTenMultiplier}")
            print(f"      uom                  : {rt.uom}")

            ibs = list(mr.intervalBlocks)
            irs_all = list(mr.intervalReadings)
            print(f"    IntervalBlock count   : {len(ibs)}")
            print(f"    IntervalReading count : {len(irs_all)}")

            if ibs:
                ib = ibs[0]
                print(f"    --- Sample IntervalBlock[0] ---")
                print(f"      title              : {ib.title!r}")
                print(f"      link_self          : {ib.link_self}")
                print(f"      interval.start     : {ib.interval.start}")
                print(f"      interval.duration  : {ib.interval.duration}")
                ir_list = list(ib.intervalReadings)
                print(f"      intervalReadings   : {len(ir_list)}")

                if ir_list:
                    ir = ir_list[0]
                    print(f"      --- Sample IntervalReading[0] ---")
                    print(f"        timePeriod.start   : {ir.timePeriod.start}")
                    print(f"        timePeriod.duration: {ir.timePeriod.duration}")
                    print(f"        value              : {ir.value}")
                    print(f"        value_units        : {ir.value_units}")
                    print(f"        value_symbol       : {ir.value_symbol}")
                    print(f"        cost               : {ir.cost}")
                    print(f"        cost_units         : {ir.cost_units}")
                    print(f"        cost_symbol        : {ir.cost_symbol}")
                    rqs = list(ir.readingQualities)
                    print(f"        readingQualities   : {len(rqs)}")
                    if rqs:
                        print(f"          quality[0]     : {rqs[0].quality}")
            print()

    # Summary counts
    print("=== Summary Counts ===")
    for up in usage_points:
        total_ib = sum(len(list(mr.intervalBlocks)) for mr in up.meterReadings)
        total_ir = sum(len(list(mr.intervalReadings)) for mr in up.meterReadings)
        print(f"  UsagePoint '{up.title}':")
        print(f"    MeterReadings    : {len(up.meterReadings)}")
        print(f"    IntervalBlocks   : {total_ib}")
        print(f"    IntervalReadings : {total_ir}")

    raw_xml_diagnostics()


if __name__ == "__main__":
    main()
