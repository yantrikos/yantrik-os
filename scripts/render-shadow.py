#!/usr/bin/env python3
"""Render the kit's one soft shadow, crates/yantrik-ui-kit/slint/images/shadow-float.png.

A floating surface's shadow is this picture, stretched as a nine-slice behind the surface
(y_shadow.slint), and not a `drop-shadow-*` property: the software renderer, which is what the
shell and every app run on in a virtual machine, draws no drop shadows at all, so a surface that
relied on the property had a shadow on a laptop and none in the VM most people first see the OS in.
A picture is drawn the same by both renderers and costs a blit.

Two layers, as light gives: a wide soft one well below the surface (the room's light) and a tight
one just under its edge (the contact). White with alpha, so the component can ink it per theme.

Usage: python3 scripts/render-shadow.py    (needs Pillow; the PNG is committed beside this)
"""
from PIL import Image, ImageChops, ImageDraw, ImageFilter

# The surface the shadow is drawn for: 40px square, 16px corners (Theme.r-xl). The nine-slice
# stretches its 4px middle, so any surface 36px or more each way wears it.
SIDE, TOP, BOTTOM, CORE, RADIUS = 56, 40, 72, 40, 16
W, H = SIDE + CORE + SIDE, TOP + CORE + BOTTOM
SCALE = 2  # drawn at 2x and reduced, so the corner's edge is smooth

def layer(dy, blur, alpha):
    im = Image.new("L", (W * SCALE, H * SCALE), 0)
    box = [SIDE * SCALE, (TOP + dy) * SCALE, (SIDE + CORE) * SCALE, (TOP + CORE + dy) * SCALE]
    ImageDraw.Draw(im).rounded_rectangle(box, radius=RADIUS * SCALE, fill=255)
    im = im.filter(ImageFilter.GaussianBlur(blur * SCALE)).resize((W, H), Image.LANCZOS)
    return im.point(lambda v: int(v * alpha))

ambient = layer(dy=14, blur=18, alpha=0.62)
contact = layer(dy=3, blur=4, alpha=0.34)
# 1 - (1 - a)(1 - b): two shadows darken, they do not add past black.
both = ImageChops.invert(ImageChops.multiply(ImageChops.invert(ambient), ImageChops.invert(contact)))
out = Image.merge("LA", (Image.new("L", (W, H), 255), both))
out.save("crates/yantrik-ui-kit/slint/images/shadow-float.png", optimize=True)
print(W, H, "peak alpha", both.getextrema()[1])
