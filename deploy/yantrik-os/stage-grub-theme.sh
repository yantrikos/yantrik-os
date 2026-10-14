#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════════════════
# stage-grub-theme.sh DEST [FONT-DIR] — the boot menu's theme, with fonts that draw it
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# Puts grub-theme/ into DEST (theme.txt, the Yantrik icon, the chosen entry's box) and makes the
# fonts theme.txt names with grub-mkfont from DejaVu Sans in FONT-DIR (the build host's
# fonts-dejavu-core by default; build-debian-iso.sh passes the image's own). build-debian-iso.sh
# stages it into the image at /usr/share/yantrik/grub-theme; the live image's menu is a copy of
# that, and both installers copy it to the installed machine's /boot/grub/themes/yantrik.
#
# It fails, rather than leaving a menu of missing-glyph boxes, when a font cannot be made, when
# theme.txt names a font it did not make, or when theme.txt uses a character outside the ranges
# the fonts carry. `stage-grub-theme.sh selftest` stages into a scratch directory, checks the
# result, and holds every boot menu entry the build writes to ASCII.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$HERE/grub-theme"
ICON="$HERE/../../brand/yantrik-icon-128.png"
# What the fonts carry: Latin (names in Western and Central European languages), general
# punctuation (dashes, quotes), arrows (the hint under the menu), box drawing (GRUB's own editor
# and command line, which use the terminal font), and the replacement character.
RANGES="0x20-0x7E,0xA0-0x17F,0x2010-0x206F,0x2190-0x21FF,0x2500-0x259F,0xFFFD-0xFFFD"
# file:size, one font per name theme.txt uses ("DejaVu Sans Regular 16" and so on).
FONTS="DejaVuSans.ttf:16 DejaVuSans.ttf:20 DejaVuSans-Bold.ttf:28"

stage() { # dest font-dir
    dest="$1"; fonts="$2"
    mkfont="$(command -v grub-mkfont || command -v grub2-mkfont || true)"
    [ -n "$mkfont" ] || { echo "stage-grub-theme: grub-mkfont is missing (package grub-common)" >&2; return 1; }
    mkdir -p "$dest"
    cp "$SRC/theme.txt" "$dest/"
    cp "$ICON" "$dest/icon.png"
    # The chosen entry's box: the accent (#8fb4e3) at about 16% over the ground (#0e1018), with a
    # 2 px edge of the accent itself on its left. Drawn here, flat colours needing no image file.
    python3 - "$dest" <<'PY' || return 1
import struct, sys, zlib
def png(path, w, h, rgba):
    rows = b"".join(b"\x00" + bytes(rgba) * w for _ in range(h))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))
png(sys.argv[1] + "/select_c.png", 8, 8, (0x22, 0x2C, 0x3D, 0xFF))
png(sys.argv[1] + "/select_w.png", 2, 8, (0x8F, 0xB4, 0xE3, 0xFF))
PY
    for f in $FONTS; do
        ttf="$fonts/${f%%:*}"; size="${f##*:}"
        [ -f "$ttf" ] || { echo "stage-grub-theme: no $ttf (package fonts-dejavu-core)" >&2; return 1; }
        out="$dest/$(basename "${ttf%.ttf}" | tr 'A-Z-' 'a-z_')_$size.pf2"
        "$mkfont" -s "$size" -r "$RANGES" -o "$out" "$ttf" \
            || { echo "stage-grub-theme: grub-mkfont failed on $ttf at $size" >&2; return 1; }
    done
    check "$dest"
}

# Every font theme.txt names is one of the fonts made here (a .pf2 carries its name as text), and
# every character it writes is in the ranges they carry.
check() { # dest
    names="$(sed -n 's/.*font[-_a-z]*[[:space:]]*[:=][[:space:]]*"\([^"]*\)".*/\1/p' "$1/theme.txt" | sort -u)"
    [ -n "$names" ] || { echo "stage-grub-theme: theme.txt names no font" >&2; return 1; }
    # Explicit returns throughout: called as `if check`, set -e does not stop it.
    old_ifs="$IFS"; IFS='
'
    for name in $names; do
        found=""
        for pf2 in "$1"/*.pf2; do
            grep -aqF "$name" "$pf2" && { found=1; break; }
        done
        [ -n "$found" ] || { IFS="$old_ifs"; echo "stage-grub-theme: theme.txt names \"$name\", which no font here is" >&2; return 1; }
    done
    IFS="$old_ifs"
    python3 - "$1/theme.txt" "$RANGES" <<'PY' || return 1
import sys
text = open(sys.argv[1], encoding="utf-8").read()
ranges = [tuple(int(x, 16) for x in r.split("-")) if "-" in r else (int(r, 16),) * 2 for r in sys.argv[2].split(",")]
outside = sorted({c for c in text if c not in "\n\t" and not any(lo <= ord(c) <= hi for lo, hi in ranges)})
if outside:
    sys.exit("stage-grub-theme: theme.txt uses characters the fonts do not carry: %r" % outside)
PY
}

if [ "${1:-}" = selftest ]; then
    T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
    stage "$T/theme" "${2:-/usr/share/fonts/truetype/dejavu}"
    for f in theme.txt icon.png select_c.png dejavusans_16.pf2 dejavusans_20.pf2 dejavusans_bold_28.pf2; do
        [ -s "$T/theme/$f" ] || { echo "FAIL the staged theme has $f"; exit 1; }
    done
    echo "ok   the theme stages with every font it names, each drawing every character it uses"
    # A theme that names a font nobody made is refused, not drawn in GRUB's ASCII fallback.
    sed 's/DejaVu Sans Bold 28/DejaVu Sans Bold 99/' "$SRC/theme.txt" > "$T/theme/theme.txt"
    if check "$T/theme" 2>/dev/null; then echo "FAIL a font theme.txt names but nobody made is refused"; exit 1; fi
    echo "ok   a font theme.txt names but nobody made is refused"
    # The live menu's entries, and anything else the build puts in a grub.cfg, in ASCII: the
    # serial line and a GRUB without the theme draw them too.
    if LC_ALL=C grep -n 'menuentry ' "$HERE/build-debian-iso.sh" | LC_ALL=C grep -P '[^\x00-\x7F]'; then
        echo "FAIL every boot menu entry is ASCII"; exit 1
    fi
    echo "ok   every boot menu entry is ASCII"
    echo "stage-grub-theme selftest: all passed"
    exit 0
fi

[ $# -ge 1 ] || { echo "usage: $0 DEST [FONT-DIR] | selftest [FONT-DIR]" >&2; exit 2; }
stage "$1" "${2:-/usr/share/fonts/truetype/dejavu}"
