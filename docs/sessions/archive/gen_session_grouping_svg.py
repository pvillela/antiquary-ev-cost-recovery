"""Generates docs/session-grouping.svg from the same figures the integration test asserts."""

# minutes from 16:00; session end is the *reported* end, padded by 60s when drawn — sessions are
# half-open, so the padded end is the first instant the session is no longer running
SESSIONS = [
    # id, start_min, end_min, kW   (start/end relative to 16:00, may fall outside 0..60)
    ("A", -6, 63, 6.0),
    ("B", -1, 15, 6.4),
    ("C", 8, 42, 6.2),
    ("E", 20, 34, 5.9),
    ("D", 24, 34, 6.6),
    ("F", 34, 42, 6.7),
    ("G", 48, 55, 6.1),
]
# group boundaries in seconds from 16:00, and the count in each
GROUPS = [
    (0, 480, 2), (480, 960, 3), (960, 1200, 2), (1200, 1440, 3), (1440, 2040, 4),
    (2040, 2100, 5), (2100, 2580, 3), (2580, 2880, 1), (2880, 3360, 2), (3360, 3600, 1),
]

# ---- layout -----------------------------------------------------------------
W, LM, RM = 940, 74, 26
PLOT = W - LM - RM
ROW_H, BAR_H = 26, 13
TOP = 46
AXIS_Y = TOP + len(SESSIONS) * ROW_H + 10
TILE_Y = AXIS_Y + 30
TILE_H = 26
ZOOM_TOP = TILE_Y + TILE_H + 74
ZOOM_H = 30
H = ZOOM_TOP + ZOOM_H + 62

INK = "#8b949e"        # readable on both light and dark
TEXT = "#6e7781"   # mid-tone: legible on both light and dark backgrounds
BAR = "#4c8eda"
TILE = "#5aa469"
SLIVER = "#d97757"
GRID = "#c9d1d9"


def x(sec):
    """seconds from 16:00 -> px"""
    return LM + PLOT * sec / 3600.0


def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;")


out = []
a = out.append
a(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}" '
  f'font-family="ui-sans-serif,-apple-system,Segoe UI,Helvetica,Arial,sans-serif" font-size="12">')
a('<title>Session grouping over a one-hour interval of interest</title>')
a(f'<text x="{LM}" y="20" font-size="13" font-weight="600" fill="{TEXT}">'
  f'Sessions and the groups they induce &#8212; 2026-06-15, interval of interest 16:00&#8211;17:00</text>')
a(f'<text x="{LM}" y="36" fill="{TEXT}">Bars show each session clipped to the interval; '
  f'the strip below is the tiling, labelled with concurrent session count.</text>')

# interval shading + quarter-hour grid
a(f'<rect x="{x(0):.1f}" y="{TOP - 8:.1f}" width="{x(3600) - x(0):.1f}" '
  f'height="{TILE_Y + TILE_H - TOP + 8:.1f}" fill="{GRID}" opacity="0.13"/>')
for m in range(0, 61, 15):
    px = x(m * 60)
    a(f'<line x1="{px:.1f}" y1="{TOP - 8:.1f}" x2="{px:.1f}" y2="{AXIS_Y:.1f}" '
      f'stroke="{GRID}" stroke-width="1" opacity="0.55"/>')
    a(f'<text x="{px:.1f}" y="{AXIS_Y + 15:.1f}" text-anchor="middle" fill="{TEXT}">'
      f'16:{m:02d}</text>' if m < 60 else
      f'<text x="{px:.1f}" y="{AXIS_Y + 15:.1f}" text-anchor="middle" fill="{TEXT}">17:00</text>')

# session bars
for i, (sid, s_min, e_min, kw) in enumerate(SESSIONS):
    y = TOP + i * ROW_H
    s_sec, e_sec = s_min * 60, e_min * 60 + 60      # padded end (exclusive)
    cs, ce = max(s_sec, 0), min(e_sec, 3600)
    a(f'<text x="{LM - 12}" y="{y + BAR_H - 2}" text-anchor="end" font-weight="600" '
      f'fill="{TEXT}">{sid}</text>')
    a(f'<text x="{LM - 30}" y="{y + BAR_H - 2}" text-anchor="end" fill="{TEXT}" '
      f'font-size="10">{kw:.1f} kW</text>')
    a(f'<rect x="{cs and x(cs) or x(0):.1f}" y="{y:.1f}" width="{x(ce) - x(cs):.1f}" '
      f'height="{BAR_H}" fill="{BAR}" rx="2.5"/>')
    # arrows where the session runs past the interval edge
    if s_sec < 0:
        a(f'<path d="M{x(0) - 3:.1f},{y:.1f} l-9,{BAR_H / 2:.1f} l9,{BAR_H / 2:.1f} z" '
          f'fill="{BAR}" opacity="0.55"/>')
    if e_sec > 3600:
        a(f'<path d="M{x(3600) + 3:.1f},{y:.1f} l9,{BAR_H / 2:.1f} l-9,{BAR_H / 2:.1f} z" '
          f'fill="{BAR}" opacity="0.55"/>')

a(f'<line x1="{x(0):.1f}" y1="{AXIS_Y:.1f}" x2="{x(3600):.1f}" y2="{AXIS_Y:.1f}" '
  f'stroke="{INK}" stroke-width="1.4"/>')

# tiling strip
for gi, (s, e, n) in enumerate(GROUPS):
    w = x(e) - x(s)
    fill = SLIVER if n == 5 else TILE
    a(f'<rect x="{x(s):.1f}" y="{TILE_Y}" width="{max(w, 1.0):.1f}" height="{TILE_H}" '
      f'fill="{fill}" opacity="0.82" stroke="#ffffff" stroke-width="1"/>')
    if w > 15:
        a(f'<text x="{x(s) + w / 2:.1f}" y="{TILE_Y + 18}" text-anchor="middle" '
          f'fill="#ffffff" font-weight="600">{n}</text>')
a(f'<text x="{LM - 12}" y="{TILE_Y + 18}" text-anchor="end" font-weight="600" '
  f'fill="{TEXT}">groups</text>')

# callout to the sliver
sx = x(2040) + (x(2100) - x(2040)) / 2
a(f'<line x1="{sx:.1f}" y1="{TILE_Y + TILE_H + 2:.1f}" x2="{sx:.1f}" y2="{ZOOM_TOP - 34:.1f}" '
  f'stroke="{SLIVER}" stroke-width="1.2" stroke-dasharray="3 2"/>')
a(f'<text x="{sx:.1f}" y="{ZOOM_TOP - 40:.1f}" text-anchor="middle" fill="{SLIVER}" '
  f'font-weight="600">5 sessions, 60 s</text>')

# ---- magnified detail -------------------------------------------------------
z0, z1 = 33 * 60, 36 * 60                              # detail window, seconds from 16:00
def zx(sec):
    return LM + PLOT * (sec - z0) / (z1 - z0)

a(f'<text x="{LM}" y="{ZOOM_TOP - 12}" font-weight="600" fill="{TEXT}">'
  f'Detail, 16:33&#8211;16:36 &#8212; D and E report ending on the minute F reports starting</text>')
for sec in range(z0, z1 + 1, 60):
    a(f'<line x1="{zx(sec):.1f}" y1="{ZOOM_TOP - 6:.1f}" x2="{zx(sec):.1f}" '
      f'y2="{ZOOM_TOP + ZOOM_H + 6:.1f}" stroke="{GRID}" stroke-width="1" opacity="0.6"/>')
    a(f'<text x="{zx(sec):.1f}" y="{ZOOM_TOP + ZOOM_H + 22:.1f}" text-anchor="middle" '
      f'fill="{TEXT}">16:{sec // 60:02d}</text>')
for s, e, n in GROUPS:
    if e <= z0 or s >= z1:
        continue
    cs, ce = max(s, z0), min(e, z1)
    fill = SLIVER if n == 5 else TILE
    a(f'<rect x="{zx(cs):.1f}" y="{ZOOM_TOP}" width="{zx(ce) - zx(cs):.1f}" height="{ZOOM_H}" '
      f'fill="{fill}" opacity="0.82" stroke="#ffffff" stroke-width="1"/>')
    a(f'<text x="{(zx(cs) + zx(ce)) / 2:.1f}" y="{ZOOM_TOP + 20}" text-anchor="middle" '
      f'fill="#ffffff" font-weight="600">{n}</text>')
a(f'<text x="{zx(2040 + 30):.1f}" y="{ZOOM_TOP - 22:.1f}" text-anchor="middle" fill="{SLIVER}" '
  f'font-size="11">16:34:00 &#8211; 16:35:00</text>')
a('</svg>')

open("docs/session-grouping.svg", "w").write("\n".join(x for x in out if x) + "\n")
print("wrote docs/session-grouping.svg")
