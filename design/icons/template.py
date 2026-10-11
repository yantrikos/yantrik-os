"""The one icon template: every app tile, the neutral plate and the hover mask are drawn by it.

Layers, bottom to top, all inside a squircle (a superellipse, n = 5, edge to edge):
  * the background: the app's tile colour as a top-lit vertical gradient (palette.gradient);
  * a 1px inner highlight along the top edge, fading out by mid-height, and a fainter shade
    along the bottom edge: the light catching a rim, which is what gives the tile depth;
  * the glyph: a Phosphor "fill" glyph, white or the table's dark ink, with a very soft shadow.

The SVG is drawn in pixels for the size it is rendered at (viewBox 0 0 S S), so "1px" and the
shadow's offset are real pixels at every size, and the glyph can be placed on the pixel grid
(snap.py) at the small sizes where half a pixel shows.
"""

import math
import re

N = 5.0           # the superellipse exponent: rounder than a rounded rectangle, flatter sides
GLYPH = 0.62      # the glyph's live area (208 of Phosphor's 256 units) as a share of the tile
LIVE = 208 / 256  # Phosphor draws inside a 24-unit margin of its 256 grid


def squircle(size: int, steps: int = 96) -> str:
    """The superellipse |x|^n + |y|^n = r^n filling a size x size box, as a closed path."""
    r = size / 2
    points = []
    for i in range(4 * steps):
        t = 2 * math.pi * i / (4 * steps)
        c, s = math.cos(t), math.sin(t)
        x = r + r * math.copysign(abs(c) ** (2 / N), c)
        y = r + r * math.copysign(abs(s) ** (2 / N), s)
        points.append(f"{x:.2f},{y:.2f}")
    return "M" + "L".join(points) + "Z"


def glyph_box(size: int) -> int:
    """The side of the 256-unit glyph grid in pixels: even, so it centres on whole pixels."""
    return 2 * round(size * GLYPH / LIVE / 2)


def glyph_body(svg: str) -> str:
    """A vendored Phosphor SVG's drawing, without its <svg> wrapper."""
    inner = re.search(r"<svg[^>]*>(.*)</svg>", svg, re.S).group(1)
    return inner.strip()


def _num(v: float) -> str:
    return f"{v:.3f}".rstrip("0").rstrip(".")


def glyph_group(size: int, body: str, ink: str, dx: float = 0.0, dy: float = 0.0) -> str:
    """The glyph scaled to its box, centred in the tile and moved by (dx, dy) pixels."""
    box = glyph_box(size)
    at_x, at_y = (size - box) / 2 + dx, (size - box) / 2 + dy
    return (f'<g transform="translate({_num(at_x)} {_num(at_y)}) scale({_num(box / 256)})" '
            f'fill="{ink}">{body}</g>')


def _shadow(size: int, ink: str) -> str:
    """A very soft shadow under the glyph, a whole pixel down at the small sizes."""
    dy = max(1, round(size * 0.022))
    blur = max(0.6, size * 0.022)
    opacity = 0.30 if ink == "#ffffff" else 0.14
    return (f'<filter id="s" x="-25%" y="-25%" width="150%" height="150%" '
            f'color-interpolation-filters="sRGB"><feDropShadow dx="0" dy="{dy}" '
            f'stdDeviation="{_num(blur)}" flood-color="#000" flood-opacity="{opacity}"/></filter>')


def tile(size: int, top: str, bottom: str, glyph: str = "", ink: str = "#ffffff",
         edge: tuple[str, float] | None = None) -> str:
    """A whole tile at `size` px. `glyph` is a glyph_group(), or "" for a bare plate. `edge`,
    a (colour, opacity), draws a 1px rim all round: the neutral plates' outline, which is what
    keeps them from melting into the surface they sit on."""
    shape = squircle(size)
    rim = 2  # stroked on the outline and clipped to it, so 1px shows inside
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {size} {size}">',
        "<defs>",
        f'<path id="q" d="{shape}"/>',
        '<clipPath id="c"><use href="#q"/></clipPath>',
        f'<linearGradient id="bg" x1="0" y1="0" x2="0" y2="{size}" gradientUnits="userSpaceOnUse">'
        f'<stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient>',
        f'<linearGradient id="hi" x1="0" y1="0" x2="0" y2="{_num(size * 0.5)}" '
        f'gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff" stop-opacity="0.42"/>'
        f'<stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>',
        f'<linearGradient id="lo" x1="0" y1="{_num(size * 0.6)}" x2="0" y2="{size}" '
        f'gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#000" stop-opacity="0"/>'
        f'<stop offset="1" stop-color="#000" stop-opacity="0.16"/></linearGradient>',
        _shadow(size, ink) if glyph else "",
        "</defs>",
        '<g clip-path="url(#c)">',
        f'<rect width="{size}" height="{size}" fill="url(#bg)"/>',
        f'<use href="#q" fill="none" stroke="url(#hi)" stroke-width="{rim}"/>',
        f'<use href="#q" fill="none" stroke="url(#lo)" stroke-width="{rim}"/>',
        f'<use href="#q" fill="none" stroke="{edge[0]}" stroke-opacity="{edge[1]}" stroke-width="{rim}"/>'
        if edge else "",
        f'<g filter="url(#s)">{glyph}</g>' if glyph else "",
        "</g>",
        "</svg>",
    ]
    return "".join(parts) + "\n"


def mask(size: int) -> str:
    """The squircle alone, in white: laid over a tile at low opacity, it is the hover state."""
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
            f'viewBox="0 0 {size} {size}"><path d="{squircle(size)}" fill="#fff"/></svg>\n')
