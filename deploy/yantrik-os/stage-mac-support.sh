#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════════════════
# stage-mac-support.sh — what an Intel Mac needs, put into a rootfs
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# Called by build-debian-iso.sh (as root) after mbpfan is installed in the chroot. Kept apart so
# it can be run, and checked, against a scratch directory without building an image:
#
#   sh deploy/yantrik-os/stage-mac-support.sh /tmp/fake-rootfs
#
# Stages, and enables only what is safe to enable on every machine:
#   - yantrik-broadcom-wifi + its unit + the NetworkManager dispatcher hook. Enabled: on a
#     machine without the chip it reads sysfs, writes "absent" and exits.
#   - yantrik-mac-fans + mbpfan's drop-in, which makes mbpfan run only on Apple hardware.
#
# Nothing here is a Broadcom binary: the Wi-Fi driver is fetched from Debian on the machine.
#
# $own below is "" or "-o root -g root", split on purpose.
# shellcheck disable=SC2086
set -eu

ROOTFS="${1:?usage: stage-mac-support.sh ROOTFS}"
HERE="$(cd "$(dirname "$0")" && pwd)"
MAC="$HERE/mac"

for f in "$HERE/yantrik-broadcom-wifi" "$HERE/yantrik-mac-fans" "$MAC/yantrik-broadcom-wifi.service" \
         "$MAC/90-yantrik-broadcom-wifi" "$MAC/mbpfan-yantrik-apple.conf"; do
    [ -f "$f" ] || { echo "stage-mac-support: missing $f" >&2; exit 1; }
done

# install(1) as root sets the owner; run unprivileged against a scratch dir it cannot, and need not.
own=""
[ "$(id -u)" = 0 ] && own="-o root -g root"

# Copied out of a working tree edited on Windows, a CRLF shebang does not run. Strip on the way in.
put() { # mode src dest
    install -d -m 0755 $own "$(dirname "$3")"
    tr -d '\r' < "$2" > "$3.tmp"
    chmod "$1" "$3.tmp"
    [ -z "$own" ] || chown root:root "$3.tmp"
    mv -f "$3.tmp" "$3"
}

put 0755 "$HERE/yantrik-broadcom-wifi"            "$ROOTFS/usr/lib/yantrik/yantrik-broadcom-wifi"
put 0755 "$HERE/yantrik-mac-fans"                 "$ROOTFS/usr/lib/yantrik/yantrik-mac-fans"
put 0644 "$MAC/yantrik-broadcom-wifi.service"     "$ROOTFS/etc/systemd/system/yantrik-broadcom-wifi.service"
put 0755 "$MAC/90-yantrik-broadcom-wifi"          "$ROOTFS/etc/NetworkManager/dispatcher.d/90-yantrik-broadcom-wifi"
put 0644 "$MAC/mbpfan-yantrik-apple.conf"         "$ROOTFS/etc/systemd/system/mbpfan.service.d/yantrik-apple.conf"

# On PATH for the person who wants to force the other driver: sudo yantrik-broadcom-wifi --driver b43
install -d -m 0755 $own "$ROOTFS/usr/local/sbin"
ln -sfn /usr/lib/yantrik/yantrik-broadcom-wifi "$ROOTFS/usr/local/sbin/yantrik-broadcom-wifi"

# Enabled by hand, as `systemctl enable` would, so this works on a scratch dir with no systemd.
install -d -m 0755 $own "$ROOTFS/etc/systemd/system/multi-user.target.wants"
ln -sfn /etc/systemd/system/yantrik-broadcom-wifi.service \
    "$ROOTFS/etc/systemd/system/multi-user.target.wants/yantrik-broadcom-wifi.service"

echo "stage-mac-support: Broadcom Wi-Fi fetcher (enabled), mbpfan drop-in and fan settings staged in $ROOTFS"
