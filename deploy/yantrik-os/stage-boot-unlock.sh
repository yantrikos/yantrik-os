#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════════════════
# stage-boot-unlock.sh — the one-password start, put into a rootfs
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# Called by build-debian-iso.sh (as root) after plymouth and cryptsetup-initramfs are installed.
# Kept apart so it can be run, and checked, against a scratch directory without building an
# image:
#
#   sh deploy/yantrik-os/stage-boot-unlock.sh /tmp/fake-rootfs
#
# Stages (boot-unlock/):
#   - askpass, the disk's keyscript; the installer names it in crypttab for an encrypted root;
#   - the initramfs hook and local-bottom script, which do nothing unless crypttab names it;
#   - consume and enrol, root's, and the socket that lets the session ask consume, once;
#   - the Yantrik plymouth theme.
# Nothing here changes a machine whose root is not encrypted by the installer.
#
# $own below is "" or "-o root -g root", split on purpose.
# shellcheck disable=SC2086
set -eu

ROOTFS="${1:?usage: stage-boot-unlock.sh ROOTFS}"
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$HERE/boot-unlock"
LIB=/usr/lib/yantrik/boot-unlock
THEME=/usr/share/plymouth/themes/yantrik

for f in askpass local-bottom initramfs-hook consume enrol yantrik-boot-unlock.socket \
         yantrik-boot-unlock@.service theme/yantrik.plymouth theme/yantrik.script theme/field.png; do
    [ -f "$SRC/$f" ] || { echo "stage-boot-unlock: missing $SRC/$f" >&2; exit 1; }
done

own=""
[ "$(id -u)" = 0 ] && own="-o root -g root"

put() { # mode src dest
    install -d -m 0755 $own "$(dirname "$3")"
    case "$2" in
        *.png) cp "$2" "$3.tmp" ;;
        *) tr -d '\r' < "$2" > "$3.tmp" ;;
    esac
    chmod "$1" "$3.tmp"
    [ -z "$own" ] || chown root:root "$3.tmp"
    mv -f "$3.tmp" "$3"
}

put 0755 "$SRC/askpass"          "$ROOTFS$LIB/askpass"
put 0700 "$SRC/consume"          "$ROOTFS$LIB/consume"
put 0700 "$SRC/enrol"            "$ROOTFS$LIB/enrol"
put 0755 "$SRC/initramfs-hook"   "$ROOTFS/etc/initramfs-tools/hooks/yantrik-unlock"
put 0755 "$SRC/local-bottom"     "$ROOTFS/etc/initramfs-tools/scripts/local-bottom/yantrik-unlock"
put 0644 "$SRC/yantrik-boot-unlock.socket"   "$ROOTFS/etc/systemd/system/yantrik-boot-unlock.socket"
put 0644 "$SRC/yantrik-boot-unlock@.service" "$ROOTFS/etc/systemd/system/yantrik-boot-unlock@.service"
put 0644 "$SRC/theme/yantrik.plymouth" "$ROOTFS$THEME/yantrik.plymouth"
put 0644 "$SRC/theme/yantrik.script"   "$ROOTFS$THEME/yantrik.script"
put 0644 "$SRC/theme/field.png"        "$ROOTFS$THEME/field.png"

# Enabled by hand, as `systemctl enable` would, so this works on a scratch dir with no systemd.
install -d -m 0755 $own "$ROOTFS/etc/systemd/system/sockets.target.wants"
ln -sfn /etc/systemd/system/yantrik-boot-unlock.socket \
    "$ROOTFS/etc/systemd/system/sockets.target.wants/yantrik-boot-unlock.socket"

echo "stage-boot-unlock: keyscript, initramfs hook, marker, root helper (socket enabled) and plymouth theme staged in $ROOTFS"
