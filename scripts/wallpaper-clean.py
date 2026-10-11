#!/usr/bin/env python3
"""Make a shipped wallpaper and its preview from a large source picture, without the grain.

    scripts/wallpaper-clean.py <source.png> <name>

Writes crates/yantrik-ui-slint/ui/wallpapers/<name>.jpg (2560x1600) and
crates/yantrik-ui-slint/ui/wallpapers/previews/<name>.png (320x200, which the Settings theme cards
and the lock screen's blur are made from).

Why: image models draw "film grain" and blotchy noise into smooth skies and water, and a JPEG
squeezed small makes it worse. The lake shipped that way and read as grainy on every screen. An
edge-preserving guided filter (He et al.) smooths the flat areas and leaves the ridges sharp; a
+-1 LSB triangular dither then keeps the long gradients from banding without adding visible grain.

Needs Pillow, NumPy and SciPy.
"""
import os
import sys

import numpy as np
from PIL import Image
from scipy.ndimage import uniform_filter

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
WALLPAPERS = os.path.join(ROOT, "crates", "yantrik-ui-slint", "ui", "wallpapers")
SIZE = (2560, 1600)
PREVIEW = (320, 200)
# Radius in source pixels, and how flat a patch must be to be smoothed: tuned on the lake at
# 3840x2160, where 12 px and this eps removed the blotches and kept the snow's texture.
RADIUS = 12
EPS = 0.01 ** 2


def box(x, r):
    return uniform_filter(x, size=2 * r + 1, mode="reflect")


def guided(guide, p, r, eps):
    mean_i, mean_p = box(guide, r), box(p, r)
    a = (box(guide * p, r) - mean_i * mean_p) / (box(guide * guide, r) - mean_i * mean_i + eps)
    b = mean_p - a * mean_i
    return box(a, r) * guide + box(b, r)


def centre_crop(pic, aspect):
    h, w, _ = pic.shape
    if w / h > aspect:
        cw = int(round(h * aspect))
        x0 = (w - cw) // 2
        return pic[:, x0:x0 + cw]
    ch = int(round(w / aspect))
    y0 = (h - ch) // 2
    return pic[y0:y0 + ch]


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    source, name = sys.argv[1], sys.argv[2]
    pic = np.asarray(Image.open(source).convert("RGB"), dtype=np.float64) / 255
    pic = centre_crop(pic, SIZE[0] / SIZE[1])
    luma = pic @ np.array([0.299, 0.587, 0.114])
    clean = np.stack([guided(luma, pic[..., c], RADIUS, EPS) for c in range(3)], -1)
    full = Image.fromarray(np.clip(clean * 255, 0, 255).astype(np.uint8)).resize(SIZE, Image.LANCZOS)

    dithered = np.asarray(full, dtype=np.float64)
    dithered += np.random.default_rng(7).triangular(-1, 0, 1, dithered.shape)
    out = Image.fromarray(np.clip(np.rint(dithered), 0, 255).astype(np.uint8))
    out.save(os.path.join(WALLPAPERS, f"{name}.jpg"), quality=95, subsampling=0, optimize=True, progressive=True)
    full.resize(PREVIEW, Image.LANCZOS).save(os.path.join(WALLPAPERS, "previews", f"{name}.png"), optimize=True)
    print(f"wrote {name}.jpg {SIZE[0]}x{SIZE[1]} and previews/{name}.png {PREVIEW[0]}x{PREVIEW[1]}")


if __name__ == "__main__":
    main()
