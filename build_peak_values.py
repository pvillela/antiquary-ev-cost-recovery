"""
Populate the two-sheet peak-values Excel report from a Toronto Hydro Green Button
(ESPI) XML file, editing the existing, pre-formatted workbook *in place*.

The workbook ``out/Green_Button_Peak_Values.xlsx`` is a formatted template. Rather than
re-saving it through a library (which rewrites styles.xml and loses the original cell
formatting), this script edits the workbook's XML **surgically**: it regenerates only the
data rows of the two sheets, reusing the template's own style indices, and leaves
``styles.xml``, the theme and every header byte-for-byte unchanged. The only structural
change is appending a single number-format style (``#,##0``) for the interval-count column.

Two sheets are filled:

* ``Interval_values`` -- one row per hourly interval (Interval local, Interval_utc,
  kWh, kW, kVA), descending by Interval_utc.
* ``Peak_values``     -- one row per monthly billing period with the total energy and the
  peak kW / kVA (overall and on-peak-only, the ``*_nop`` columns), descending by period.

Usage:
    uv run build_peak_values.py [INPUT_XML] [WORKBOOK_XLSX]

Defaults:
    INPUT_XML     = data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML
    WORKBOOK_XLSX = out/Green_Button_Peak_Values.xlsx   (edited in place)

Design notes (see reference/Prompt_Green_Button_peak_values.md):
  - Integer math: every sum/max is performed on the raw integer ``<espi:value>`` read
    straight from the XML. Values are converted to float and divided to kilo units
    (kWh/kW/kVA) only at the moment a cell is written.
  - Billing period for a month runs from the start of the 24th of the previous month to
    the end of the 23rd of the month; ``Billing_period_ending`` is that 23rd (local time).
  - Off-peak = weekends, Toronto (Ontario) statutory holidays incl. the Civic Holiday,
    and weekday local start times < 07:00 or >= 19:00. On-peak = the complement.
  - Incremental & non-destructive:
      * Peak_values -- already-complete billing periods keep their exact stored values;
        the most-recent (previously in-progress) period is recomputed; new periods and the
        newly-added columns are added. Rows stay in descending order.
      * Interval_values -- on an empty sheet every interval is written; on later runs only
        intervals newer than those already present are added. Existing rows are preserved.
"""

from __future__ import annotations

import datetime as dt
import os
import re
import sys
import zipfile
from collections import defaultdict
from zoneinfo import ZoneInfo

import holidays
from holidays.constants import OPTIONAL, PUBLIC

import xml.etree.ElementTree as ET

LOCAL_TZ = ZoneInfo("America/Toronto")

ATOM = "{http://www.w3.org/2005/Atom}"
ESPI = "{http://naesb.org/espi}"

# ReadingType unit-of-measure code -> logical series name.
UOM_TO_SERIES = {"72": "kwh", "38": "kw", "61": "kva"}

# Peak_values row-4 machine column names.
PEAK_COLUMNS = [
    "Billing_period_ending", "Nbr_of_intervals", "kWH",
    "Max_kW", "Max_kW_Interval", "Max_kW_Interval_utc", "Max_kW_kVA",
    "Max_kW_nop", "Max_kW_nop_interval", "Max_kW_nop_interval_utc", "Max_kW_nop_kVA",
    "Max_kVA", "Max_kVA_Interval", "Max_kVA_Interval_utc", "Max_kVA_kW",
    "Max_kVA_nop", "Max_kVA_nop_interval", "Max_kVA_nop_interval_utc", "Max_kVA_nop_kW",
]

EXCEL_EPOCH = dt.datetime(1899, 12, 30)   # serial-date origin (1900 date system)


# --------------------------------------------------------------------------- parsing


def parse_series(input_xml):
    """Read the raw XML into ``series[name][epoch_start] = raw_int`` plus the per-series
    powerOfTenMultiplier. Values stay integers exactly as stored in ``<espi:value>``."""
    root = ET.parse(input_xml).getroot()

    id_map = {}
    for entry in root.iter(f"{ATOM}entry"):
        rt = entry.find(f".//{ESPI}ReadingType")
        if rt is None:
            continue
        self_href = next(
            (l.get("href") for l in entry.findall(f"{ATOM}link") if l.get("rel") == "self"),
            None,
        )
        if self_href is None:
            continue
        name = UOM_TO_SERIES.get(rt.findtext(f"{ESPI}uom"))
        if name is None:
            continue
        id_map[self_href.rsplit("/", 1)[1]] = (
            name,
            int(rt.findtext(f"{ESPI}powerOfTenMultiplier")),
        )

    series = defaultdict(dict)
    pot = {}
    for entry in root.iter(f"{ATOM}entry"):
        ib = entry.find(f".//{ESPI}IntervalBlock")
        if ib is None:
            continue
        self_href = next(
            (l.get("href") for l in entry.findall(f"{ATOM}link") if l.get("rel") == "self"),
            "",
        )
        if "/MeterReading/" not in self_href:
            continue
        rid = self_href.split("/MeterReading/")[1].split("/")[0]
        if rid not in id_map:
            continue
        name, series_pot = id_map[rid]
        pot[name] = series_pot
        for ir in ib.findall(f"{ESPI}IntervalReading"):
            start = int(ir.findtext(f"{ESPI}timePeriod/{ESPI}start"))
            value = int(ir.findtext(f"{ESPI}value"))
            series[name][start] = value

    missing = {"kwh", "kw", "kva"} - series.keys()
    if missing:
        raise SystemExit(f"error: input file is missing series: {sorted(missing)}")
    return series, pot


# ------------------------------------------------------------------- domain helpers


def billing_period_ending(local_date):
    """Billing period ends on the 23rd; days on/after the 24th roll to next month."""
    year, month = local_date.year, local_date.month
    if local_date.day >= 24:
        month += 1
        if month == 13:
            month, year = 1, year + 1
    return dt.date(year, month, 23)


def build_holiday_set(years):
    """Ontario statutory holidays including the (optional) August Civic Holiday."""
    return holidays.CA(subdiv="ON", years=years, categories=(PUBLIC, OPTIONAL))


def is_off_peak(local_start, holiday_dates):
    """Off-peak if weekend, holiday, or weekday local hour < 7 or >= 19."""
    if local_start.weekday() >= 5:
        return True
    if local_start.date() in holiday_dates:
        return True
    hour = local_start.hour
    return hour < 7 or hour >= 19


class Reading:
    __slots__ = ("epoch", "local", "utc", "kwh_i", "kw_i", "kva_i", "on_peak")

    def __init__(self, epoch, local, utc, kwh_i, kw_i, kva_i, on_peak):
        self.epoch = epoch
        self.local = local
        self.utc = utc
        self.kwh_i = kwh_i
        self.kw_i = kw_i
        self.kva_i = kva_i
        self.on_peak = on_peak


def build_readings(series, holiday_dates):
    """Merge the three series into per-hour Reading records, ascending by time."""
    timestamps = sorted(series["kwh"])
    for name in ("kw", "kva"):
        if set(series[name]) != set(timestamps):
            print(f"warning: {name} series timestamps differ from kwh series")

    readings = []
    for epoch in timestamps:
        utc = dt.datetime.fromtimestamp(epoch, dt.timezone.utc)
        local = utc.astimezone(LOCAL_TZ)
        readings.append(Reading(
            epoch=epoch,
            local=local.replace(tzinfo=None),
            utc=utc.replace(tzinfo=None),
            kwh_i=series["kwh"][epoch],
            kw_i=series["kw"].get(epoch, 0),
            kva_i=series["kva"].get(epoch, 0),
            on_peak=not is_off_peak(local, holiday_dates),
        ))
    return readings


def first_max(readings, attr):
    """First (earliest) reading attaining the max of integer attribute ``attr``.

    ``readings`` must be ascending in time. Returns None if empty; strict ``>`` keeps the
    earliest interval on ties.
    """
    best = None
    for r in readings:
        if best is None or getattr(r, attr) > getattr(best, attr):
            best = r
    return best


# ------------------------------------------------------------ per-period computation


def compute_period_values(readings, div):
    """Peak_values cell values for one billing period. ``div[name]`` converts a raw
    integer to kilo units; division happens only here, on floats."""
    vals = {c: None for c in PEAK_COLUMNS}
    vals["Nbr_of_intervals"] = len(readings)
    vals["kWH"] = sum(r.kwh_i for r in readings) / div["kwh"]

    on_peak = [r for r in readings if r.on_peak]
    specs = [
        ("kw_i", readings, "Max_kW", "Interval", "kva_i", "kva"),
        ("kw_i", on_peak, "Max_kW_nop", "interval", "kva_i", "kva"),
        ("kva_i", readings, "Max_kVA", "Interval", "kw_i", "kw"),
        ("kva_i", on_peak, "Max_kVA_nop", "interval", "kw_i", "kw"),
    ]
    peak_div = {"kw_i": "kw", "kva_i": "kva"}
    for attr, subset, prefix, ikey, comp_attr, comp_div in specs:
        r = first_max(subset, attr)
        if r is None:
            continue
        comp_suffix = "kVA" if attr == "kw_i" else "kW"
        vals[prefix] = getattr(r, attr) / div[peak_div[attr]]
        vals[f"{prefix}_{ikey}"] = r.local
        vals[f"{prefix}_{ikey}_utc"] = r.utc
        vals[f"{prefix}_{comp_suffix}"] = getattr(r, comp_attr) / div[comp_div]
    return vals


# --------------------------------------------------------------- XML cell formatting


def excel_serial(when):
    """Excel serial number for a naive datetime/date (1900 date system)."""
    if isinstance(when, dt.date) and not isinstance(when, dt.datetime):
        when = dt.datetime(when.year, when.month, when.day)
    return (when - EXCEL_EPOCH).total_seconds() / 86400.0


def num_text(x):
    """Shortest round-trip decimal text for a float, no scientific notation."""
    x = float(x)
    if x == int(x):
        return str(int(x))
    return repr(x)


# Peak_values column layout: (letter, machine name | None for spacer, style index, kind).
# ``kind``: date (serial, date fmt), dt (serial, datetime fmt), num, count, spacer.
# Column E carries no cell in the template's data rows and is intentionally omitted.
# ``kind`` selects the style (sampled at runtime) and value encoding.
PEAK_LAYOUT_TEMPLATE = [
    ("A", "Billing_period_ending", "date"),
    ("B", "Nbr_of_intervals", "count"),
    ("C", None, "spacer"),
    ("D", "kWH", "num"),
    ("F", "Max_kW", "num"),
    ("G", "Max_kW_Interval", "dt_local"),
    ("H", "Max_kW_Interval_utc", "dt_utc"),
    ("I", "Max_kW_kVA", "num"),
    ("J", None, "spacer"),
    ("K", "Max_kW_nop", "num"),
    ("L", "Max_kW_nop_interval", "dt_local"),
    ("M", "Max_kW_nop_interval_utc", "dt_utc"),
    ("N", "Max_kW_nop_kVA", "num"),
    ("O", None, "spacer"),
    ("P", "Max_kVA", "num"),
    ("Q", "Max_kVA_Interval", "dt_local"),
    ("R", "Max_kVA_Interval_utc", "dt_utc"),
    ("S", "Max_kVA_kW", "num"),
    ("T", None, "spacer"),
    ("U", "Max_kVA_nop", "num"),
    ("V", "Max_kVA_nop_interval", "dt_local"),
    ("W", "Max_kVA_nop_interval_utc", "dt_utc"),
    ("X", "Max_kVA_nop_kW", "num"),
]

# Fallback style indices for a brand-new template with no data row to sample from.
DEFAULT_STYLES = {"date": 31, "num": 4, "dt_local": 6, "dt_utc": 5, "spacer": 32}

ROW_ATTRS = ('customFormat="false" ht="12.8" hidden="false" customHeight="false" '
             'outlineLevel="0" collapsed="false"')

# Original Peak_values data columns (present in the template before this script ran).
PEAK_ORIGINAL_COLS = ["A", "D", "F", "G", "H", "K", "L", "M", "P", "Q", "R", "U", "V", "W"]

# Column letter -> kind, for sampling styles from an existing data row.
SAMPLE_COLS = {"A": "date", "D": "num", "G": "dt_local", "H": "dt_utc", "C": "spacer", "B": "count"}


def sample_styles(sheet_xml):
    """Read the style index for each column kind from the first existing data row (r >= 5).

    Style indices are *not* stable across saves by Excel/LibreOffice, so we always read the
    template's current indices rather than hard-coding them. Returns {} for a sheet with no
    data rows (a blank template), in which case the caller uses ``DEFAULT_STYLES``.
    """
    for row_m in re.finditer(r'<row r="(\d+)"[^>]*>(.*?)</row>', sheet_xml, re.S):
        if int(row_m.group(1)) < 5:
            continue
        cell_s = dict(re.findall(r'<c r="([A-Z]+)\d+" s="(\d+)"', row_m.group(2)))
        return {kind: int(cell_s[col]) for col, kind in SAMPLE_COLS.items() if col in cell_s}
    return {}


def cell_xml(col, row, style, kind, value):
    """One ``<c>`` element, or '' when the value is absent."""
    if kind == "spacer":
        return f'<c r="{col}{row}" s="{style}"/>'
    if value is None:
        return ""
    if kind == "count":
        text = str(int(value))
    elif kind in ("date", "dt_local", "dt_utc"):
        text = num_text(excel_serial(value))
    else:
        text = num_text(value)
    return f'<c r="{col}{row}" s="{style}" t="n"><v>{text}</v></c>'


def emit_peak_row(row, vals, styles, preserve=None):
    """Build a Peak_values ``<row>``. ``styles`` maps kind -> style index. ``preserve`` maps
    a column letter to its verbatim ``<v>`` text (or None if that cell was absent in the
    template) so already-finalized periods keep their exact stored values."""
    cells = []
    for col, name, kind in PEAK_LAYOUT_TEMPLATE:
        style = styles[kind]
        if preserve is not None and col in preserve:
            raw = preserve[col]
            cells.append("" if raw is None else f'<c r="{col}{row}" s="{style}" t="n"><v>{raw}</v></c>')
        else:
            cells.append(cell_xml(col, row, style, kind, vals.get(name)))
    return f'<row r="{row}" {ROW_ATTRS}>{"".join(cells)}</row>'


# ---------------------------------------------------------------- styles.xml editing


def resolve_count_style(styles_xml, num_index):
    """Return (styles_xml, style_index) for a ``#,##0`` data style matching the numeric
    data style's font. Reuses an existing matching xf if present (leaving styles.xml
    untouched); otherwise clones the numeric style with a ``#,##0`` format. Idempotent.

    Style/format indices are looked up by content, so this survives Excel/LibreOffice
    re-saves that renumber them.
    """
    m = re.search(r'<cellXfs count="(\d+)">(.*?)</cellXfs>', styles_xml, re.S)
    body = m.group(2)
    xfs = re.findall(r"<xf\b.*?(?:/>|</xf>)", body, re.S)

    # numFmtIds meaning '#,##0': any custom numFmt with that code, plus builtin id 3.
    hash0_ids = {nid for nid, code in
                 re.findall(r'<numFmt numFmtId="(\d+)" formatCode="([^"]*)"', styles_xml)
                 if code == "#,##0"} | {"3"}
    num_font = re.search(r'fontId="(\d+)"', xfs[num_index]).group(1)

    for i, xf in enumerate(xfs):
        nid = re.search(r'numFmtId="(\d+)"', xf).group(1)
        fid = re.search(r'fontId="(\d+)"', xf).group(1)
        if nid in hash0_ids and fid == num_font:
            return styles_xml, i

    target = next(iter(hash0_ids - {"3"}), "3")        # prefer a defined #,##0, else builtin 3
    clone = re.sub(r'numFmtId="\d+"', f'numFmtId="{target}"', xfs[num_index], count=1)
    new_index = len(xfs)
    styles_xml = styles_xml.replace(
        f'<cellXfs count="{m.group(1)}">{body}</cellXfs>',
        f'<cellXfs count="{new_index + 1}">{body + clone}</cellXfs>',
    )
    return styles_xml, new_index


# ------------------------------------------------------------------ sheet XML editing


def parse_peak_rows(sheet_xml):
    """Existing Peak_values data rows -> {ending_date: {col_letter: <v> text or None}}.

    A column present in PEAK_ORIGINAL_COLS but absent from the row maps to None so it can
    be re-emitted as absent (matching e.g. a period that had no on-peak interval).
    """
    result = {}
    for row_m in re.finditer(r'<row r="(\d+)"[^>]*>(.*?)</row>', sheet_xml, re.S):
        rownum, body = int(row_m.group(1)), row_m.group(2)
        if rownum < 5:
            continue
        vmap = {c: (v if v != "" else None)
                for c, v in re.findall(r'<c r="([A-Z]+)\d+"[^>]*?>(?:<v>(.*?)</v>)?', body)}
        a = vmap.get("A")
        if a is None:
            continue
        ending = (EXCEL_EPOCH + dt.timedelta(days=float(a))).date()
        result[ending] = {c: vmap.get(c) for c in PEAK_ORIGINAL_COLS}
    return result


def rebuild_peak_sheet(sheet_xml, period_vals, styles):
    """Regenerate Peak_values data rows (descending by period), preserving finalized
    periods' stored values and recomputing the most-recent existing period. ``styles`` maps
    kind -> style index (including 'count')."""
    existing = parse_peak_rows(sheet_xml)
    prev_most_recent = max(existing) if existing else None

    for end, vals in period_vals.items():
        vals["Billing_period_ending"] = dt.datetime(end.year, end.month, end.day)

    endings = sorted(set(period_vals) | set(existing), reverse=True)
    rows = []
    for i, end in enumerate(endings):
        row = 5 + i
        if end in existing and end != prev_most_recent and end in period_vals:
            # Finalized period: keep stored values verbatim, add the new columns only.
            rows.append(emit_peak_row(row, period_vals[end], styles, preserve=existing[end]))
        elif end in period_vals:
            # New period, or the previously in-progress one: full (re)computation.
            rows.append(emit_peak_row(row, period_vals[end], styles))
        else:
            # Period present in the sheet but no longer in the data: keep it as-is.
            rows.append(emit_peak_row(row, {}, styles, preserve=existing[end]))

    end_idx = sheet_xml.index("</sheetData>")
    start = sheet_xml.index('<row r="5"') if '<row r="5"' in sheet_xml else end_idx
    return sheet_xml[:start] + "".join(rows) + sheet_xml[end_idx:], len(endings)


def parse_interval_rows(sheet_xml):
    """Existing Interval_values data rows (r >= 4) -> list of (utc_serial, row_body)."""
    out = []
    for row_m in re.finditer(r'<row r="(\d+)"[^>]*>(.*?)</row>', sheet_xml, re.S):
        rownum, body = int(row_m.group(1)), row_m.group(2)
        if rownum < 4:
            continue
        b = re.search(r'<c r="B\d+"[^>]*?><v>(.*?)</v>', body)
        if b:
            out.append((float(b.group(1)), body))
    return out


def rebuild_interval_sheet(sheet_xml, readings, div, styles):
    """Fill Interval_values descending by Interval_utc. Existing rows are preserved; only
    intervals newer than the newest existing one are added. ``styles`` maps kind -> index
    (local datetime, UTC datetime, numeric) sampled from the workbook."""
    existing = parse_interval_rows(sheet_xml)
    existing_max = max((u for u, _ in existing), default=None)
    s_local, s_utc, s_num = styles["dt_local"], styles["dt_utc"], styles["num"]

    computed = sorted(readings, key=lambda r: r.epoch, reverse=True)
    new_bodies = []
    for r in computed:
        u = excel_serial(r.utc)
        if existing_max is not None and u <= existing_max:
            continue
        new_bodies.append(
            f'<c r="A{{r}}" s="{s_local}" t="n"><v>{num_text(excel_serial(r.local))}</v></c>'
            f'<c r="B{{r}}" s="{s_utc}" t="n"><v>{num_text(u)}</v></c>'
            f'<c r="C{{r}}" s="{s_num}" t="n"><v>{num_text(r.kwh_i / div["kwh"])}</v></c>'
            f'<c r="D{{r}}" s="{s_num}" t="n"><v>{num_text(r.kw_i / div["kw"])}</v></c>'
            f'<c r="E{{r}}" s="{s_num}" t="n"><v>{num_text(r.kva_i / div["kva"])}</v></c>'
        )

    # All bodies (new + preserved) ordered newest-first, then numbered from row 4.
    ordered_bodies = new_bodies + [body for _, body in sorted(existing, reverse=True)]
    rows = []
    for i, body in enumerate(ordered_bodies):
        r = 4 + i
        rows.append(f'<row r="{r}" {ROW_ATTRS}>{body.format(r=r)}</row>')

    # Replace the existing data-row span (r >= 4), not just append, so re-runs stay stable.
    end_idx = sheet_xml.index("</sheetData>")
    start = sheet_xml.index('<row r="4"') if '<row r="4"' in sheet_xml else end_idx
    sheet_xml = sheet_xml[:start] + "".join(rows) + sheet_xml[end_idx:]
    last_row = 3 + len(ordered_bodies)
    sheet_xml = re.sub(r'<dimension ref="A1:[A-Z]+\d+"/>',
                       f'<dimension ref="A1:AMJ{last_row}"/>', sheet_xml, count=1)
    return sheet_xml, len(ordered_bodies)


# ------------------------------------------------------------------ workbook plumbing


def sheet_paths(zf):
    """Map sheet display name -> zip member path via workbook.xml + its rels."""
    wb = zf.read("xl/workbook.xml").decode()
    rels = zf.read("xl/_rels/workbook.xml.rels").decode()
    rid_target = dict(re.findall(r'<Relationship Id="([^"]+)"[^>]*Target="([^"]+)"', rels))
    paths = {}
    for name, rid in re.findall(r'<sheet [^>]*name="([^"]+)"[^>]*r:id="([^"]+)"', wb):
        paths[name] = "xl/" + rid_target[rid].lstrip("/")
    return paths


def rewrite_workbook(path, edits):
    """Rewrite ``path`` (a zip), replacing the members named in ``edits`` and copying every
    other member byte-for-byte so all untouched formatting is preserved exactly."""
    tmp = path + ".tmp"
    with zipfile.ZipFile(path) as zin, zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zout:
        for item in zin.infolist():
            data = edits.get(item.filename)
            zout.writestr(item, data.encode("utf-8") if data is not None else zin.read(item.filename))
    os.replace(tmp, path)


def main():
    input_xml = sys.argv[1] if len(sys.argv) > 1 else "data/TH_Electric_Usage_23-11-2024_to_24-06-2026.XML"
    workbook_xlsx = sys.argv[2] if len(sys.argv) > 2 else "out/Green_Button_Peak_Values.xlsx"

    print(f"Reading {input_xml} ...")
    series, pot = parse_series(input_xml)
    div = {name: 10 ** (3 - p) for name, p in pot.items()}

    timestamps = sorted(series["kwh"])
    years = range(
        dt.datetime.fromtimestamp(timestamps[0], dt.timezone.utc).astimezone(LOCAL_TZ).year,
        dt.datetime.fromtimestamp(timestamps[-1], dt.timezone.utc).astimezone(LOCAL_TZ).year + 1,
    )
    holiday_dates = build_holiday_set(years)
    print("Holidays applied (off-peak):")
    for d in sorted(holiday_dates):
        print(f"  {d}  {holiday_dates[d]}")

    readings = build_readings(series, holiday_dates)
    print(f"  {len(readings)} hourly intervals")

    grouped = defaultdict(list)
    for r in readings:
        grouped[billing_period_ending(r.local.date())].append(r)
    period_vals = {end: compute_period_values(rs, div) for end, rs in grouped.items()}

    print(f"Editing workbook {workbook_xlsx} ...")
    with zipfile.ZipFile(workbook_xlsx) as zf:
        names = sheet_paths(zf)
        styles_xml = zf.read("xl/styles.xml").decode()
        peak_xml = zf.read(names["Peak_values"]).decode()
        interval_xml = zf.read(names["Interval_values"]).decode()

    # Sample the template's current style indices (robust to Excel/LibreOffice re-saves that
    # renumber them); fall back to defaults for a brand-new blank template.
    style_idx = {**DEFAULT_STYLES, **sample_styles(peak_xml)}
    if "count" not in style_idx:
        styles_xml, style_idx["count"] = resolve_count_style(styles_xml, style_idx["num"])

    peak_xml, n_periods = rebuild_peak_sheet(peak_xml, period_vals, style_idx)
    interval_xml, n_intervals = rebuild_interval_sheet(interval_xml, readings, div, style_idx)

    rewrite_workbook(workbook_xlsx, {
        "xl/styles.xml": styles_xml,
        names["Peak_values"]: peak_xml,
        names["Interval_values"]: interval_xml,
    })
    print(f"Peak_values: {n_periods} billing periods")
    print(f"Interval_values: {n_intervals} interval rows")
    print(f"\nSaved {workbook_xlsx}")


if __name__ == "__main__":
    main()
