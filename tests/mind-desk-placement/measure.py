#!/usr/bin/env python3
"""Where the Mind desk's bars are in a screenshot of the nested display.

    measure.py SHOT.png BAR_ROWS [--background R,G,B]

The nested compositor draws nothing of its own, so a pixel that is not the background belongs to a
window, and the only window is the desk's bar. The bar is the run of rows that are mostly not the
background. Prints one line per run, `rows FROM TO width MIN_X MAX_X`, and exits 1 unless there is
exactly one: from row 0, for BAR_ROWS rows, as wide as the picture.

Stdlib only: a small PNG reader (8-bit RGB or RGBA, not interlaced), which is what grim writes.
"""

import struct
import sys
import zlib


def read_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos, idat, header = 8, b"", None
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos : pos + 8])
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    width, height, depth, colour, _, _, interlace = header
    assert depth == 8 and colour in (2, 6) and interlace == 0, f"unsupported PNG {header}"
    step = 3 if colour == 2 else 4
    raw = zlib.decompress(idat)
    stride = width * step
    rows, previous = [], bytearray(stride)
    for y in range(height):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1 : (y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - step] if i >= step else 0
            b = previous[i]
            c = previous[i - step] if i >= step else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        previous = line
    return width, height, step, rows


def bands(path, background):
    width, height, step, rows = read_png(path)
    bg = bytes(background)
    found, start, lo, hi = [], None, width, -1
    for y in range(height + 1):
        if y < height:
            line = rows[y]
            xs = [x for x in range(width) if bytes(line[x * step : x * step + 3]) != bg]
        else:
            xs = []
        if len(xs) * 2 >= width:
            if start is None:
                start, lo, hi = y, width, -1
            lo, hi = min(lo, xs[0]), max(hi, xs[-1])
        elif start is not None:
            found.append((start, y - 1, hi - lo + 1, lo, hi))
            start = None
    return width, height, found


def main():
    path, rows = sys.argv[1], int(sys.argv[2])
    background = (0, 0, 0)
    if "--background" in sys.argv:
        background = tuple(int(v) for v in sys.argv[sys.argv.index("--background") + 1].split(","))
    width, height, found = bands(path, background)
    print(f"{path}: {width}x{height}")
    for first, last, span, lo, hi in found:
        print(f"rows {first}..{last} width {span} x {lo}..{hi}")
    got = [(f[0], f[1]) for f in found]
    wide = all(f[2] == width and f[3] == 0 for f in found)
    if got != [(0, rows - 1)] or not wide:
        print(f"FAIL: wanted the bar at rows 0..{rows - 1}, {width} wide; found {got}, full width: {wide}")
        sys.exit(1)
    print(f"PASS: the bar is at rows 0..{rows - 1} and {width} wide")


if __name__ == "__main__":
    main()
