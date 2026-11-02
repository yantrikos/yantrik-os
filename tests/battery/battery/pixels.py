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


def _longest_run(w, h, data, x, pred, y0=40, y1=None):
    best, run, start = (0, 0), 0, 0
    y1 = min(h, y1 or h)
    for y in range(y0, y1 + 1):
        if y < y1 and pred(_px(data, w, x, y)):
            if run == 0:
                start = y
            run += 1
        else:
            if run > best[0]:
                best = (run, start)
            run = 0
    return best


def _border(p):
    """The card's amber outline."""
    return p[0] > 200 and 150 < p[1] < 200 and 80 < p[2] < 130


def find_card_buttons(w: int, h: int, data: bytes):
    """(decline_x, y) of an approval card's Decline button, or None.

    The card sits at the top right with an amber outline. Its vertical extent is found from that outline (its
    right edge, 17 px in from the screen edge at 1280 wide); inside it, the Allow/Run button is the tallest
    solid light or red run in a column near the card's right edge, with no text on it; Decline sits level with
    it, about a quarter of the screen width to the left. Card heights differ, so the row is found, not assumed."""
    run, top = _longest_run(w, h, data, w - int(w * 0.0133), _border)
    if run < 60:
        return None
    n, start = _longest_run(w, h, data, w - int(w * 0.035), _button_pixel, y0=top, y1=top + run)
    if n < 18:
        return None
    return w - int(w * 0.25), start + n // 2


def looks_like_allow(w: int, h: int, data: bytes, x: int, y: int) -> bool:
    """True when the spot (and a few pixels around it) is solid light or red: the Allow/Run button's fill. A Decline
    button is dark with a thin outline, so a click that would land on a solid light/red area is refused."""
    pts = [(x + dx, y + dy) for dx in (-30, 0, 30) for dy in (-6, 0, 6)]
    solid = sum(1 for px, py in pts if 0 <= px < w and 0 <= py < h and _button_pixel(_px(data, w, px, py)))
    return solid >= 3
