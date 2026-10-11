"""Put a glyph on the pixel grid at the sizes where half a pixel shows.

At 32px a glyph is about 20px across, and Phosphor's 256-unit grid does not land on whole pixels
at that scale: an edge that falls between two pixels is drawn as two half-grey ones, and a glyph
made of such edges reads as soft. So at the small sizes the glyph is tried at every offset within
a third of a pixel, in eighths, and drawn where the fewest of its pixels are part-covered,
with moving off-centre counted against it.

The search is deterministic (a fixed order and a fixed tie-break), so re-running the generator
gives the same placement and the same bytes.
"""

import io

import resvg_py
from PIL import Image

import template

SNAPPED = (32, 48)  # the sizes this applies to: the dock's tile and the desktop's
STEPS = [i / 8 for i in range(-3, 4)]


def render(svg: str) -> bytes:
    return bytes(resvg_py.svg_to_bytes(svg_string=svg, skip_system_fonts=True))


def softness(png: bytes) -> float:
    """How much of the glyph is drawn part-covered: 0 when every edge sits on a pixel edge."""
    alpha = Image.open(io.BytesIO(png)).getchannel("A").tobytes()
    return sum(min(a, 255 - a) for a in alpha) / 255


def offset(size: int, body: str) -> tuple[float, float]:
    """The (dx, dy) in pixels that draws the glyph crispest at `size`; (0, 0) above SNAPPED."""
    if size not in SNAPPED:
        return 0.0, 0.0
    best = None
    for dy in STEPS:
        for dx in STEPS:
            group = template.glyph_group(size, body, "#ffffff", dx, dy)
            svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
                   f'viewBox="0 0 {size} {size}">{group}</svg>')
            # Off-centre costs as much as a pixel of blur per pixel moved, so the glyph moves
            # only for a real gain, never for a rounding error.
            score = (round(softness(render(svg)) + abs(dx) + abs(dy), 3), dy, dx)
            if best is None or score < best[0]:
                best = (score, (dx, dy))
    return best[1]
