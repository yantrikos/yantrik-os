# Glyphs

The app tiles' glyphs are Phosphor Icons, "fill" weight, copied unchanged from the npm package
`@phosphor-icons/core` version 2.1.1 (`assets/fill/<name>-fill.svg`). Phosphor Icons is MIT
licensed; the licence is in `LICENSE` beside this file.

- Source: https://github.com/phosphor-icons/core
- Package: https://www.npmjs.com/package/@phosphor-icons/core/v/2.1.1

One glyph is not Phosphor's: `settings-gear-fill.svg` was drawn for this set, on the same 256
grid. Every Phosphor gear, in every weight, has round and shallow teeth, and at the dock's 32px
they merge and the gear reads as a flower. This one has eight flat-topped teeth (outer radius
108, root 84, each tooth 32% of its period at the tip and 44% at the root) and a 34-unit hole,
so each tooth is two or three whole pixels at 32px. It is covered by this repository's licence.

Only the glyphs `../glyph_map.py` names are here. To use another, copy its file from the same
package version into this folder, name it in `glyph_map.py`, and run `../generate.py`.
