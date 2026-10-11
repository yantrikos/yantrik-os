"""Generate the app icon set: one SVG per app, PNGs at every size, and the Slint mapping to them.

Every app `AppColor.hue-for-app` gives a colour gets a tile from the one template (template.py):
its hue as a top-lit gradient on a squircle, a 1px highlight, and its Phosphor glyph (glyph_map.py,
vendored in glyphs/) filled, at 62% of the tile. Written:

  crates/yantrik-ui-kit/assets/app-icons/<id>.svg            the tile, drawn at 256
  crates/yantrik-ui-kit/assets/app-icons/<size>/<id>.png     rendered by resvg at each size
  crates/yantrik-ui-kit/assets/app-icons/<size>/_plate-light.png, _plate-dark.png
                                                             the neutral plate (unknown apps)
  crates/yantrik-ui-kit/assets/app-icons/<size>/_mask.png    the squircle in white (hover)
  crates/yantrik-ui-kit/assets/app-icons/fingerprint.txt     what this run read (fingerprint.py)
  crates/yantrik-ui-kit/slint/app_icons.slint                AppIcons: id and size to image

The PNGs are committed, so building the shell needs neither Python nor resvg. Run this after
changing an app's colour, adding an app, or changing anything in design/icons/, from the
repository root, in a venv with requirements.txt installed:

  python design/icons/generate.py           write the set
  python design/icons/generate.py --check   exit 1, naming the files, if a fresh run would differ
"""

import argparse
import sys
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(HERE))

import fingerprint  # noqa: E402
import palette  # noqa: E402
import slint_map  # noqa: E402
import snap  # noqa: E402
import template  # noqa: E402
from glyph_map import GLYPHS  # noqa: E402

SIZES = (32, 48, 64, 96, 128, 256)
OUT = Path("crates/yantrik-ui-kit/assets/app-icons")
SLINT = Path("crates/yantrik-ui-kit/slint/app_icons.slint")
# The neutral plates, one per theme: (top, bottom), the 1px rim as (colour, opacity), and the ink
# of the initial on it. No hue, so an app we know nothing about claims no category. In the light
# theme it is a pale card a theme's own logo sits on; in the dark theme a pale tile flashed in
# the dock, so it is a dim graphite there with a lighter rim, quieter than any app of ours.
PLATES = {
    "light": (("#f4f5f8", "#d3d7de"), ("#000000", 0.14), "#4a5160"),
    "dark": (("#4b5561", "#39414b"), ("#5d6874", 1.0), "#ffffff"),
}


def glyph(name: str) -> str:
    return template.glyph_body((HERE / "glyphs" / f"{name}-fill.svg").read_text(encoding="utf-8"))


def build() -> dict[Path, bytes]:
    """Every output file, by its path from the repository root."""
    apps = palette.read_apps(ROOT)
    unmapped = [a.id for a in apps if a.id not in GLYPHS]
    if unmapped:
        sys.exit(f"no glyph in glyph_map.py for: {', '.join(unmapped)}")
    files: dict[Path, bytes] = {}
    for app in apps:
        body = glyph(GLYPHS[app.id])
        top, bottom = palette.gradient(app.tile)
        master = template.glyph_group(256, body, app.ink)
        files[OUT / f"{app.id}.svg"] = template.tile(256, top, bottom, master, app.ink).encode()
        for size in SIZES:
            dx, dy = snap.offset(size, body)
            group = template.glyph_group(size, body, app.ink, dx, dy)
            files[OUT / str(size) / f"{app.id}.png"] = snap.render(template.tile(size, top, bottom, group, app.ink))
    for size in SIZES:
        for theme, (colours, rim, _) in PLATES.items():
            files[OUT / str(size) / f"_plate-{theme}.png"] = snap.render(template.tile(size, *colours, edge=rim))
        files[OUT / str(size) / "_mask.png"] = snap.render(template.mask(size))
    files[OUT / "fingerprint.txt"] = fingerprint.text(HERE, apps).encode()
    inks = {theme: ink for theme, (_, _, ink) in PLATES.items()}
    files[SLINT] = slint_map.source([a.id for a in apps], SIZES, inks).encode()
    return files


def on_disk() -> set[Path]:
    """What the generator owns now: everything under the output folder, and the mapping."""
    found = {p.relative_to(ROOT) for p in (ROOT / OUT).rglob("*") if p.is_file()}
    return found | ({SLINT} if (ROOT / SLINT).exists() else set())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="compare with a fresh run, write nothing")
    check = parser.parse_args().check
    files = build()
    stale = sorted(on_disk() - set(files))
    changed = sorted(p for p, data in files.items()
                     if not (ROOT / p).exists() or (ROOT / p).read_bytes() != data)
    if check:
        for p in changed:
            print(f"differs: {p.as_posix()}")
        for p in stale:
            print(f"not generated: {p.as_posix()}")
        print(f"{len(files)} files checked, {len(changed) + len(stale)} out of date")
        return 1 if changed or stale else 0
    for p in stale:
        (ROOT / p).unlink()
    for p in changed:
        (ROOT / p).parent.mkdir(parents=True, exist_ok=True)
        (ROOT / p).write_bytes(files[p])
    print(f"{len(files)} files, {len(changed)} written, {len(stale)} removed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
