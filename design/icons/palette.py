"""The app table, read from the one place it lives, and the tile gradient derived from it.

crates/yantrik-ui-kit/slint/app_color.slint gives every app a hue name (`hue-for-app`), every hue
a solid tile colour (`tile-*`), and says which hues carry dark ink instead of white (`on-tile`).
Nothing here restates any of that: an app added there, or a hue changed, reaches the icons the
next time the generator runs, and the kit's tests say when it has not.

The gradient keeps the hue exactly and moves only lightness, so a tile stays the colour the
table gave it: Browser the only teal, Notes the only amber (colour roles).
"""

import colorsys
import re
from dataclasses import dataclass
from pathlib import Path

APP_COLOR = Path("crates/yantrik-ui-kit/slint/app_color.slint")


@dataclass(frozen=True)
class App:
    id: str
    hue: str
    tile: str  # "#rrggbb", the solid tile colour the table gives the hue
    ink: str   # the glyph colour on that tile: white, or the table's dark ink on a light hue


def function_body(src: str, name: str) -> str:
    """The text of `public pure function <name>(...)`, up to its closing brace."""
    start = src.index(f"public pure function {name}(")
    return src[start:src.index("\n    }", start)]


def read_apps(root: Path) -> list[App]:
    """Every app `hue-for-app` names, in the table's order."""
    src = (root / APP_COLOR).read_text(encoding="utf-8")
    pairs = re.findall(r'id == "([^"]+)"\s*\?\s*"([^"]+)"', function_body(src, "hue-for-app"))
    tiles = dict(re.findall(r"out property <color> tile-([a-z]+):\s*(#[0-9a-fA-F]{6});", src))
    ink = tiles.pop("ink").lower()
    on_tile = function_body(src, "on-tile")
    light = set(re.findall(r'name == "([a-z]+)"', on_tile[:on_tile.index("? root.tile-ink")]))
    return [App(i, h, tiles[h].lower(), ink if h in light else "#ffffff") for i, h in pairs]


def table_lines(apps: list[App]) -> str:
    """The table as the fingerprint reads it: `id:hue:tile:ink`, one app a line."""
    return "".join(f"{a.id}:{a.hue}:{a.tile}:{a.ink}\n" for a in apps)


def _rgb(hex_: str) -> tuple[float, float, float]:
    return tuple(int(hex_[i:i + 2], 16) / 255 for i in (1, 3, 5))


def _hex(rgb) -> str:
    return "#" + "".join(f"{round(max(0.0, min(1.0, c)) * 255):02x}" for c in rgb)


def gradient(tile: str) -> tuple[str, str]:
    """(top, bottom): lit from above, deeper below, the same hue throughout."""
    h, l, s = colorsys.rgb_to_hls(*_rgb(tile))
    top = colorsys.hls_to_rgb(h, l + (1 - l) * 0.24, s)
    bottom = colorsys.hls_to_rgb(h, l * 0.80, min(1.0, s * 1.04))
    return _hex(top), _hex(bottom)
