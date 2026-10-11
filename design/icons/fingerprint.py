"""A fingerprint of everything the icon set is generated from, written beside the output.

The PNGs are committed so the build needs no Python and no resvg, which means nothing at build
time notices when the inputs move on without them. This closes that: the kit's tests compute the
same fingerprint from the same inputs (src/app_icons.rs) and fail while it differs from the one
the generator last wrote. The inputs are every file under design/icons/ (the generator, the glyph
map, the vendored glyphs, the pinned requirements) and AppColor's table as `id:hue:tile:ink`
lines, so a comment edited in app_color.slint does not count and a hue changed there does.

FNV-1a, 64-bit: no library on either side, and a mismatch is all it has to show. Carriage
returns are dropped first, so a Windows checkout and a Linux one agree.
"""

from pathlib import Path

import palette

FNV_OFFSET = 0xCBF29CE484222325
FNV_PRIME = 0x100000001B3


def fnv1a64(data: bytes) -> int:
    h = FNV_OFFSET
    for b in data:
        h = ((h ^ b) * FNV_PRIME) & 0xFFFFFFFFFFFFFFFF
    return h


def inputs(icons_dir: Path, apps: list[palette.App]) -> bytes:
    """The bytes hashed: each input file as `path\\ncontent\\n`, in path order, then the table."""
    out = bytearray()
    files = {p.relative_to(icons_dir).as_posix(): p for p in icons_dir.rglob("*")
             if p.is_file() and "__pycache__" not in p.parts}
    for name in sorted(files):
        out += name.encode() + b"\n"
        out += files[name].read_bytes().replace(b"\r", b"") + b"\n"
    out += palette.table_lines(apps).encode()
    return bytes(out)


def text(icons_dir: Path, apps: list[palette.App]) -> str:
    return ("# What the app icons were generated from: design/icons/ and AppColor's table.\n"
            "# Written by design/icons/generate.py and checked by the kit's tests (src/app_icons.rs).\n"
            f"fnv1a64 {fnv1a64(inputs(icons_dir, apps)):016x}\n")
