#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════════════════
# initramfs-check.sh — build a real initramfs with the one-password start and look inside
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# Stages boot-unlock onto THIS system, writes a German keyboard and an enrolled account, and runs
# mkinitramfs with a crypttab naming the keyscript. Then checks the image:
#
#   graphical  the keyscript, cryptsetup's text prompt, the marker script, plymouth, the
#              Yantrik theme with the account's name and the layout written into it, a label
#              plugin and a font, the XKB data, the layout, a console keymap, plymouthd.conf
#              naming the theme;
#   text       the same build with plymouth's label plugin hidden: plymouthd.conf names the
#              text `details` theme, its plugin is there, and the keyscript and prompt still are;
#   untouched  a crypttab without the keyscript: none of it, and cryptsetup's own prompt.
#
# It writes to /etc (crypttab is put back), /usr and /lib/modules, so it only runs where that is thrown away: a CI
# runner or a scratch container, as root, with the packages the image installs:
#
#   apt-get install initramfs-tools cryptsetup-initramfs plymouth plymouth-label console-setup \
#       busybox fonts-dejavu-core kbd xkb-data jq
#   sh deploy/yantrik-os/boot-unlock/initramfs-check.sh --scratch-system
set -u
[ "${1:-}" = --scratch-system ] || { echo "usage: $0 --scratch-system  (writes to this system; see the header)" >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 2; }
for tool in mkinitramfs lsinitramfs unmkinitramfs plymouth cryptsetup busybox setupcon jq; do
    command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 2; }
done
HERE="$(cd "$(dirname "$0")" && pwd)"
T="$(mktemp -d)"
KVER="0.0.0-yantrik-check"
PLUGINS="$(plymouth --get-splash-plugin-path)"; PLUGINS="${PLUGINS%/}"
HIDDEN=""
LOOP=""
cleanup() {
    [ -n "$HIDDEN" ] && mv "$T/hidden/$HIDDEN" "$PLUGINS/$HIDDEN"
    [ -n "$LOOP" ] && losetup -d "$LOOP"
    if [ -e "$T/crypttab.orig" ]; then mv "$T/crypttab.orig" /etc/crypttab; else rm -f /etc/crypttab; fi
    rm -rf "$T" "/lib/modules/$KVER"
}
trap cleanup EXIT
FAILS=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; FAILS=$((FAILS + 1)); }
has() { grep -qE "$1" "$T/list" && ok "$2" || bad "$2"; }
lacks() { grep -qE "$1" "$T/list" && bad "$2" || ok "$2"; }

sh "$HERE/../stage-boot-unlock.sh" / >/dev/null || { echo "staging failed" >&2; exit 1; }

# The person: a German keyboard, and an account with a name and a password, enrolled.
printf 'XKBMODEL="pc105"\nXKBLAYOUT="de"\nXKBVARIANT=""\nXKBOPTIONS=""\nBACKSPACE="guess"\n' > /etc/default/keyboard
setupcon --save-only >/dev/null 2>&1 || true
id ykcheck >/dev/null 2>&1 || useradd -M -c "Asha Check" ykcheck
usermod -p '$6$salt$abcdefghijklmnopqrstuv' ykcheck

# A kernel with no modules is enough for the hooks to run.
mkdir -p "/lib/modules/$KVER"
: > "/lib/modules/$KVER/modules.order"; : > "/lib/modules/$KVER/modules.builtin"
depmod "$KVER" 2>/dev/null || true

# cryptsetup's hook reads /etc/crypttab and nothing else; it is put back afterwards.
[ -e /etc/crypttab ] && cp -a /etc/crypttab "$T/crypttab.orig"

# A real LUKS2 container on a loop device: cryptsetup's hook leaves out a crypttab line whose
# source it cannot find. The passphrase is a throwaway, given on stdin.
truncate -s 32M "$T/luks.img"
LOOP="$(losetup -f --show "$T/luks.img")" || { echo "no loop device" >&2; exit 2; }
printf 'throwaway' | cryptsetup luksFormat --type luks2 --batch-mode --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
    --key-file=- "$LOOP" || { echo "luksFormat failed" >&2; exit 2; }
UUID="$(cryptsetup luksUUID "$LOOP")"
# Enrolled against it, as the installer does with the root it just made: the account, and a
# digest of the header's one keyslot.
/usr/lib/yantrik/boot-unlock/enrol ykcheck "$LOOP" || bad "enrol"
grep -qE "^header=[0-9a-f]{64}$" /etc/yantrik/boot-unlock.conf && ok "enrol records the LUKS header's digest" \
    || bad "enrol records the LUKS header's digest"
getent group yantrik-boot-unlock | grep -qE ":ykcheck$" && ok "enrol makes the account the one member of yantrik-boot-unlock" \
    || bad "enrol makes the account the one member of yantrik-boot-unlock"
udevadm settle 2>/dev/null || true
[ -e "/dev/disk/by-uuid/$UUID" ] || { mkdir -p /dev/disk/by-uuid; ln -sfn "$LOOP" "/dev/disk/by-uuid/$UUID"; }

build() { # crypttab-options
    printf 'yantrik-root UUID=%s none %s\n' "$UUID" "$1" > /etc/crypttab
    rm -f "$T/initrd.img"
    CRYPTSETUP=y mkinitramfs -o "$T/initrd.img" "$KVER" > "$T/mkinitramfs.log" 2>&1 \
        || { cat "$T/mkinitramfs.log"; bad "mkinitramfs ran"; return 1; }
    lsinitramfs "$T/initrd.img" > "$T/list"
    rm -rf "$T/x"; unmkinitramfs "$T/initrd.img" "$T/x" >/dev/null 2>&1
    ROOT="$T/x"; [ -d "$T/x/main" ] && ROOT="$T/x/main"
}

KS=/usr/lib/yantrik/boot-unlock/askpass
echo "── graphical"
build "luks,initramfs,keyscript=$KS"
has "^(usr/)?lib/yantrik/boot-unlock/askpass$" "the keyscript is in the initramfs"
has "cryptsetup/askpass$" "and cryptsetup's text prompt it falls back to"
has "^scripts/local-bottom/yantrik-unlock$" "the signed-in marker script"
has "bin/plymouth$" "plymouth"
has "sbin/plymouthd$" "plymouthd"
has "plymouth/script.so$" "the script plugin"
has "plymouth/label[^/]*\.so$" "a label plugin to write with"
has "plymouth/details.so$" "the text plugin plymouth falls back to"
has "plymouth/renderers/(drm|frame-buffer).so$" "a renderer for simpledrm or efifb"
has "^usr/share/plymouth/themes/yantrik/yantrik.script$" "the Yantrik theme"
has "^usr/share/plymouth/themes/yantrik/field.png$" "its password field"
has "DejaVuSans.ttf$" "a font"
has "^usr/share/X11/xkb/symbols/de$" "the XKB data for the layout"
has "(etc/console-setup/cached_.*\.kmap(\.gz)?|etc/boottime\.kmap\.gz)$" "a console keymap"
grep -qx 'name_text = "Asha Check";' "$ROOT/usr/share/plymouth/themes/yantrik/yantrik.script" \
    && ok "the theme names the person" || bad "the theme names the person"
grep -qx 'layout_text = "de";' "$ROOT/usr/share/plymouth/themes/yantrik/yantrik.script" \
    && ok "the theme names the layout" || bad "the theme names the layout"
grep -q 'XKBLAYOUT="de"' "$ROOT/etc/default/keyboard" 2>/dev/null \
    && ok "the layout is the one chosen" || bad "the layout is the one chosen"
grep -qx 'Theme=yantrik' "$ROOT/etc/plymouth/plymouthd.conf" 2>/dev/null \
    && ok "plymouthd.conf names the Yantrik theme" || bad "plymouthd.conf names the Yantrik theme"
grep -q "keyscript=$KS" "$ROOT/cryptroot/crypttab" 2>/dev/null \
    && ok "the initramfs crypttab names the keyscript" || bad "the initramfs crypttab names the keyscript"
for s in "$ROOT$KS" "$ROOT/scripts/local-bottom/yantrik-unlock"; do
    busybox sh -n "$s" && ok "$(basename "$s") parses under busybox" || bad "$(basename "$s") parses under busybox"
done
for applet in awk grep sed mv mkdir chmod cat; do
    [ -e "$ROOT/bin/$applet" ] || [ -e "$ROOT/usr/bin/$applet" ] || [ -e "$ROOT/sbin/$applet" ] \
        && ok "$applet for the marker" || bad "$applet for the marker"
done
lacks "boot-unlock\.conf|shadow" "nothing about the account's password goes in"

echo "── text (no label plugin)"
label=""
for l in label-pango.so label.so label-freetype.so; do [ -f "$PLUGINS/$l" ] && { label="$l"; break; }; done
mkdir -p "$T/hidden"; mv "$PLUGINS/$label" "$T/hidden/$label"; HIDDEN="$label"
build "luks,initramfs,keyscript=$KS"
mv "$T/hidden/$HIDDEN" "$PLUGINS/$HIDDEN"; HIDDEN=""
grep -qx 'Theme=details' "$ROOT/etc/plymouth/plymouthd.conf" 2>/dev/null \
    && ok "plymouthd.conf names the text theme" || bad "plymouthd.conf names the text theme"
has "plymouth/details.so$" "the text theme's plugin"
has "^(usr/)?lib/yantrik/boot-unlock/askpass$" "the keyscript, with its words"
has "cryptsetup/askpass$" "and cryptsetup's text prompt"
has "(etc/console-setup/cached_.*\.kmap(\.gz)?|etc/boottime\.kmap\.gz)$" "the console keymap"
grep -q "W: yantrik-unlock: plymouth has no label plugin" "$T/mkinitramfs.log" \
    && ok "the build says why" || bad "the build says why"

echo "── untouched (crypttab without the keyscript)"
build "luks,initramfs"
lacks "boot-unlock/askpass|themes/yantrik" "nothing of the Yantrik prompt"
grep -q "keyscript" "$ROOT/cryptroot/crypttab" 2>/dev/null && bad "the initramfs crypttab is cryptsetup's own" || ok "the initramfs crypttab is cryptsetup's own"
# The marker script is always there and does nothing without the keyscript (selftest holds it).
has "cryptsetup/askpass$" "cryptsetup's own prompt"

[ "$FAILS" = 0 ] && echo "initramfs-check: all passed" || { echo "initramfs-check: $FAILS failed"; exit 1; }
