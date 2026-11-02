"""Finding things on a screenshot by colour, relative to the screen's own size (never absolute coordinates)."""


def _px(data: bytes, w: int, x: int, y: int):
    i = (y * w + x) * 3
    return data[i], data[i + 1], data[i + 2]


def is_light(p, floor=105):
    return p[0] >= floor and p[1] >= floor and p[2] >= floor


def find_lens_header_icons(w: int, h: int, data: bytes):
    """Centres of the glyphs in the Lens header (its +, history, export, close): the light marks in the band
    just under the top bar, in the right-hand 15% of the screen. Returned left to right.
    The first one is the + (New chat)."""
    x0, x1 = int(w * 0.86), w - 4
    y0, y1 = 60, min(h, 100)
    cols = [x for x in range(x0, x1) if any(is_light(_px(data, w, x, y)) for y in range(y0, y1))]
    groups, cur = [], []
    for x in cols:
        if cur and x - cur[-1] > 6:
            groups.append(cur)
            cur = []
        cur.append(x)
    if cur:
        groups.append(cur)
    icons = []
    for g in groups:
        xs = range(g[0], g[-1] + 1)
        rows = [y for y in range(y0, y1) if any(is_light(_px(data, w, x, y)) for x in xs)]
        if rows and 6 <= (g[-1] - g[0] + 1) <= 24:
            icons.append(((g[0] + g[-1]) // 2, (rows[0] + rows[-1]) // 2))
    return icons


def _button_pixel(p):
    """Near white (the Allow button) or red (the Run button)."""
    return (p[0] > 225 and p[1] > 225 and p[2] > 225) or (p[0] > 200 and p[1] < 110 and p[2] < 110)


def _row_span(w, data, y, x0, x1, min_run=20):
    """(left, right) of the solid light/red parts of row y between x0 and x1, counting only runs of at least
    `min_run` pixels so glyphs (the Decline label, the Allow label's edges) do not widen or split the span."""
    left = right = None
    run = 0
    for x in range(x0, min(x1, w - 1) + 2):
        if x <= min(x1, w - 1) and _button_pixel(_px(data, w, x, y)):
            run += 1
        else:
            if run >= min_run:
                if left is None:
                    left = x - run
                right = x - 1
            run = 0
    return left, right


def find_card_buttons(w: int, h: int, data: bytes):
    """(decline_x, y) of an approval card's Decline button, or None.

    A card is shown two ways (an overlay at the top right, or inline in the Lens panel), so the card is not
    assumed to be anywhere: the Allow/Run button is found, a wide solid light or red band (at least 12% of the
    screen wide and 18 px tall) in the right 40% of the screen with no text across its edge, and Decline is the
    button level with it, immediately to its left (about 98 px left of the Allow button's left edge at 1280 wide,
    scaled with the width). Returns the first such band from the top."""
    x0, x1 = int(w * 0.6), w - 1
    min_w = int(w * 0.12)
    band_top, band = None, []
    for y in range(40, min(h, 780)):
        left, right = _row_span(w, data, y, x0, x1)
        if left is not None and right - left >= min_w:
            if band_top is None:
                band_top = y
            band.append((right - left, left))
        else:
            if band_top is not None and len(band) >= 18:
                break
            band_top, band = None, []
    if band_top is None or len(band) < 18:
        return None
    left = min(b[1] for b in band)
    return left - int(w * 0.0766), band_top + len(band) // 2


def looks_like_allow(w: int, h: int, data: bytes, x: int, y: int) -> bool:
    """True when the spot (and a few pixels around it) is solid light or red: the Allow/Run button's fill. A Decline
    button is dark with a thin outline, so a click that would land on a solid light/red area is refused."""
    pts = [(x + dx, y + dy) for dx in (-30, 0, 30) for dy in (-6, 0, 6)]
    solid = sum(1 for px, py in pts if 0 <= px < w and 0 <= py < h and _button_pixel(_px(data, w, px, py)))
    return solid >= 7
