# The YantrikDB mark: three concepts

Three concepts drawn on 1 Nov 2026 from the OS mark's own numbers (the Y, its 5.5-unit bevels,
its channels and its bindu are `brand/build.py`'s, unchanged). Pranab chose 2, Strata; it now
lives in `brand/build.py` as `yantrikdb-emblem.svg` and `yantrikdb-mark.svg`. The concept files
named below were the working set; `concepts-sheet.png` beside this file is the comparison:

| concept | full mark | 32 px | one line |
|---|---|---|---|
| 1 · Confluence | `ydb-concept-1.svg` | `ydb-concept-1-32.svg` | Three threads an arm join, one after another, into one channel before the bindu: many notes become one fact. |
| 2 · Strata | `ydb-concept-2.svg` | `ydb-concept-2-32.svg` | The Y over a disc of time layers, where the OS puts its gear: Y over mechanism, Y over memory. |
| 3 · Core | `ydb-concept-3.svg` | `ydb-concept-3-32.svg` | The bindu grown into a bezel-set well with strata rings round the pearl: the point is the memory. |

The sheet shows each beside the OS mark at 256 and at 64 / 32 / 16 px, on GitHub dark
(#0d1117) and on white.

The palette is the family's: gold face and bevels, navy `#03101D` recesses, channel blue
`#168CFF`, hot centre `#A0E6FF`, the well and the pearl as in the OS. No text, no fonts, no
cylinder, no stack, no brain.

## What the brief asked the mark to say

YantrikDB consolidates (many notes become one fact), notices contradictions, keeps history
(what did you believe in March?), and can say why it recalled something. One SQLite file, the
shared memory of every agent on the OS. Each concept takes one of those as its idea and leaves
the rest to the family resemblance, because a mark that says three things says none.

## 1 · Confluence

**The idea.** The Y is already a diagram of consolidation: two in, one out. Concept 1 makes
the channels say it. Each arm carries three threads: a centre channel and two parallel traces
at 8.5 units either side, which turn in at 30° (a machined bend, not a curve) and join the
centre at 52% and 74% of the way down. The two arms meet at the bindu, and the stem leaves it
as one heavier channel (13 wide against the arms' 10). Many notes, consolidated at the centre,
one fact out.

**Next to the OS mark.** Same silhouette, same metal, same bindu. The difference is inside the
bars: where the OS has one line of light per bar, the DB has traces merging into it. From
across the room they are the same object; up close the DB one is wired.

**Where it falls short.** At 32 px the traces are 0.75 px apart and fuse into a slightly
broader band at the top of each arm; at 16 px it is the OS mark. The idea lives above 64 px.
It also has no ground of its own, so as an avatar it needs the navy tile, like the OS icon.

## 2 · Strata

**The idea.** The OS emblem sets the Y over a gear: *yāntrika*, mechanism. The DB sets the
same Y over a disc of concentric strata: memory. Nine layers, each a shade apart with a gold
hairline on the step, thinning toward the rim (recent memory is dense; old memory has been
consolidated). One stratum is lit blue with a brighter arc upper left, as the emblem's rim is.
A machined gold band rims the disc, lit from the upper left like the Y's bevels. The bindu is
the spindle, the Y's tips overhang the rim exactly as they overhang the gear.

It is also the honest abstraction of the cylinder: a database seen from above is a platter,
and a platter of rings is a record, an archive, a tree's years.

**Next to the OS mark.** This is the clearest sibling relation of the three, because it is the
emblem's own grammar: an instrument (the Y) over what it stands on. Mechanism for the OS,
strata for the DB. Put the two emblems side by side and the sentence writes itself.

**Small sizes.** The 32 px version keeps the rim, one inner ring and the Y with channels at
16 units. At 32 px on white it is a navy coin with a gold rim and a gold Y; on GitHub dark the
disc recedes and it reads as a gold ring holding the Y. At 16 px it is still a coin with a Y in
it, which is more than the OS core manages at 16.

**Where it falls short.** It is the busiest of the three at 256, and the disc wants the Y's
full bevelled treatment to hold its own against the rings, so the flat-gold core version is
only for 64 px and below.

## 3 · Core

**The idea.** The bindu is the yantra's centre and, in the OS mark, the only bright point.
In YantrikDB the centre *is* the product, so the bindu grows: a 50-unit gold bezel set into the
junction, lit upper left, shaded lower right, holding a deep blue well. Inside the well, two
faint rings round the pearl: strata of time around the fact. The three channels still run in to
it. Point-in-time recall, drawn as a set jewel.

**Next to the OS mark.** The same Y with its heart enlarged; the silhouette changes (a disc at
the junction) so it is never mistaken for the OS mark, yet every part is inherited. It is the
quietest of the three and the most "jewellery".

**Small sizes.** The best of the three at 16 and 32 px: a gold Y with a blue eye reads at any
size, and the bezel is 9 px across at 32 px.

**Where it falls short.** A ringed blue disc in a gold bezel is also a lens, or an eye. The
rings are kept faint for that reason, but the association is there, and "the OS that watches
you" is not a sentence we want anyone to finish. It also has no ground of its own.

## The pick: 2 · Strata

Three reasons, in order.

1. **It says the right thing in the family's own grammar.** Y over gear, Y over strata.
   Nothing new is invented; the emblem's structure is reused with the one substitution that
   describes the product. That is what "obvious sibling, not a copy" means in practice.
2. **It carries its own ground.** Of the three, only Strata works without a tile: GitHub
   avatar, PyPI, crates.io, a favicon on white, a social card on dark. The OS has a navy tile
   for that job; the DB would otherwise need one too, and then the two icons are a Y on a tile
   and a Y on a tile.
3. **It survives 16 px with a distinct shape.** A coin with a Y in it is a different object
   from a bare Y, at a size where Confluence has collapsed into the OS mark and Core is a Y
   with a dot.

Second is Core, for its small-size strength; it would be the pick if the mark lived mostly in
a tab bar. Confluence is the most intellectually right and the least visible; its merging
traces would make a fine detail inside Strata's Y later, if the OS mark ever gains the same.

## What to do with it

Fold `concept2` (and `strata_disc`, `RIMLIGHT`) into `brand/build.py` as
`yantrikdb-emblem.svg` (512 field) and `yantrikdb-mark.svg` (the 32 px drawing), and add the
two to `render.py`'s ladder. The disc is `C`-centred like the gear, so the existing `inset`,
`the_y`, `channels` and `bindu` are reused as they are; nothing in this folder redraws them.
