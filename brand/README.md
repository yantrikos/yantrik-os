# The Yantrik OS mark

**One script draws the brand.** `build.py` holds every number of the emblem and the name and
writes the SVGs below; `render.py` turns them into every raster the project uses. Nothing
downstream redraws the mark: the OS shows these SVG files themselves (Slint renders them with
the resvg it ships), the app icon is rendered from them, and the cards and avatar embed them.
To change the mark, change `build.py` and run two commands.

| file | what it is | where it is used |
|---|---|---|
| `yantrik-emblem.svg` | the full emblem, 512 field | the OS above 64 px (boot, login), the GitHub avatar, wallpapers |
| `yantrik-mark.svg` | the core: the Y, its channels, the bindu | the OS at 64 px and below (status bar, About), favicons |
| `yantrik-icon.svg` | the core on a rounded navy tile | the app icon, hicolor `yantrik` |
| `yantrik-name.svg` | "YANTRIK OS" alone | boot, About and login, under or beside the mark |
| `yantrik-wordmark.svg` | horizontal lockup: emblem + name | website header, README banners, the OG card |
| `yantrik-lockup-stacked.svg` | stacked lockup: emblem over name | splash and print |
| `yantrikdb-emblem.svg` | YantrikDB: the same Y over a disc of strata, 512 field | YantrikDB above 64 px: its GitHub avatar, README, site |
| `yantrikdb-mark.svg` | YantrikDB at 64 px and below: disc, band, Y, bindu | YantrikDB favicons, tab bars, lists of minds |
| `yantrikdb-wordmark.svg` | horizontal lockup: DB emblem + "YANTRIK DB" | YantrikDB's social preview and README banner |

All nine are generated — do not edit them by hand.

## What the mark is

*Yantra* is *yam* ("to hold, to control") + *-tra* ("instrument"): an instrument of control.
*Yāntrika* means "mechanical". The OS is an instrument a person controls, and the mark is drawn
as one: a precise object, not an illustration.

- **The Y**, centred: its junction is the exact centre of the field. Arms 42° off vertical,
  cut flat at the top with the outer tip flared, a stem that ends in chamfers. Gold, machined:
  a broad face with a narrow reflected band, and 5.5-unit bevels lit from the upper left.
- **Three channels** cut dark into the bars, with blue light inside them, converging on
- **the bindu** — the yantra's centre point: one blue well holding one gold pearl. It is the
  only bright point in the mark.
- **The gear**: sixteen shallow teeth, darker than the Y and behind it — *yāntrika*.
- **The lattice**: four triangles in the yantra's own discipline — every apex lands on another
  triangle's base, and no two bases coincide. Quiet: it rewards a second look at boot size.
- **The bhupura**: the four T-gates every yantra stands in, with a stud and a ray at each
  cardinal point, and an outer ring broken at the gates.

The direction is Pranab's (a gold Y over a gear inside a yantra, October 2026). It was redrawn
as exact geometry and then put through an outside critique by GPT-6 (Astra), whose ten changes
— the Y's proportions, the metal, light from recesses, one bindu, quieter surroundings, a
correct lattice — are all in `build.py`.

**It is not the orb.** `components/orb.slint` is the companion's presence: it pulses when the
mind is thinking. A logo that animates is not a logo, it is a widget. The orb stays on the lock
and onboarding screens; it is never the product mark.

## The YantrikDB mark

YantrikDB is the memory under the OS: one local file that consolidates many notes into one
fact, notices contradictions, keeps every earlier belief, and is shared by every mind on the
machine. Its mark says that in the emblem's own grammar. The OS sets the Y over a gear —
mechanism. The DB sets the same Y over **strata** — memory.

- **The Y, its channels and its bindu** are the OS's, number for number. The tips overhang the
  band exactly as they overhang the gear, and the bindu is the spindle.
- **The strata**: nine layers in the disc, a shade apart, with a gold hairline on every step.
  They thin toward the rim: recent memory is dense, old memory has been consolidated. One
  stratum is lit blue, with a brighter arc upper left, as the emblem's rim is.
- **The band**: a machined gold ring at 176 — where the gear's teeth end — lit from the upper
  left like the Y's bevels, with a dark edge so it holds on white.

It is not a cylinder, a stack or a brain. Seen from above a database is a platter, and a platter
of rings is a record; that is as literal as it gets. Because it carries its own ground it needs
no tile: it is the only one of the two marks that stands alone on an avatar or a favicon.
Three directions were drawn (`design/yantrikdb-mark-2026-11-01/`); Pranab chose this one.

## Sizes

Two levels of one drawing, not two logos. Above 64 px the full emblem; at 64 px and below the
core, because the gates, lattice and hairline rings turn to noise at a status-bar size. The core
is the same Y, channels and bindu — only what is around them is left off. `YantrikMark` in
`crates/yantrik-ui-slint/ui/components/yantrik_mark.slint` makes that choice from its size.

The core carries a 2-unit dark edge, so the gold holds on white (app stores, GitHub light mode).

YantrikDB keeps the same two levels. Above 64 px, `yantrikdb-emblem.svg`. At 64 px and below,
`yantrikdb-mark.svg`: the disc, the band and one stratum, with the channels at 16 units and the
bindu at 33, wider than the OS core can afford, because here the disc gives them a ground. At
16 px it is a navy coin with a gold rim and a gold Y in it.

## Colours

| role | hex |
|---|---|
| gold face, light → dark | `#FFF5D2` `#FFE9A8` `#F0C674` `#EABB66` `#D59A42` `#BC8439` `#A96E28` |
| gold bevel: lit / mid / shadow | `#FFF0B7` / `#C18D43` / `#75491D` |
| core, flat gold | `#E4B368` |
| channel light / hot centre / recess | `#168CFF` / `#A0E6FF` / `#03101D` |
| bindu well, rim | `#165BA4` → `#030B18`, `#63C7FF` |
| pearl | `#FFFAE0` → `#EDB652` → `#B67523` |
| gear | `#80643A` → `#35281B` |
| instrument lines (gates, ring, lattice) | `#B88C49` `#AA8145` `#BD9658` |
| strata (YantrikDB): bands, steps, band | `#0E1E34` / `#060F1C`, `#BD9658`, `#C18D43` (small: `#0C1930` disc, `#D5A54E` band) |
| ground | `#05070d` (cards), `#09111F` (icon tile) |
| dark edge, for white grounds | `#17202B` |

The UI keeps its own teal accent (`Theme.accent`). Gold belongs to the mark.

## The name

**YANTRIK OS**, in capitals, set in Marcellus (Brian J. Bonislawsky / Astigmatic, SIL OFL 1.1,
vendored in `fonts/` with its licence). 64-unit capitals, 4 units of tracking after kerning, and
34 units of ink between the K and the O so the category reads as a second word. The A loses its
crossbar and stands over a small blue triangle: a Λ over a point, the yantra's triangle in the
name. The name is outlines, never `<text>`, so it renders the same on every machine; the OS
never sets it in a UI font. **YANTRIK DB** is set the same way, DB standing apart as OS does.

## Clear space and minimum size

Clear space is one eighth of the mark's height on every side. The core is legible down to
16 px, where it is a gold Y with a blue centre. The full emblem is not used below 72 px.

## Regenerating

```sh
pip install fonttools uharfbuzz resvg-py pillow
python3 brand/render.py             # build.py's SVGs, then every raster, then preview.png
python3 brand/build.py              # the SVGs only
python3 brand/render.py --rasters   # the rasters only
```

`render.py` rasterises with **resvg_py** when it is installed — the renderer Slint uses, so a
PNG here is what the OS draws — else cairosvg, else `rsvg-convert`. Pillow only writes the .ico
and checks sizes; it never traces the mark.

What it writes, into this directory:

| file | for |
|---|---|
| `yantrik-icon-{16,32,48,64,128,256,512,1024}.png` | the app icon ladder; 48/128/256 ship to `hicolor` and are committed |
| `yantrik-mark.ico` | favicon: 16 + 32 + 48, each rendered at its own size |
| `apple-icon-180.png` | iOS home screen, on the ground (iOS forbids transparency) |
| `og-image-1200x630.png` | the site's Open Graph and Twitter card |
| `social-preview-1280x640.png` | GitHub's repo social preview |
| `github-avatar-512.png` | the org avatar |
| `yantrikdb-avatar-512.png` | YantrikDB's GitHub avatar |
| `yantrikdb-social-preview-1280x640.png` | YantrikDB's repo social preview |
| `yantrikdb-favicon-{16,32,48}.png`, `yantrikdb-mark.ico` | YantrikDB favicons, each rendered at its own size; committed |
| `preview.png` | contact sheet: everything above in one picture, on dark and on white |

## Where it is used

- **OS**: boot (`boot.slint`), login (`login.slint`), the status bar
  (`components/status_bar.slint`) and About (`about.slint`) use `YantrikMark` and
  `YantrikName` from `components/yantrik_mark.slint`, which embed the SVGs here.
- **Icons on disk**: `deploy/yantrik-os/build-release.sh` stages `yantrik-icon.svg` and the
  48/128/256 PNGs as hicolor `yantrik`; `build-debian-iso.sh` also lands them in
  `/usr/share/icons/hicolor`; `yantrik-update` mirrors `share/` to installed machines. Every
  `apps/desktop-files/*.desktop` says `Icon=yantrik`.
- **Website**: `yantrik-website/public/brand/` should be a copy of the files here; re-copy after
  a change.
- **GitHub**: see `GITHUB.md`; the org avatar and the repo social preview are uploaded by hand.
