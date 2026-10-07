#!/usr/bin/env python3
"""
brand/build.py — draws every Yantrik SVG from one geometry.

    python3 build.py          # writes the six SVGs below, beside this file

The emblem: a gold Y over a gear, inside a yantra. Every number is here, nothing is traced, and
the outputs are generated — edit this file and re-run, never the SVGs.

  yantrik-emblem.svg          the full emblem (512 field): gates, ring, gear, rim, lattice, the Y,
                              its channels and the bindu. Above 64 px.
  yantrik-mark.svg            the core: the Y, its channels, the bindu. 64 px and below, favicons.
  yantrik-icon.svg            the core on a rounded navy tile: the app icon (hicolor 'yantrik').
  yantrik-name.svg            "YANTRIK OS" alone, outlined.
  yantrik-wordmark.svg        the horizontal lockup: emblem + name.
  yantrik-lockup-stacked.svg  the stacked lockup: emblem over name.

Design notes, and why each part is the way it is, are in README.md.
Needs fonttools and uharfbuzz (pip install fonttools uharfbuzz).
"""
from __future__ import annotations

import math
from pathlib import Path

HERE = Path(__file__).resolve().parent
NAME_FONT = HERE / "fonts" / "Marcellus-Regular.ttf"

C = 256.0                   # the field is 512 x 512; the Y's junction is its centre
LIGHT = (-0.6, -0.8)        # direction to the light: upper left
DARK_EDGE = "#17202B"       # a dark edge under pale gold, so it holds on white


def P(x: float, y: float) -> str:
    return f"{x:.2f},{y:.2f}"


def poly(pts) -> str:
    return "M" + " L".join(P(*p) for p in pts) + "Z"


# ── The Y ───────────────────────────────────────────────────────────────────────────────
# Arms 42° off vertical, 44 wide, cut flat at y=126 with the outer tip flared 18 out and
# clipped 3; the stem ends at y=412 with 8-unit chamfers. 324 wide by 286 high.
ARM_DEG, HALF, TOP_Y, FLARE, STEM_END, FOOT, TIP_CLIP = 42, 22, 126, 18, 412, 8, 3


def y_silhouette():
    t = math.radians(ARM_DEG)
    d = (-math.sin(t), -math.cos(t))                # the left arm's direction, up and out
    n_out = (-math.cos(t), math.sin(t))             # its outer side

    def on_line(p0, y):
        k = (p0[1] - y) / math.cos(t)
        return (p0[0] + d[0] * k, y)

    def along(a, b, dist):
        ln = math.dist(a, b)
        return (a[0] + (b[0] - a[0]) * dist / ln, a[1] + (b[1] - a[1]) * dist / ln)

    inner0 = (C - n_out[0] * HALF, C - n_out[1] * HALF)
    outer0 = (C + n_out[0] * HALF, C + n_out[1] * HALF)
    crotch = (C, inner0[1] - (inner0[0] - C) / math.tan(t))
    inner_top, outer_top = on_line(inner0, TOP_Y), on_line(outer0, TOP_Y)
    tip = (outer_top[0] - FLARE, TOP_Y)
    outer_mid = on_line(outer0, TOP_Y + FLARE * 0.9)
    k_sh = (outer0[0] - (C - HALF)) / math.sin(t)
    shoulder = (C - HALF, outer0[1] - math.cos(t) * k_sh)
    left = [crotch, inner_top, along(tip, inner_top, TIP_CLIP), along(tip, outer_mid, TIP_CLIP), outer_mid, shoulder]
    foot = [(C - HALF, STEM_END - FOOT), (C - HALF + FOOT, STEM_END), (C + HALF - FOOT, STEM_END), (C + HALF, STEM_END - FOOT)]
    return left + foot + [(2 * C - x, y) for (x, y) in reversed(left[1:])]


def _area(pts):
    n = len(pts)
    return 0.5 * sum(pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1] for i in range(n))


def inset(pts, dist, limit=2.0):
    """Inward offset of a simple polygon: (face, per-edge (start, end) points, inward normals).
    A convex corner whose miter would run past limit*dist is cut square across its bisector."""
    n, ccw = len(pts), _area(pts) > 0
    normals = []
    for i in range(n):
        (x0, y0), (x1, y1) = pts[i], pts[(i + 1) % n]
        ln = math.hypot(x1 - x0, y1 - y0)
        dx, dy = (x1 - x0) / ln, (y1 - y0) / ln
        normals.append((-dy, dx) if ccw else (dy, -dx))

    def line(i):
        (x0, y0), (x1, y1) = pts[i], pts[(i + 1) % n]
        return (x0 + normals[i][0] * dist, y0 + normals[i][1] * dist), (x1 - x0, y1 - y0)

    def meet(a, b):
        (p, r), (q, s) = a, b
        den = r[0] * s[1] - r[1] * s[0]
        if abs(den) < 1e-9:
            return None
        t = ((q[0] - p[0]) * s[1] - (q[1] - p[1]) * s[0]) / den
        return (p[0] + r[0] * t, p[1] + r[1] * t)

    starts, ends = [None] * n, [None] * n
    for j in range(n):
        prev = (j - 1) % n
        m, v = meet(line(prev), line(j)), pts[j]
        if m is not None and math.dist(m, v) <= limit * dist:
            ends[prev] = starts[j] = m
            continue
        bx, by = normals[prev][0] + normals[j][0], normals[prev][1] + normals[j][1]
        bl = math.hypot(bx, by) or 1.0
        b = (bx / bl, by / bl)
        cut = ((v[0] + b[0] * limit * dist, v[1] + b[1] * limit * dist), (-b[1], b[0]))
        ends[prev], starts[j] = meet(line(prev), cut) or m, meet(line(j), cut) or m
    face = []
    for i in range(n):
        if not face or math.dist(face[-1], starts[i]) > 1e-6:
            face.append(starts[i])
        face.append(ends[i])
    return face, list(zip(starts, ends)), normals


# ── Paint ───────────────────────────────────────────────────────────────────────────────
def _stops(*s):
    return "".join(f'<stop offset="{o}" stop-color="{c}"' + (f' stop-opacity="{a[0]}"' if a else "") + "/>" for o, c, *a in s)


DEFS = (
    f'<radialGradient id="found" gradientUnits="userSpaceOnUse" cx="244" cy="238" r="176">{_stops((0, "#101C2B"), (0.7, "#080E18"), (1, "#050910"))}</radialGradient>'
    # The Y's face: a narrow reflected band at 36-46% is what reads as metal rather than paint.
    f'<linearGradient id="face" gradientUnits="userSpaceOnUse" x1="104" y1="126" x2="368" y2="412">'
    f'{_stops((0, "#BC8439"), (0.18, "#F0C674"), (0.36, "#FFE9A8"), (0.41, "#FFF5D2"), (0.46, "#D59A42"), (0.72, "#EABB66"), (1, "#A96E28"))}</linearGradient>'
    f'<linearGradient id="gearfill" gradientUnits="userSpaceOnUse" x1="0" y1="80" x2="0" y2="432">{_stops((0, "#80643A"), (0.55, "#594328"), (1, "#35281B"))}</linearGradient>'
    f'<radialGradient id="well" gradientUnits="userSpaceOnUse" cx="247" cy="247" r="40">{_stops((0, "#165BA4"), (0.65, "#082449"), (1, "#030B18"))}</radialGradient>'
    f'<radialGradient id="pearl" gradientUnits="userSpaceOnUse" cx="251.5" cy="250.5" r="18">{_stops((0, "#FFFAE0"), (0.24, "#FFE7A0"), (0.65, "#EDB652"), (1, "#B67523"))}</radialGradient>'
    f'<radialGradient id="halo" gradientUnits="userSpaceOnUse" cx="256" cy="256" r="43">{_stops((0, "#218EFF", 0.40), (0.55, "#1579E8", 0.16), (1, "#1579E8", 0))}</radialGradient>'
    '<filter id="shadow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="2"/></filter>'
    '<filter id="bloom" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="2.2"/></filter>'
)

# Three channels cut into the bars, converging on the bindu.
CHANNELS = ("M164.16 154 L256 256", "M347.84 154 L256 256", "M256 256 L256 392")


def channels(full: bool) -> str:
    def layer(w, col, op):
        return "".join(f'<path d="{d}" stroke="{col}" stroke-opacity="{op}" stroke-width="{w}" stroke-linecap="round" fill="none"/>' for d in CHANNELS)
    bloom = f'<g clip-path="url(#yclip)" filter="url(#bloom)">{layer(5, "#168CFF", 0.28)}</g>' if full else ""
    return bloom + layer(10, "#03101D", 1) + layer(4.2, "#168CFF", 1) + layer(1.2, "#A0E6FF", 0.9)


def bindu(full: bool) -> str:
    """One blue well holding one gold pearl: the only bright point in the mark."""
    halo = '<circle cx="256" cy="256" r="43" fill="url(#halo)"/>' if full else ""
    return halo + ('<circle cx="256" cy="256" r="27" fill="url(#well)" stroke="#63C7FF" stroke-opacity="0.9" stroke-width="1.5"/>'
                   '<circle cx="256" cy="256" r="13.5" fill="url(#pearl)"/>')


def the_y(full: bool) -> str:
    sil = y_silhouette()
    sd = poly(sil)
    if not full:
        return f'<path d="{sd}" fill="#E4B368" stroke="{DARK_EDGE}" stroke-width="2" paint-order="stroke"/>'
    face, edges, normals = inset(sil, 5.5)
    out = (f'<path d="{sd}" transform="translate(0 4)" fill="#000" fill-opacity="0.4" filter="url(#shadow)"/>'
           f'<path d="{sd}" transform="translate(0 3)" fill="#382516"/>'
           f'<path d="{sd}" fill="#C18D43" stroke="{DARK_EDGE}" stroke-width="4" paint-order="stroke"/>')
    lit = ""
    for i, (p0, p1) in enumerate(zip(sil, sil[1:] + sil[:1])):
        q0, q1 = edges[i]
        dot = -normals[i][0] * LIGHT[0] - normals[i][1] * LIGHT[1]       # outward normal . light
        col = "#FFF0B7" if dot >= 0.35 else "#75491D" if dot <= -0.35 else "#C18D43"
        out += f'<path d="{poly([p0, p1, q1, q0])}" fill="{col}"/>'
        if dot >= 0.35:
            lit += f'<path d="M{P(*p0)} L{P(*p1)}" stroke="#FFF0B7" stroke-opacity="0.65" stroke-width="0.8" stroke-linecap="round"/>'
    return out + f'<path d="{poly(face)}" fill="url(#face)"/>' + lit


def gear() -> str:
    """Sixteen shallow teeth (164 to 176) with 2-unit chamfers, phased so a gap sits at the top."""
    prof = [(-11.25, 164), (-6.5, 164), (-4.5, 174), (-3.5, 176), (3.5, 176), (4.5, 174), (6.5, 164), (11.25, 164)]
    pts = [(C + r * math.cos(math.radians(-78.75 + 22.5 * k + off)), C + r * math.sin(math.radians(-78.75 + 22.5 * k + off)))
           for k in range(16) for off, r in prof]
    ring = poly(pts) + " M406 256 A150 150 0 1 0 106 256 A150 150 0 1 0 406 256Z"
    hl = ""
    for p0, p1 in zip(pts, pts[1:] + pts[:1]):
        mx, my = (p0[0] + p1[0]) / 2 - C, (p0[1] + p1[1]) / 2 - C
        ln = math.dist(p0, p1) or 1
        nx, ny = (p1[1] - p0[1]) / ln, -(p1[0] - p0[0]) / ln
        if nx * mx + ny * my < 0:
            nx, ny = -nx, -ny
        if nx * LIGHT[0] + ny * LIGHT[1] >= 0.35:
            hl += f"M{P(*p0)} L{P(*p1)} "
    return (f'<path d="{ring}" fill="none" stroke="{DARK_EDGE}" stroke-width="2"/>'
            f'<path d="{ring}" fill="url(#gearfill)" fill-rule="evenodd"/>'
            f'<path d="{hl}" stroke="#BD9658" stroke-opacity="0.45" stroke-width="1" fill="none"/>')


def lattice() -> str:
    """Four triangles; every apex lands on another triangle's base, and no two bases coincide."""
    tris = [[(256, 148), (168, 304), (344, 304)], [(256, 364), (168, 208), (344, 208)],
            [(256, 208), (208, 364), (304, 364)], [(256, 304), (208, 148), (304, 148)]]
    return '<g opacity="0.38" fill="none" stroke="#BD9658" stroke-width="1">' + "".join(f'<path d="{poly(t)}"/>' for t in tris) + "</g>"


def rim() -> str:
    a0, a1 = math.radians(195), math.radians(285)
    arc = f"M{P(C + 138 * math.cos(a0), C + 138 * math.sin(a0))} A138 138 0 0 1 {P(C + 138 * math.cos(a1), C + 138 * math.sin(a1))}"
    return ('<circle cx="256" cy="256" r="138" fill="none" stroke="#248FE8" stroke-opacity="0.38" stroke-width="1.6"/>'
            f'<path d="{arc}" fill="none" stroke="#78CFFF" stroke-opacity="0.65" stroke-width="0.9"/>')


GATE = "M204 38 H308 V50 H278 V70 H234 V50 H204 Z"     # the bhupura's T-gate, top; rotated for the rest
RAY = "M256 14 L257.5 31 L256 36 L254.5 31 Z"


def surround() -> str:
    arcs = ""
    for q in range(4):
        a0, a1 = math.radians(8 + 90 * q), math.radians(82 + 90 * q)
        arcs += f"M{P(C + 196 * math.cos(a0), C + 196 * math.sin(a0))} A196 196 0 0 1 {P(C + 196 * math.cos(a1), C + 196 * math.sin(a1))} "
    out = (f'<path d="{arcs}" fill="none" stroke="{DARK_EDGE}" stroke-width="3.75"/>'
           f'<path d="{arcs}" fill="none" stroke="#AA8145" stroke-opacity="0.9" stroke-width="1.5"/>')
    for deg in (0, 90, 180, 270):
        out += (f'<g transform="rotate({deg} 256 256)">'
                f'<path d="{GATE}" fill="none" stroke="{DARK_EDGE}" stroke-width="3.75"/>'
                f'<path d="{GATE}" fill="none" stroke="#B88C49" stroke-opacity="0.9" stroke-width="1.5"/>'
                '<circle cx="256" cy="60" r="3.5" fill="#C89D55"/>'
                f'<path d="{RAY}" fill="#B88C49" fill-opacity="0.75"/></g>')
    return out


def emblem() -> tuple[str, str]:
    """(extra defs, body) of the full emblem in the 512 field."""
    yclip = f'<clipPath id="yclip"><path d="{poly(y_silhouette())}"/></clipPath>'
    body = (surround() + '<circle cx="256" cy="256" r="151" fill="url(#found)"/>' + gear() + rim()
            + lattice() + the_y(True) + channels(True) + bindu(True))
    return yclip, body


def core() -> str:
    return the_y(False) + channels(False) + bindu(False)


# ── The name ────────────────────────────────────────────────────────────────────────────
# Marcellus (SIL OFL), shaped with HarfBuzz and outlined: a 64-unit cap height, 4 units of
# tracking after kerning, YANTRIK and OS set separately with 34 units of ink between K and O.
# The A loses its crossbar and stands over a 16 x 17 blue triangle.
CAP_UNITS, TRACK_UNITS, WORD_GAP = 64.0, 4.0, 34.0


class _Font:
    def __init__(self, path: Path):
        from fontTools.ttLib import TTFont
        from fontTools.pens.boundsPen import BoundsPen
        self.path = str(path)
        self.tt = TTFont(self.path)
        self.gs = self.tt.getGlyphSet()
        self.order = self.tt.getGlyphOrder()
        self.upm = self.tt["head"].unitsPerEm
        bp = BoundsPen(self.gs)
        self.gs["H"].draw(bp)
        self.cap = bp.bounds[3]

    def shape(self, text: str, track: float):
        import uharfbuzz as hb
        font = hb.Font(hb.Face(hb.Blob.from_file_path(self.path)))
        buf = hb.Buffer()
        buf.add_str(text)
        buf.guess_segment_properties()
        hb.shape(font, buf, {"kern": True})
        x, out = 0, []
        for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
            out.append((self.order[info.codepoint], x + pos.x_offset))
            x += pos.x_advance + track
        return out

    def ink(self, glyphs):
        from fontTools.pens.boundsPen import BoundsPen
        from fontTools.pens.transformPen import TransformPen
        bp = BoundsPen(self.gs)
        for name, gx in glyphs:
            self.gs[name].draw(TransformPen(bp, (1, 0, 0, 1, gx, 0)))
        return bp.bounds

    def a_counter(self):
        """The A's counter is the triangle above its crossbar. Its slanted sides, run down to
        the baseline, bound the region between the legs; masking that out removes the bar."""
        from fontTools.pens.recordingPen import RecordingPen
        rp = RecordingPen()
        self.gs["A"].draw(rp)
        contours, cur = [], []
        for op, args in rp.value:
            if op == "moveTo":
                cur = [args[0]]
            elif op in ("lineTo", "qCurveTo", "curveTo"):
                cur.append(args[-1])
            elif op in ("closePath", "endPath"):
                contours.append(cur)
        counter = min(contours, key=lambda c: max(p[0] for p in c) - min(p[0] for p in c))
        apex = max(counter, key=lambda p: p[1])
        rest = [p for p in counter if p != apex]
        bl, br = min(rest, key=lambda p: p[0]), max(rest, key=lambda p: p[0])

        def down(p, y):
            k = (apex[1] - y) / (apex[1] - p[1])
            return (apex[0] + (p[0] - apex[0]) * k, y)
        return apex, down(bl, -40), down(br, -40), down(bl, 0), down(br, 0)


def name(cx: float, base: float, fnt: _Font) -> tuple[str, str, tuple[float, float]]:
    """(defs, body, (left, right) ink) for YANTRIK OS centred on its ink at cx, baseline base."""
    from fontTools.pens.svgPathPen import SVGPathPen
    from fontTools.pens.transformPen import TransformPen
    s = CAP_UNITS / fnt.cap
    track = TRACK_UNITS / s
    words = [fnt.shape("YANTRIK", track), fnt.shape("OS", track)]
    inks = [fnt.ink(g) for g in words]
    widths = [(b[2] - b[0]) * s for b in inks]
    left = cx - (widths[0] + WORD_GAP + widths[1]) / 2
    starts = [left, left + widths[0] + WORD_GAP]
    defs, backs, fills, fills_tri = "", "", "", ""
    for glyphs, ink, x_ink in zip(words, inks, starts):
        x0 = x_ink - ink[0] * s
        for gname, gx in glyphs:
            pen = SVGPathPen(fnt.gs)
            fnt.gs[gname].draw(TransformPen(pen, (s, 0, 0, -s, x0 + gx * s, base)))
            d, m = pen.getCommands(), ""
            if gname == "A":
                apex, bl, br, bl0, br0 = fnt.a_counter()
                tf = lambda p: (x0 + (gx + p[0]) * s, base - p[1] * s)
                hole = " ".join(P(*tf(p)) for p in (apex, bl, br))
                defs += (f'<mask id="lambda" maskUnits="userSpaceOnUse" x="-4000" y="-4000" width="8000" height="8000">'
                         f'<rect x="-4000" y="-4000" width="8000" height="8000" fill="#fff"/><polygon points="{hole}" fill="#000"/></mask>')
                m = ' mask="url(#lambda)"'
                mid = (tf(bl0)[0] + tf(br0)[0]) / 2
                fills_tri = f'<polygon points="{P(mid - 8, base - 1)} {P(mid + 8, base - 1)} {P(mid, base - 18)}" fill="#168CFF"/>'
            backs += f'<path d="{d}" fill="none" stroke="#352917" stroke-width="2.7" stroke-linejoin="round"{m}/>'
            fills += f'<path d="{d}" fill="url(#wm)" stroke="url(#wm)" stroke-width="1.2" stroke-linejoin="round"{m}/>'
    defs += (f'<linearGradient id="wm" gradientUnits="userSpaceOnUse" x1="0" y1="{base - CAP_UNITS}" x2="0" y2="{base}">'
             f'{_stops((0, "#EDC97E"), (0.55, "#D6A04A"), (1, "#A96D28"))}</linearGradient>')
    return defs, backs + fills + fills_tri, (left, left + widths[0] + WORD_GAP + widths[1])


# ── Files ───────────────────────────────────────────────────────────────────────────────
HEADER = ("<!-- Generated by brand/build.py. Do not edit: change build.py and re-run it.\n"
          "     Text is outlines, not <text>, so this renders the same everywhere. -->")


def svg(body: str, view: str, w: float, h: float, defs: str = "") -> str:
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view}" width="{w:.0f}" height="{h:.0f}" role="img" aria-label="Yantrik OS">\n'
            f'<title>Yantrik OS</title>\n{HEADER}\n<defs>{DEFS}{defs}</defs>\n{body}\n</svg>\n')


def build() -> list[Path]:
    fnt = _Font(NAME_FONT)
    yclip, full = emblem()
    files = {}
    files["yantrik-emblem.svg"] = svg(full, "0 0 512 512", 512, 512, yclip)
    files["yantrik-mark.svg"] = svg(core(), "64 80 384 384", 384, 384)
    tile = '<rect x="64" y="80" width="384" height="384" rx="84" fill="#09111F"/>'
    files["yantrik-icon.svg"] = svg(tile + f'<g transform="translate(256 272) scale(0.8) translate(-256 -272)">{core()}</g>',
                                    "64 80 384 384", 384, 384)
    # The name alone, with 6 units of clear space round its ink.
    nd, nb, (l, r) = name(0, 70, fnt)
    files["yantrik-name.svg"] = svg(nb, f"{l - 6:.2f} 0 {r - l + 12:.2f} 80", r - l + 12, 80, nd)
    # Stacked: the emblem at native scale, the name's cap top at 536 and baseline at 600.
    sd, sb, _ = name(256, 600, fnt)
    files["yantrik-lockup-stacked.svg"] = svg(full + sb, "-24 0 560 640", 560, 640, yclip + sd)
    # Horizontal: the emblem at 0.42, the name at 0.8 on its centre line.
    k, ts = 0.42, 0.8
    hd, hb_, (hl, hr) = name(0, 0, fnt)
    tx = 512 * k + 30 - hl * ts
    width = 512 * k + 30 + (hr - hl) * ts + 16
    body = (f'<g transform="scale({k})">{full}</g>'
            f'<g transform="translate({tx:.2f} {256 * k + 26:.2f}) scale({ts})">{hb_}</g>')
    files["yantrik-wordmark.svg"] = svg(body, f"0 0 {width:.2f} {512 * k:.2f}", width, 512 * k, yclip + hd)
    out = []
    for fname, text in files.items():
        path = HERE / fname
        path.write_text(text, encoding="utf-8", newline="\n")
        out.append(path)
        print(f"  {fname}")
    return out


if __name__ == "__main__":
    build()
