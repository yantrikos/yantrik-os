#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════════════
# build-image.sh — build the Yantrik OS disk image
# ═══════════════════════════════════════════════════════════════════════════════════════
#
# This produces the OS. Boot the result and you are running Yantrik OS — there is no
# install step, nothing is fetched on first boot, and the machine does not become the OS
# by assembling itself out of somebody else's.
#
# That distinction is the whole point and it was got wrong first: an earlier cloud-init
# pulled a stock Debian image and installed Yantrik onto it at boot. That is a provisioning
# script, not a distribution. It also produced the black-screen bug directly — Debian's
# genericcloud kernel carries no virtio-gpu, so the image had to rip out its own kernel
# while running on it. Here the kernel question is settled at build time, once, offline,
# where getting it wrong breaks the build instead of a user's machine.
#
# Debian is the base we build FROM. It is not something the user pulls.
#
#   ./build-image.sh --payload <tarball> [--base <qcow2>] [--out DIR] [--size 12G]
#
# Runs where libguestfs is available (the Proxmox node has it). Needs no VM and no network
# inside the guest: virt-customize edits the filesystem offline.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BASE_IMAGE="/var/lib/vz/template/iso/debian-13-genericcloud-amd64.qcow2"
PAYLOAD=""
OUT_DIR="/var/lib/vz/template/iso"
SIZE="12G"

while [ $# -gt 0 ]; do
  case "$1" in
    --payload) PAYLOAD="$2"; shift 2 ;;
    --base)    BASE_IMAGE="$2"; shift 2 ;;
    --out)     OUT_DIR="$2"; shift 2 ;;
    --size)    SIZE="$2"; shift 2 ;;
    -h|--help) sed -n '2,26p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
fail() { printf '\033[31mFAIL: %s\033[0m\n' "$*" >&2; exit 1; }

# Run a build step, keeping its output. The first version of this script sent every
# virt-customize call to /dev/null and then printed its own message, so a disk-full error
# arrived as "FAIL: payload install failed" with nothing behind it — the one thing that
# could explain it, discarded by the script that then complained it did not know why.
LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT
step() {
  local what="$1"; shift
  if ! "$@" >"$LOG" 2>&1; then
    printf '[31mFAIL: %s[0m
' "$what" >&2
    echo "--- last 15 lines ---" >&2
    tail -15 "$LOG" >&2
    exit 1
  fi
}

command -v virt-customize >/dev/null || fail "virt-customize not installed (apt install libguestfs-tools)"
[ -f "$BASE_IMAGE" ] || fail "no base image at $BASE_IMAGE"
[ -n "$PAYLOAD" ] || fail "--payload is required (build-release.sh produces it)"
[ -f "$PAYLOAD" ] || fail "payload not found: $PAYLOAD"

# The version comes from the BUILD manifest INSIDE the payload, not from its filename.
# Parsing the name produced yantrik-os-yantrik-payload.tar.zst-amd64.qcow2, because the
# file had been renamed on download. A name is a label someone chose; the manifest is
# what the build said about itself.
VERSION="$(tar --zstd -xOf "$PAYLOAD" --wildcards '*/BUILD' 2>/dev/null | grep '^version=' | cut -d= -f2)"
[ -n "$VERSION" ] || fail "no version in the payload BUILD manifest"
NAME="yantrik-os-${VERSION}-amd64"
IMAGE="$OUT_DIR/${NAME}.qcow2"

say "Building $NAME"
echo "   base:    $BASE_IMAGE"
echo "   payload: $(basename "$PAYLOAD") ($(du -h "$PAYLOAD" | cut -f1))"

mkdir -p "$OUT_DIR"
# `qemu-img resize` grows the DISK; the partition and filesystem inside do not follow. A
# cloud image normally grows on first boot via growpart — which never happens here, because
# not booting is the point. The first attempt filled the original 2.8G with packages and the
# payload upload died with a bare "write error". virt-resize grows the partition while it
# copies, so the filesystem is actually the size the disk claims.
say "Sizing"
qemu-img create -f qcow2 "$IMAGE" "$SIZE" >/dev/null
virt-resize --quiet --expand /dev/sda1 "$BASE_IMAGE" "$IMAGE" >/dev/null 2>&1   || fail "could not expand the base image to $SIZE"
FS_SIZE="$(virt-df -h -a "$IMAGE" 2>/dev/null | awk '/sda1/{getline; print $1; exit}')"
echo "   disk $SIZE, root filesystem ${FS_SIZE:-unknown}"

# ── The kernel, settled at build time ───────────────────────────────────────────────────
#
# The cloud kernel is removed while nothing is running on it, so there is no ordering
# problem and no reboot dance. If this step fails the BUILD fails, which is the correct
# place for that failure — a machine that boots without /dev/dri shows a black screen and
# logs nothing wrong, and is a miserable thing to debug from the outside.
#
# Remove the VERSIONED packages, not just the metapackage. `linux-image-cloud-amd64` is a
# pointer; purging it leaves linux-image-6.12.107+deb13-cloud-amd64 and its vmlinuz sitting
# in /boot, which is exactly what the runtime cloud-init version did — it reported success
# and changed nothing. The check below reads /boot rather than asking apt, because apt was
# perfectly happy both times.
say "Kernel"
virt-customize -a "$IMAGE" \
  --install linux-image-amd64 \
  --run-command 'apt-get remove -y --purge $(dpkg-query -W -f="\${Package}\n" "linux-image*cloud*" 2>/dev/null | tr "\n" " ") || true' \
  --run-command 'update-grub' \

echo "   linux-image-amd64 in, cloud kernel out"

# Prove it rather than assume it: the whole reason this script exists is that the runtime
# version of this step silently did not take.
# Same reason: read the filesystem rather than ask a command to report on it. If a cloud
# kernel image is still on disk the purge did not take, and this image would boot with no
# /dev/dri — a black screen and no error anywhere, which is the bug this script exists for.
if virt-ls -a "$IMAGE" /boot 2>/dev/null | grep -q "cloud-amd64"; then
  fail "a cloud kernel is still in /boot — this image would boot with no /dev/dri"
fi
echo "   verified: no cloud kernel left in /boot"

say "Packages"
# One layer, one apt run: the desktop, the eyes, the ears, and what yos needs to talk CDP.
# mpv: Files opens sound and video in mpv's own window (#255); the image shipped no player.
# nano, htop: the person's editor (yantrik-session exports EDITOR=nano) and process viewer;
# the image shipped neither (#210). iproute2, iputils-ping: `ip`, `ss` and `ping`, named by
# the mind's network tools — installed here as well so all three provisioning paths lay
# down the same base system rather than drifting from each other (#210).
virt-customize -a "$IMAGE" \
  --install labwc,seatd,mesa-utils,foot,chromium,pipewire-pulse,wireplumber,pulseaudio-utils,python3-websocket,qemu-guest-agent,curl,ca-certificates,fontconfig,grim,wlrctl,wlr-randr,mpv,nano,htop,iproute2,iputils-ping \

echo "   desktop, browser, audio, agent surface deps"

say "Payload"
virt-customize -a "$IMAGE" \
  --mkdir /opt/yantrik \
  --upload "$PAYLOAD:/tmp/payload.tar.zst" \
  --run-command 'tar --zstd -xf /tmp/payload.tar.zst -C /opt/yantrik --strip-components=1' \
  --run-command 'rm -f /tmp/payload.tar.zst' \
  --run-command 'ln -sf /opt/yantrik/bin/yos /usr/local/bin/yos' \

# virt-customize --run-command does NOT return the guest command's stdout to the caller —
# it goes to virt-customize's own log. An earlier check grepped for a number that could
# never appear and failed a build that had worked. virt-ls actually returns what it lists.
COUNT="$(virt-ls -a "$IMAGE" /opt/yantrik/bin 2>/dev/null | wc -l)"
[ "${COUNT:-0}" -gt 5 ] || fail "payload did not unpack (found ${COUNT:-0} binaries)"
echo "   $COUNT binaries at /opt/yantrik/bin"

say "Session"
virt-customize -a "$IMAGE" \
  --write '/etc/systemd/system/yantrik-session.service:[Unit]
Description=Yantrik OS desktop session
After=systemd-user-sessions.service seatd.service
Wants=seatd.service

[Service]
User=yantrik
PAMName=login
TTYPath=/dev/tty1
StandardInput=tty
TTYReset=yes
TTYVHangup=yes
Environment=XDG_RUNTIME_DIR=/run/user/1000
Environment=XDG_SESSION_TYPE=wayland
Environment=WLR_RENDERER_ALLOW_SOFTWARE=1
ExecStart=/opt/yantrik/bin/yantrik-session
Restart=always
RestartSec=3

[Install]
WantedBy=graphical.target' \
  --write '/opt/yantrik/bin/yantrik-session:#!/bin/sh
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
mkdir -p "$XDG_RUNTIME_DIR"
# The window decorations and the typeface they are drawn in.
#
# labwc reads its config from the user'\''s home, so this needs no root and re-applies on every
# start — a machine that was deployed before the theme existed picks it up by rebooting. Both
# sources live in the release payload; see deploy/yantrik-os/build-release.sh.
CHROME=/opt/yantrik/share
if [ -d "$CHROME/labwc" ]; then
  mkdir -p "$HOME/.config/labwc" "$HOME/.local/share/themes/Yantrik/labwc"
  cp -f "$CHROME/labwc/rc.xml" "$HOME/.config/labwc/rc.xml"
  cp -f "$CHROME/labwc/themerc" "$HOME/.local/share/themes/Yantrik/labwc/themerc"
fi
if [ -d "$CHROME/fonts" ]; then
  mkdir -p "$HOME/.local/share/fonts"
  cp -f "$CHROME/fonts/"*.ttf "$HOME/.local/share/fonts/" 2>/dev/null
  fc-cache -f "$HOME/.local/share/fonts" >/dev/null 2>&1
fi
exec labwc -s "/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml"' \
  --chmod '0755:/opt/yantrik/bin/yantrik-session' \
  --run-command 'useradd -m -s /bin/bash -G sudo,video,render,input,audio yantrik 2>/dev/null || true' \
  --run-command 'echo "yantrik ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/yantrik' \
  --run-command 'chown -R yantrik:yantrik /opt/yantrik' \
  --run-command 'systemctl enable seatd yantrik-session qemu-guest-agent' \
  --run-command 'systemctl set-default graphical.target' \
  >/dev/null 2>&1 || fail "session setup failed"
echo "   session unit enabled, boots to graphical.target"

# A VM has no sound card. Without a virtual one the machine is simply deaf and `yos web
# listen` records silence, which is indistinguishable from a video with nobody talking.
virt-customize -a "$IMAGE" \
  --mkdir /etc/pipewire/pipewire.conf.d \
  --write '/etc/pipewire/pipewire.conf.d/10-yantrik-capture.conf:context.modules = [
  { name = libpipewire-module-loopback
    args = {
      node.name = "yantrik_capture"
      node.description = "yantrik_capture"
      capture.props = { media.class = "Audio/Sink" }
    }
  }
]' >/dev/null 2>&1 || fail "audio setup failed"
echo "   audio capture sink"

say "Sealing"
# Strip the identity of the machine this was built as, so every instance created from the
# image is its own machine rather than a clone with a borrowed hostname and host keys.
virt-sysprep -a "$IMAGE" --operations defaults,-ssh-userdir >/dev/null 2>&1 \
  || echo "   (sysprep reported a problem — check before publishing)"
qemu-img convert -O qcow2 -c "$IMAGE" "$IMAGE.tmp" && mv "$IMAGE.tmp" "$IMAGE"
echo "   sysprepped and compressed"

( cd "$OUT_DIR" && sha256sum "$(basename "$IMAGE")" > "$(basename "$IMAGE").sha256" )

say "Built"
echo "   $IMAGE"
echo "   $(du -h "$IMAGE" | cut -f1)  ·  $COUNT binaries  ·  $VERSION"
echo
echo "   Create an instance:"
echo "     qm create <vmid> --name yantrik --memory 8192 --cores 4 --net0 virtio,bridge=vmbr0 \\"
echo "       --scsihw virtio-scsi-single --agent 1 --serial0 socket --vga serial0"
echo "     qm importdisk <vmid> $IMAGE local-lvm"
echo "     qm set <vmid> --scsi0 local-lvm:vm-<vmid>-disk-0 --boot order=scsi0 --ide2 local-lvm:cloudinit"
