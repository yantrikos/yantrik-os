#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# build-debian-iso.sh — Create a bootable Yantrik OS ISO (Debian-based)
# ═══════════════════════════════════════════════════════════════
#
# Builds a Debian Trixie-based live/installer ISO containing:
#   - Debian 13 (testing) minimal (glibc, systemd, NetworkManager)
#   - labwc Wayland compositor (Yantrik runs fullscreen)
#   - Yantrik UI + CLI binaries (pre-compiled)
#   - MiniLM embedder model (~87MB)
#   - Whisper tiny model (~146MB)
#   - Qwen 3.5 4B GGUF offline LLM (~2.5GB)
#   - Calamares installer for disk installation
#   - All packages baked in — NO internet needed for install
#
# Output: yantrik-os-<version>.iso
#
# Requirements (run on Ubuntu/WSL2):
#   - debootstrap, xorriso, squashfs-tools, grub-pc-bin, grub-efi-amd64-bin
#   - sudo access
#   - yantrik-ui + yantrik binaries already built (release)
#
# Usage:
#   ./build-debian-iso.sh                     # default (Ollama backend)
#   ./build-debian-iso.sh --with-llm          # include offline LLM (~2.5GB)
#   ./build-debian-iso.sh --skip-models       # skip model downloads
#   YANTRIK_BINARY=/path/to/bin ./build-debian-iso.sh
#
# Test:
#   qemu-system-x86_64 -m 4G -cdrom yantrik-os-*.iso -boot d \
#     -enable-kvm -display gtk -device virtio-vga
#
# VirtualBox:
#   Import ISO as boot media, 4GB RAM, EFI enabled
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Where cargo ACTUALLY puts things, asked of cargo rather than assumed — the same question
# build-release.sh asks, for the same reason.
#
# This used to read "${CARGO_TARGET_DIR:-/home/yantrik/target-yantrik}": one developer's home
# directory, hardcoded. On every machine but that one the prerequisite check below looked for
# yantrik-ui in a directory that does not exist and the build refused to start, with a message
# telling you to run a build you had already run.
TARGET_DIR="${TARGET_DIR:-$( \
  cd "$PROJECT_ROOT" && cargo metadata --format-version 1 --no-deps --offline 2>/dev/null \
    | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')}"
[ -n "$TARGET_DIR" ] || TARGET_DIR="${CARGO_TARGET_DIR:-$PROJECT_ROOT/target}"

# ── Configuration ──
# From git, like the tarball's. A version hardcoded in a build script names whatever it named
# the day it was written: this said 0.3.0 for five months, across every commit, so two ISOs
# built a season apart had the same filename and nothing on either could tell them apart.
#
# Exported, so build-release.sh below packages the bundle under this same string rather than
# running `git describe` again for itself. Two describes a few minutes apart can differ — a tag
# pushed between them is enough — and then the ISO's filename names one build while the BUILD
# marker inside it names another.
export YANTRIK_VERSION="${YANTRIK_VERSION:-$(git -C "$PROJECT_ROOT" describe --tags --always --dirty 2>/dev/null)}"
[ -n "$YANTRIK_VERSION" ] || export YANTRIK_VERSION="0.0.0-unknown"
DEBIAN_SUITE="trixie"
DEBIAN_MIRROR="http://deb.debian.org/debian"
ARCH="amd64"

WORK_DIR="/tmp/yantrik-debian-iso"
ROOTFS="$WORK_DIR/rootfs"
ISO_DIR="$WORK_DIR/iso"
OUTPUT="yantrik-os-${YANTRIK_VERSION}.iso"

# What the OS is made of is DISCOVERED by build-release.sh, never listed here.
# This script used to name two binaries and therefore shipped an ISO with no apps
# and no services — a desktop that looked right and could not open anything.
# Set RELEASE_TARBALL to reuse a prebuilt artifact instead of repackaging.
RELEASE_TARBALL="${RELEASE_TARBALL:-}"

# The mind is its own bundle, built from the yantrik-mind repository by its
# deploy/yantrik-os/package.sh, and installed BESIDE the OS (/opt/yantrik-mind), not
# inside it. Required: an image without it boots a body with no mind, which is the
# failure a fresh install exists to catch. YANTRIK_ISO_WITHOUT_MIND=1 builds one on purpose.
MIND_TARBALL="${MIND_TARBALL:-}"

INCLUDE_LLM=false
INCLUDE_WHISPER=false
# Embedder is ALWAYS included — essential for CognitiveRouter (Core Mode)
# Without it, 80% of functionality is lost. Only 22MB.

# ── Parse flags ──
for arg in "$@"; do
    case "$arg" in
        --with-llm)     INCLUDE_LLM=true; INCLUDE_WHISPER=true ;;
        --with-models)  INCLUDE_WHISPER=true ;;
        --no-embedder)  NO_EMBEDDER=true ;;
        --help|-h)
            echo "Usage: $0 [--with-models] [--with-llm]"
            echo "  (default)      Include embedder for Core Mode (~22MB, always)"
            echo "  --with-models  Also include whisper voice model (+146MB)"
            echo "  --with-llm    Include all models + offline LLM (+2.7GB)"
            echo ""
            echo "Embedder is always included (essential for CognitiveRouter)."
            echo "Without it, tool routing, recipe matching, and most"
            echo "interactions require an external LLM."
            exit 0
            ;;
    esac
done

# ── Colors ──
CYAN='\033[0;36m'
GREEN='\033[0;32m'
AMBER='\033[0;33m'
RED='\033[0;31m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

step()  { echo -e "\n${CYAN}::${NC} ${BOLD}$1${NC}"; }
info()  { echo -e "   ${DIM}$1${NC}"; }
ok()    { echo -e "   ${GREEN}✓${NC} $1"; }
warn()  { echo -e "   ${AMBER}!${NC} $1"; }
fail()  { echo -e "   ${RED}✗${NC} $1"; exit 1; }

echo
echo -e "${CYAN}╔═══════════════════════════════════════════════════╗${NC}"
# No "v" prefix here: git describe already returns one (v0.1.0-217-g48ac75c), and the banner
# printed "vv0.1.0-217-g48ac75c".
echo -e "${CYAN}║${NC}  ${BOLD}Yantrik OS${NC} — Debian ISO Builder ${YANTRIK_VERSION}         ${CYAN}║${NC}"
echo -e "${CYAN}║${NC}  ${DIM}Fully offline-capable installation ISO${NC}           ${CYAN}║${NC}"
echo -e "${CYAN}╚═══════════════════════════════════════════════════╝${NC}"
echo
echo -e "  Base:       ${BOLD}Debian ${DEBIAN_SUITE}${NC} (${ARCH})"
echo -e "  Core Mode:   ${BOLD}always (embedder baked in)${NC}"
echo -e "  Voice:       ${BOLD}${INCLUDE_WHISPER}${NC}"
echo -e "  Offline LLM: ${BOLD}${INCLUDE_LLM}${NC}"
echo -e "  Output:     ${BOLD}${OUTPUT}${NC}"
echo

# ── Verify prerequisites ──
MISSING=""
for cmd in debootstrap xorriso mksquashfs grub-mkrescue setcap getcap; do
    command -v "$cmd" &>/dev/null || MISSING="$MISSING $cmd"
done
if [ -n "$MISSING" ]; then
    fail "Missing tools:$MISSING\n  Install: sudo apt install debootstrap xorriso squashfs-tools grub-pc-bin grub-efi-amd64-bin mtools"
fi

if [ -n "$RELEASE_TARBALL" ]; then
    [ -f "$RELEASE_TARBALL" ] || fail "RELEASE_TARBALL not found: $RELEASE_TARBALL"
elif [ ! -x "$TARGET_DIR/release/yantrik-ui" ]; then
    fail "No built workspace at $TARGET_DIR/release\n  Build first: cargo build --release --workspace"
fi

# ── Cleanup function ──
cleanup() {
    echo "Cleaning up mounts..."
    for mp in proc sys dev/pts dev; do
        sudo umount "$ROOTFS/$mp" 2>/dev/null || true
    done
}
trap cleanup EXIT

# ═══════════════════════════════════════════════════════════════
# STEP 1: Bootstrap Debian rootfs
# ═══════════════════════════════════════════════════════════════
step "[1/10] Bootstrapping Debian ${DEBIAN_SUITE} rootfs..."

sudo rm -rf "$WORK_DIR"
mkdir -p "$ROOTFS" "$ISO_DIR/boot/grub" "$ISO_DIR/live" "$ISO_DIR/install"

sudo debootstrap \
    --arch="$ARCH" \
    --variant=minbase \
    --include=systemd,systemd-sysv,dbus,udev,sudo,ca-certificates,locales,wget,curl \
    "$DEBIAN_SUITE" "$ROOTFS" "$DEBIAN_MIRROR"

ok "Base system bootstrapped"

# Mount for chroot
sudo mount -t proc proc "$ROOTFS/proc"
sudo mount -t sysfs sysfs "$ROOTFS/sys"
sudo mount --bind /dev "$ROOTFS/dev"
sudo mount --bind /dev/pts "$ROOTFS/dev/pts"
sudo cp /etc/resolv.conf "$ROOTFS/etc/resolv.conf"

# ═══════════════════════════════════════════════════════════════
# STEP 2: Configure apt sources + install packages
# ═══════════════════════════════════════════════════════════════
step "[2/10] Installing system packages (all cached for offline)..."

# Full sources.list with contrib + non-free for firmware
sudo tee "$ROOTFS/etc/apt/sources.list" > /dev/null <<APT
deb ${DEBIAN_MIRROR} ${DEBIAN_SUITE} main contrib non-free non-free-firmware
APT

sudo chroot "$ROOTFS" /bin/bash <<'CHROOT_PACKAGES'
set -e
export DEBIAN_FRONTEND=noninteractive

apt-get update -qq

# ── Kernel + boot ──
apt-get install -y -qq \
    linux-image-amd64 \
    grub-pc grub-efi-amd64-bin \
    initramfs-tools \
    live-boot live-config live-config-systemd

# ── Wayland desktop (installer-only minimal) ──
apt-get install -y -qq \
    labwc foot \
    wl-clipboard \
    wlrctl wlr-randr \
    mesa-utils libgl1-mesa-dri libegl-mesa0 \
    libinput-tools \
    fonts-dejavu-core \
    xwayland || true

# ── Network + hardware ──
# iproute2 and iputils-ping: a machine that cannot run `ip addr` or `ping` cannot be debugged by
# the person sitting at it, and the mind's network tools were found reporting a missing `ping`
# as an unreachable internet. The tools no longer need either; the person still does.
apt-get install -y -qq \
    network-manager \
    wpasupplicant \
    iproute2 iputils-ping \
    pciutils usbutils || true

# ── Firmware (non-free, for real hardware) ──
for pkg in firmware-linux-free firmware-misc-nonfree firmware-realtek \
           firmware-iwlwifi firmware-amd-graphics; do
    apt-get install -y -qq "$pkg" 2>/dev/null || true
done

# ── VirtualBox guest support ──
apt-get install -y -qq virtualbox-guest-utils 2>/dev/null || true

# ── Yantrik UI runtime dependencies ──
apt-get install -y -qq \
    speech-dispatcher libspeechd2 \
    2>/dev/null || true

# ── The desktop's own runtime ──
# What a machine built from cloud-init/user-data.yaml gets, because the same shell runs on both.
# The ISO installed a handful of these and a different browser, so an installed machine had a
# Browser pin that ran `chromium` and found nothing, no notification daemon, no portals, no
# screenshots — each failing quietly. No `|| true`: a desktop missing its runtime is a failed
# build, and the parity check after this step names anything user-data.yaml gains later.
apt-get install -y -qq     seatd     libcap2-bin     chromium     pipewire-pulse wireplumber pulseaudio-utils     python3-websocket     fontconfig     grim slurp     qemu-guest-agent     libnotify-bin     xdg-desktop-portal xdg-desktop-portal-wlr     lxpolkit     udisks2     brightnessctl     bluez alsa-utils

# Three programs the shell shells out to by name, and did not have.
#   swaybg     — yantrik-companion-tools/src/wallpaper.rs: setting a wallpaper did nothing
#   xdg-utils  — mime_dispatch.rs and lens.rs call xdg-open to hand a file to its app
#   espeak-ng  — the voice fallback when Piper is not the chosen engine
# Each failed silently, which is why none of them was noticed from inside the desktop.
apt-get install -y -qq     swaybg     xdg-utils     espeak-ng

# ── Utilities (installer essentials) ──
apt-get install -y -qq \
    jq parted rsync openssh-server openssl \
    dosfstools e2fsprogs grub-efi-amd64-bin grub-pc-bin \
    libpam-modules initramfs-tools || true

# ── Calamares installer ──
apt-get install -y -qq \
    2>/dev/null || {
        echo "Calamares not in repos — will use text installer"
    }

# ── Locale ──
echo "en_US.UTF-8 UTF-8" > /etc/locale.gen
locale-gen
update-locale LANG=en_US.UTF-8

# ── Regenerate initramfs with live-boot hooks ──
# CRITICAL: live-boot was installed after the kernel, so the initrd
# doesn't contain the live-boot hooks yet. Without this, the ISO
# will kernel panic (can't find root) and reboot loop.
update-initramfs -u

# ── Clean apt cache but keep .deb files for offline install ──
# We keep /var/cache/apt/archives/ populated so the installed system
# can reinstall packages without internet
apt-get clean

echo "Package installation complete"
CHROOT_PACKAGES

ok "All packages installed"

# The ISO and a cloud-init machine run the same shell, so they need the same packages. The two
# lists drifted — chromium, the notification daemon, the portals and a dozen more were only in
# user-data.yaml — and nothing noticed until a person clicked Browser on an installed machine.
# Every package cloud-init installs must be installed here.
CLOUD_INIT_PACKAGES=$(awk '
    /^packages:/ { inlist = 1; next }
    inlist && /^[^ #]/ { inlist = 0 }
    inlist && /^  - / { sub(/^  - /, ""); sub(/[ 	]*#.*/, ""); print }
' "$SCRIPT_DIR/cloud-init/user-data.yaml")
[ -n "$CLOUD_INIT_PACKAGES" ] || fail "read no packages from cloud-init/user-data.yaml"
MISSING_PACKAGES=""
for pkg in $CLOUD_INIT_PACKAGES; do
    status=$(sudo chroot "$ROOTFS" dpkg-query -W -f='${db:Status-Status}' "$pkg" 2>/dev/null || true)
    [ "$status" = "installed" ] || MISSING_PACKAGES="$MISSING_PACKAGES $pkg"
done
[ -z "$MISSING_PACKAGES" ]     || fail "installed by cloud-init but not in the ISO:$MISSING_PACKAGES"
ok "Every package cloud-init installs is in the image ($(echo $CLOUD_INIT_PACKAGES | wc -w))"

# ═══════════════════════════════════════════════════════════════
# STEP 3: Create yantrik user + directory structure
# ═══════════════════════════════════════════════════════════════
step "[3/10] Creating user and directories..."

sudo chroot "$ROOTFS" /bin/bash <<'CHROOT_USER'
set -e

# Create yantrik user
useradd -m -s /bin/bash -G sudo,video,audio,input yantrik
echo "yantrik:yantrik" | chpasswd
# Root is locked. It used to be root:root with SSH root login allowed, and the installer copies
# this filesystem, so every installed machine accepted that login from the network.
passwd -l root

# Passwordless sudo for yantrik
echo "yantrik ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/yantrik
chmod 440 /etc/sudoers.d/yantrik

# Directory structure
mkdir -p /opt/yantrik/{bin,data,logs,models/{embedder,whisper,llm,tts},skills,i18n}

# Hostname
echo "yantrik" > /etc/hostname
cat > /etc/hosts <<HOSTS
127.0.0.1   localhost
127.0.1.1   yantrik
HOSTS
CHROOT_USER

ok "User yantrik created"

# ═══════════════════════════════════════════════════════════════
# STEP 4: Install Yantrik binaries
# ═══════════════════════════════════════════════════════════════
step "[4/10] Installing Yantrik OS (every binary, discovered)..."

# Package the workspace unless a prebuilt artifact was handed to us. --no-build
# because this script is not the thing that decides when to compile; it installs
# what has already been built.
if [ -z "$RELEASE_TARBALL" ]; then
    # Handed the SAME directory this script checked for yantrik-ui, spelled the way
    # build-release.sh spells it (it takes the release dir itself, not its parent). Without
    # this the two scripts resolve the target directory independently and the check above can
    # pass against one build while the tarball is packed from another.
    TARGET_DIR="$TARGET_DIR/release" "$SCRIPT_DIR/build-release.sh" --no-build --out "$WORK_DIR/dist" \
        || fail "build-release.sh failed — cannot determine what the OS contains"
    RELEASE_TARBALL="$(ls -t "$WORK_DIR/dist"/yantrik-os-*.tar.zst 2>/dev/null | head -1)"
    [ -n "$RELEASE_TARBALL" ] || fail "build-release.sh produced no tarball"
fi

UNPACK="$WORK_DIR/release-unpack"
rm -rf "$UNPACK"; mkdir -p "$UNPACK"
tar --zstd -xf "$RELEASE_TARBALL" -C "$UNPACK" --strip-components=1 \
    || fail "could not unpack $RELEASE_TARBALL"

sudo mkdir -p "$ROOTFS/opt/yantrik/bin" "$ROOTFS/opt/yantrik/models"
sudo cp -a "$UNPACK/bin/." "$ROOTFS/opt/yantrik/bin/"
sudo chmod +x "$ROOTFS/opt/yantrik/bin/"*

# libcap2-bin is installed in the rootfs above so the updater can re-apply this after a swap;
# the image tested on 2026-09-23 had no setcap at all, and the grant silently did not happen.
# perception-service is the one program here that needs privilege, and only for two calls at
# startup: `fanotify_init` (CAP_SYS_ADMIN) and joining the process connector (CAP_NET_ADMIN).
# It then applies Landlock and drops every capability, irreversibly - main.rs explains the
# ordering and why each step is load-bearing. Nothing granted them: the shell starts it as the
# user, so `fanotify_init` failed and the service came up on PSI alone, and `os_perception`
# answered fourteen requests with one apology. File capabilities on the binary are the grant:
# the kernel raises them on exec whoever runs it, the service hands them back before it serves,
# and no other binary in bin/ carries any. mksquashfs keeps xattrs and yantrik-install.sh copies
# with rsync -X, so they survive into the image and onto the disk.
sudo setcap cap_sys_admin,cap_net_admin=ep "$ROOTFS/opt/yantrik/bin/perception-service" \
    || fail "setcap failed on perception-service"
sudo getcap "$ROOTFS/opt/yantrik/bin/perception-service" | grep -q cap_sys_admin \
    || fail "perception-service carries no file capabilities after setcap - the image would ship it blind"

# The session's own furniture: compositor config, theme, fonts, desktop entries. `yantrik-session`
# installs these into the person's session at every login. The image used to leave them out and
# start labwc with a hand-written config instead, so an installed machine drew the shell inside a
# window with a title bar while every cloud-init machine did not.
[ -d "$UNPACK/share" ] || fail "release tarball carries no share/ — the session would have no compositor config"
sudo mkdir -p "$ROOTFS/opt/yantrik/share"
sudo cp -a "$UNPACK/share/." "$ROOTFS/opt/yantrik/share/"
sudo chown -R root:root "$ROOTFS/opt/yantrik/share"
for required in share/labwc/rc.xml share/labwc/autostart bin/yantrik-session \
                share/icons/hicolor/scalable/apps/yantrik.svg; do
    [ -e "$ROOTFS/opt/yantrik/$required" ] || fail "$required missing from the image — the desktop session would not be the shipped one"
done

# ── The app icon, also in the system icon theme ──
#
# It already landed under /opt/yantrik/share above, which is where the Yantrik session finds
# it because yantrik-session puts that directory on XDG_DATA_DIRS. But this image also runs
# a live installer, a polkit agent and whatever else the person launches, none of which get
# that environment — and the .desktop entries this OS ships say `Icon=yantrik` everywhere.
# So the same files go into the ordinary system theme as well, where every toolkit looks
# without being told to. Two copies of one 1.7 KB file and three small PNGs, versus a
# launcher full of blank tiles in half the contexts on the machine.
sudo mkdir -p "$ROOTFS/usr/share/icons/hicolor/scalable/apps"
sudo cp "$ROOTFS/opt/yantrik/share/icons/hicolor/scalable/apps/yantrik.svg" \
        "$ROOTFS/usr/share/icons/hicolor/scalable/apps/yantrik.svg"
for px in 48 128 256; do
    src="$ROOTFS/opt/yantrik/share/icons/hicolor/${px}x${px}/apps/yantrik.png"
    if [ -f "$src" ]; then
        sudo mkdir -p "$ROOTFS/usr/share/icons/hicolor/${px}x${px}/apps"
        sudo cp "$src" "$ROOTFS/usr/share/icons/hicolor/${px}x${px}/apps/yantrik.png"
    fi
done
# Refresh the theme cache if the tool is in the chroot; harmless and skipped if not.
sudo chroot "$ROOTFS" gtk-update-icon-cache -f -t /usr/share/icons/hicolor >/dev/null 2>&1 || true

# The build manifest travels with the image so a running machine can answer
# "which build is this?" — a question that was unanswerable on the VM all day.
# Written as `if`, not `[ ... ] && cp`: under `set -e` a false test is the last
# command in that compound and would abort the whole build silently.
if [ -f "$UNPACK/BUILD" ]; then
    sudo cp "$UNPACK/BUILD" "$ROOTFS/opt/yantrik/BUILD"
else
    warn "release tarball carries no BUILD manifest — the image will not be able to say which build it is"
fi
if [ -f "$UNPACK/config.yaml" ]; then
    sudo cp "$UNPACK/config.yaml" "$ROOTFS/opt/yantrik/config.yaml"
else
    warn "release tarball carries no config.yaml — the machine will need one before it can talk to a model"
fi
if [ -d "$UNPACK/models" ] && [ -n "$(ls -A "$UNPACK/models" 2>/dev/null)" ]; then
    sudo cp -a "$UNPACK/models/." "$ROOTFS/opt/yantrik/models/"
fi

# A desktop with no apps behind it is the failure this whole change exists to stop,
# so assert the shape of what landed rather than trusting the copy.
INSTALLED=$(ls "$ROOTFS/opt/yantrik/bin" | wc -l)
[ "$INSTALLED" -ge 20 ] || fail "only $INSTALLED binaries landed in the image — expected the full set"
for required in yantrik-ui yantrik yantrik-notes weather-service yos; do
    [ -e "$ROOTFS/opt/yantrik/bin/$required" ] \
        || fail "$required missing from the image — the ISO would boot without it"
done

# Copy i18n files if they exist
if [ -d "$PROJECT_ROOT/crates/yantrik-ui/i18n" ]; then
    sudo cp -r "$PROJECT_ROOT/crates/yantrik-ui/i18n/"* "$ROOTFS/opt/yantrik/i18n/" 2>/dev/null || true
fi

# Copy skill manifests if they exist
if [ -d "$PROJECT_ROOT/skills" ]; then
    sudo cp -r "$PROJECT_ROOT/skills/"* "$ROOTFS/opt/yantrik/skills/" 2>/dev/null || true
fi

ok "Installed $INSTALLED binaries ($(sudo du -sh "$ROOTFS/opt/yantrik/bin" | cut -f1))"

# ── What this image redistributes that we did not write ──
#
# The embedder, Whisper, Piper and its voice, the fonts, and a whole Debian system. Several of
# those licences require their terms to travel with the copy, and publishing an ISO is making
# copies. Required, not best-effort: an image with no attribution file is an image that cannot
# be published, and discovering that at upload time is discovering it too late.
[ -f "$SCRIPT_DIR/THIRD-PARTY-NOTICES.md" ] \
    || fail "no THIRD-PARTY-NOTICES.md beside this script — this image redistributes other people's software and must say so"
sudo cp "$SCRIPT_DIR/THIRD-PARTY-NOTICES.md" "$ROOTFS/opt/yantrik/THIRD-PARTY-NOTICES.md"
if [ -f "$PROJECT_ROOT/LICENSE" ]; then
    sudo cp "$PROJECT_ROOT/LICENSE" "$ROOTFS/opt/yantrik/LICENSE"
    ok "Licence and third-party notices installed"
else
    warn "the repository has no LICENSE file, so the image ships none — by default that is"
    warn "all rights reserved, and nobody who downloads the ISO may redistribute it"
fi

# ── The Hermes plugin ──
#
# Not a Yantrik binary and not started by anything here: it is the adapter that lets an
# existing Hermes install attach to this desktop's harness socket, so it ships as source
# beside the OS rather than being installed into a Python environment the image does not have.
# Hermes keeps its own model, keys and memory; nothing about them is in this image.
if [ -d "$PROJECT_ROOT/harnesses/hermes" ]; then
    sudo mkdir -p "$ROOTFS/opt/yantrik/share/harnesses/hermes"
    # harness.yaml goes with it: it is what Settings > Harnesses reads to say Hermes exists at
    # all, and without it the desktop is back to only knowing what has attached.
    sudo cp "$PROJECT_ROOT/harnesses/hermes/"*.py "$PROJECT_ROOT/harnesses/hermes/plugin.yaml" \
        "$PROJECT_ROOT/harnesses/hermes/harness.yaml" \
        "$ROOTFS/opt/yantrik/share/harnesses/hermes/" 2>/dev/null || true
    ok "Hermes desktop plugin staged at /opt/yantrik/share/harnesses/hermes"
fi

# ── The other harnesses ──
#
# Same rule as Hermes and for the same reason: source beside the OS, nothing installed into a
# Python environment the image does not have, and NOTHING ENABLED. Each one needs an endpoint,
# a model and a key that only the person has, and an image that started one of these on its own
# would be an image that phoned a provider the first time it booted.
#
# `lib` is the half they share (attach, poll, heartbeat, the MCP client, one place a turn is
# closed); `deepseek` is a tool-calling loop over an OpenAI-compatible API; `pi` drives the pi
# coding agent over its RPC mode and carries the extension that gives it the desktop's tools;
# `openclaw` drives an OpenClaw install, which brings its own MCP client and so reaches the
# desktop's tools through an entry in the person's own ~/.openclaw/openclaw.json rather than
# through anything staged here.
#
# Each one's unit file is staged TWICE and the second copy is the one that matters. Beside its
# source, where a person following the README copies it from — and in /etc/systemd/user, where
# systemd itself looks. Without the second, `systemctl --user enable --now yantrik-pi` fails
# with "unit not found" and the Start button on the Harnesses page has nothing to start: the
# image shipped a harness whose own instructions were the only way to run it. Staging a unit is
# not enabling it — `enable` writes a symlink into default.target.wants and nothing here does,
# so the rule above still holds and a fresh image still starts none of them.
sudo mkdir -p "$ROOTFS/etc/systemd/user"
for harness in lib deepseek pi openclaw; do
    [ -d "$PROJECT_ROOT/harnesses/$harness" ] || continue
    sudo mkdir -p "$ROOTFS/opt/yantrik/share/harnesses/$harness"
    sudo cp -r "$PROJECT_ROOT/harnesses/$harness/." \
        "$ROOTFS/opt/yantrik/share/harnesses/$harness/" 2>/dev/null || true
    # __pycache__ from someone's checkout is not part of the image.
    sudo rm -rf "$ROOTFS/opt/yantrik/share/harnesses/$harness/__pycache__"
    unit="$PROJECT_ROOT/harnesses/$harness/yantrik-$harness.service"
    if [ -f "$unit" ]; then
        sudo install -m 644 "$unit" "$ROOTFS/etc/systemd/user/" \
            || fail "could not stage yantrik-$harness.service — the image would ship a harness nothing can start"
        ok "Harness staged at /opt/yantrik/share/harnesses/$harness, unit installed (not enabled)"
    else
        ok "Harness source staged at /opt/yantrik/share/harnesses/$harness (no unit of its own)"
    fi
done

# The manifests are what Settings > Harnesses reads, so an image that staged the source and not
# the manifests is an image whose picker is back to listing only what has attached. Asserted
# rather than hoped for: this is the whole of issue #56 and it is one missing file away.
for harness in hermes deepseek pi openclaw; do
    [ -d "$PROJECT_ROOT/harnesses/$harness" ] || continue
    [ -f "$ROOTFS/opt/yantrik/share/harnesses/$harness/harness.yaml" ] \
        || fail "$harness has no harness.yaml in the image — Settings would not know it exists"
done
ok "Harness manifests staged — Settings can name every mind this image ships"

# ── The mind, beside the OS ──
#
# Installed and enabled, never configured: the OS holds no model, endpoint or key for
# it. Its user unit reads the mind's own settings file, and with no model in it the mind
# asks for one in the desktop chat. Enabled globally (/etc/systemd/user), so it runs for
# whichever user the installer creates, not only for `yantrik`.
if [ "${YANTRIK_ISO_WITHOUT_MIND:-0}" = "1" ]; then
    warn "Building WITHOUT a mind (YANTRIK_ISO_WITHOUT_MIND=1) — the desktop will have only its builtin companion"
else
    [ -n "$MIND_TARBALL" ] \
        || fail "MIND_TARBALL is not set. Build one with yantrik-mind's deploy/yantrik-os/package.sh, or set YANTRIK_ISO_WITHOUT_MIND=1 to build a body without a mind on purpose"
    [ -f "$MIND_TARBALL" ] || fail "MIND_TARBALL not found: $MIND_TARBALL"
    MIND_UNPACK="$WORK_DIR/mind-unpack"
    rm -rf "$MIND_UNPACK"; mkdir -p "$MIND_UNPACK"
    tar --zstd -xf "$MIND_TARBALL" -C "$MIND_UNPACK" --strip-components=1 \
        || fail "could not unpack $MIND_TARBALL"

    sudo mkdir -p "$ROOTFS/opt/yantrik-mind/bin" "$ROOTFS/etc/systemd/user"
    sudo cp -a "$MIND_UNPACK/bin/." "$ROOTFS/opt/yantrik-mind/bin/"
    sudo chmod 755 "$ROOTFS/opt/yantrik-mind/bin/"*
    if [ -f "$MIND_UNPACK/BUILD" ]; then
        sudo cp "$MIND_UNPACK/BUILD" "$ROOTFS/opt/yantrik-mind/BUILD"
    else
        warn "mind bundle carries no BUILD manifest — the image will not be able to say which mind it carries"
    fi
    sudo cp "$MIND_UNPACK/systemd/user/"*.service "$ROOTFS/etc/systemd/user/"
    # Owned by root: `cp -a` keeps the build user's uid, which on the installed machine is the
    # person's own uid — leaving the mind's binaries writable by every process they run.
    sudo chown -R root:root "$ROOTFS/opt/yantrik-mind"
    sudo chown root:root "$ROOTFS/etc/systemd/user/yantrik-mind.service" "$ROOTFS/etc/systemd/user/yantrik-memory.service"
    sudo chmod 644 "$ROOTFS/etc/systemd/user/yantrik-mind.service" "$ROOTFS/etc/systemd/user/yantrik-memory.service"
    sudo chroot "$ROOTFS" systemctl --global enable yantrik-mind.service \
        || fail "could not enable yantrik-mind.service for users"

    for required in yantrik-mind yantrik-memory; do
        [ -x "$ROOTFS/opt/yantrik-mind/bin/$required" ] \
            || fail "$required missing from the image — the ISO would boot without its mind"
    done
    [ -L "$ROOTFS/etc/systemd/user/default.target.wants/yantrik-mind.service" ] \
        || fail "yantrik-mind.service is not enabled in the image"
    [ ! -e "$ROOTFS/etc/systemd/user/default.target.wants/yantrik-memory.service" ] \
        || fail "yantrik-memory.service is enabled — it would fight the mind for the memory file"
    ok "Yantrik Mind $(sed -n 's/^commit=//p' "$ROOTFS/opt/yantrik-mind/BUILD" 2>/dev/null) installed beside the OS, enabled for every user"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 5: Download AI models (baked into ISO)
# ═══════════════════════════════════════════════════════════════
step "[5/10] Downloading AI models (baked into ISO)..."

# ── MiniLM embedder (~22MB) — ALWAYS included ──
# Essential for CognitiveRouter (Core Mode). Without this, the OS
# can't route queries to tools/recipes and needs an external LLM for everything.
if [ -z "${NO_EMBEDDER:-}" ]; then
    EMB_DIR="$ROOTFS/opt/yantrik/models/embedder"
    HF_EMB="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main"
    info "Embedder model (~22MB) — required for Core Mode..."
    for f in config.json tokenizer.json tokenizer_config.json special_tokens_map.json model.safetensors; do
        if [ ! -f "$EMB_DIR/$f" ]; then
            sudo wget -q -O "$EMB_DIR/$f" "$HF_EMB/$f"
        fi
    done
    ok "Embedder model (CognitiveRouter enabled — 245 tools, 50 recipes)"
else
    warn "Embedder skipped (--no-embedder) — Core Mode will NOT work"
fi

# (Whisper is now always included above)

# ── Piper TTS (~66MB) — ALWAYS included for voice ──
# Natural-sounding voice synthesis via Piper (binary + voice model)
TTS_DIR="$ROOTFS/opt/yantrik/models/tts"
PIPER_URL="https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_x86_64.tar.gz"
PIPER_VOICE_URL="https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/medium"
if [ ! -f "$TTS_DIR/piper" ]; then
    info "Piper TTS binary + voice model (~66MB)..."
    # Staged inside the build dir, not /tmp. `sudo wget` left a root-owned file in
    # a sticky /tmp and the unprivileged `rm` below could not remove it — under
    # `set -e` that aborted the whole ISO build at step 5 of 10, after debootstrap
    # and every apt install had already run.
    PIPER_TMP="$WORK_DIR/piper-stage"
    sudo rm -rf "$PIPER_TMP"
    sudo mkdir -p "$PIPER_TMP"
    sudo wget -q "$PIPER_URL" -O "$PIPER_TMP/piper.tar.gz"
    sudo tar xzf "$PIPER_TMP/piper.tar.gz" -C "$PIPER_TMP"
    sudo cp "$PIPER_TMP/piper/piper" "$TTS_DIR/"
    sudo cp "$PIPER_TMP/piper/lib"*.so* "$TTS_DIR/" 2>/dev/null || true
    sudo cp -r "$PIPER_TMP/piper/espeak-ng-data" "$TTS_DIR/"
    sudo chmod +x "$TTS_DIR/piper"
    sudo rm -rf "$PIPER_TMP"
    ok "Piper binary installed"
else
    ok "Piper binary (cached)"
fi
if [ ! -f "$TTS_DIR/en_US-lessac-medium.onnx" ]; then
    sudo wget -q "$PIPER_VOICE_URL/en_US-lessac-medium.onnx" -O "$TTS_DIR/en_US-lessac-medium.onnx"
    sudo wget -q "$PIPER_VOICE_URL/en_US-lessac-medium.onnx.json" -O "$TTS_DIR/en_US-lessac-medium.onnx.json"
    ok "Piper voice model (en_US-lessac-medium)"
else
    ok "Piper voice model (cached)"
fi

# ── Whisper STT (~75MB) — ALWAYS included for voice ──
WHISPER_DIR="$ROOTFS/opt/yantrik/models/whisper"
HF_WHISPER="https://huggingface.co/openai/whisper-tiny/resolve/main"
info "Whisper STT model (~75MB)..."
for f in config.json tokenizer.json model.safetensors; do
    if [ ! -f "$WHISPER_DIR/$f" ]; then
        sudo wget -q -O "$WHISPER_DIR/$f" "$HF_WHISPER/$f"
    fi
done
ok "Whisper STT model"

# ── Offline LLM (~2.6GB) — optional, for Enhanced Mode ──
if $INCLUDE_LLM; then
    LLM_DIR="$ROOTFS/opt/yantrik/models/llm"
    LLM_GGUF="yantrik-4b.gguf"
    # A local GGUF, if the builder points at one. LOCAL_GGUF=/path/to/model.gguf.
    #
    # This used to be one developer's Windows home directory and one Ollama blob hash, written
    # into the script: on their machine the ISO silently shipped a fine-tuned model, on every
    # other machine it silently shipped a different one, and the image could not say which.
    OLLAMA_BLOB="${LOCAL_GGUF:-}"
    if [ -n "$OLLAMA_BLOB" ] && [ -f "$OLLAMA_BLOB" ]; then
        info "Copying local GGUF from $OLLAMA_BLOB ..."
        sudo cp "$OLLAMA_BLOB" "$LLM_DIR/$LLM_GGUF"
        ok "Local GGUF installed (Enhanced Mode) — sha256 $(sha256sum "$OLLAMA_BLOB" | cut -c1-16)…"
    elif [ ! -f "$LLM_DIR/$LLM_GGUF" ]; then
        info "Fine-tuned model not found, downloading base Qwen3.5-4B (~2.5GB)..."
        sudo wget -q --show-progress -O "$LLM_DIR/$LLM_GGUF" \
            "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf"
        ok "Base Qwen3.5-4B (Enhanced Mode fallback)"
    fi
else
    info "LLM skipped (use --with-llm for offline Enhanced Mode)"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 6: Generate default config
# ═══════════════════════════════════════════════════════════════
step "[6/10] Generating default configuration..."

# The config a published image ships used to be a 130-line heredoc right here, which meant it
# could only be audited by reading a shell script — and that it silently overwrote the config
# the release tarball had just installed two steps ago, so the image ran on a config no other
# install path had. It is a file now: deploy/yantrik-os/config-default.yaml, greppable for the
# addresses, names and keys a public image must not carry.
#
# It still overwrites what the tarball brought, and that is deliberate: the tarball carries
# config/yantrik-ollama.yaml, the author's dev config, which names a private LAN address as the
# model endpoint and the author by name. Correct for the machine it was written for; wrong for
# every machine that boots this ISO.
[ -f "$SCRIPT_DIR/config-default.yaml" ]     || fail "no config-default.yaml beside this script — the image would ship the dev config"

# Refuse to publish somebody's private network. Cheap, and it is the check that was missing
# when the dev config was the one being shipped.
CONFIG_LEAKS=$(grep -nE '192\.168\.|10\.[0-9]+\.[0-9]+\.[0-9]+|172\.(1[6-9]|2[0-9]|3[01])\.|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}'     "$SCRIPT_DIR/config-default.yaml" || true)
[ -z "$CONFIG_LEAKS" ]     || fail "config-default.yaml carries a private address or an e-mail address:
$CONFIG_LEAKS"

sudo cp "$SCRIPT_DIR/config-default.yaml" "$ROOTFS/opt/yantrik/config.yaml"
sudo chmod 644 "$ROOTFS/opt/yantrik/config.yaml"

# One answer to "which channel is this machine on", and one file that holds it.
#
# This used to read the channel out of config-default.yaml's `updates.channel`, which said
# `beta`. Nothing on the machine ever read that key — not the desktop, not the updater, nothing
# — but THIS line read it, so the first public nightly ISO was built to take its updates from
# beta: a channel whose newest build was six months older than the image being written. The
# config no longer carries an `updates:` section at all, so the channel is decided here, from
# this script's own default and the pipeline's override, and written to the one file that owns
# it: /opt/yantrik/update.conf.
#
# nightly, because it is the only channel that has ever had a build published to it. When
# stable starts receiving builds, change this line and the matching CHANNEL_DEFAULT in
# deploy/yantrik-os/yantrik-update — those two are the whole list.
UPDATE_CHANNEL="${YANTRIK_UPDATE_CHANNEL:-nightly}"
case "$UPDATE_CHANNEL" in
    nightly|beta|stable) ;;
    *) fail "YANTRIK_UPDATE_CHANNEL must be nightly, beta or stable (got '$UPDATE_CHANNEL')" ;;
esac
sudo tee "$ROOTFS/opt/yantrik/update.conf" > /dev/null <<UPDATECONF
# Read by yantrik-update, and by nothing else. This file is the single owner of which channel
# this machine follows, which server it follows it on, and over which scheme.
#
# Change it with: yantrik-update set-channel nightly|beta|stable
# or from the desktop: About -> UPDATES -> the channel chips.
CHANNEL=$UPDATE_CHANNEL
HOST=releases.yantrikos.com
SCHEME=https
UPDATECONF

ok "Default config installed from config-default.yaml (loopback model endpoint, no name, no key)"
ok "Update channel: $UPDATE_CHANNEL via https://releases.yantrikos.com"

# ═══════════════════════════════════════════════════════════════
# STEP 7: Configure desktop session (labwc + auto-login)
# ═══════════════════════════════════════════════════════════════
step "[7/10] Configuring desktop session..."

# ── labwc config ──
LABWC_DIR="$ROOTFS/home/yantrik/.config/labwc"
sudo mkdir -p "$LABWC_DIR"

# Environment — and no renderer in it.
#
# This used to set WLR_RENDERER=pixman and LIBGL_ALWAYS_SOFTWARE=1 for every machine, "so it
# always works", and it did: by drawing every desktop on the CPU, a laptop with an Intel, AMD or
# NVIDIA GPU included. yantrik-session decides GPU or software at every login now, with a probe,
# a list of combinations known to be broken, and a fallback to software when the GPU fails in use
# (see "Graphics" in deploy/yantrik-os/yantrik-session). The first line is its mark: a file
# without it is an older image's, and the session strips that image's software lines from it.
# The mark must match GRAPHICS_ENV_MARK in yantrik-session, byte for byte.
sudo tee "$LABWC_DIR/environment" > /dev/null <<'ENV'
# yantrik-graphics: yantrik-session decides the renderer
# yantrik-session chooses the GPU or software at every login; `yantrik-session graphics` says
# what it would choose and why. To force one, add a line: YANTRIK_GRAPHICS=software or
# YANTRIK_GRAPHICS=gpu. WLR_RENDERER (labwc's renderer) and SLINT_BACKEND (the shell's) are
# honoured as written here.
WLR_NO_HARDWARE_CURSORS=1
XDG_SESSION_TYPE=wayland
QT_QPA_PLATFORM=wayland
MOZ_ENABLE_WAYLAND=1
ENV
MARK_IN_SESSION="$(sed -n "s/^GRAPHICS_ENV_MARK='\(.*\)'$/\1/p" "$ROOTFS/opt/yantrik/bin/yantrik-session")"
[ "$(sudo head -n 1 "$LABWC_DIR/environment")" = "$MARK_IN_SESSION" ] \
    || fail "the labwc environment's first line is not yantrik-session's GRAPHICS_ENV_MARK ('$MARK_IN_SESSION') — the session would strip it as an old image's"

# Autostart — Yantrik is the shell
sudo tee "$LABWC_DIR/autostart" > /dev/null <<'AUTOSTART'
#!/bin/sh
# No notification daemon is started here.
#
# mako used to be, and it raced the desktop for org.freedesktop.Notifications: whichever won
# the name got every notify-send on the machine, and if it was mako the shell's notification
# centre stayed empty. The notifications service owns that name now and is the one store.
# libnotify-bin is still installed — that is the notify-send client, not a daemon.

# Start Yantrik OS as the desktop shell
/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
AUTOSTART
sudo chmod +x "$LABWC_DIR/autostart"

# Window rules — Yantrik fullscreen, no decorations
sudo tee "$LABWC_DIR/rc.xml" > /dev/null <<'RCXML'
<?xml version="1.0" encoding="UTF-8"?>
<labwc_config>
  <core><gap>0</gap></core>
  <!-- Apps are real windows and get real title bars; the shell removes its own below. -->
  <theme><titlebar><height>28</height></titlebar></theme>
  <keyboard>
    <keybind key="A-Tab"><action name="NextWindow" /></keybind>
    <keybind key="A-F4"><action name="Close" /></keybind>
    <keybind key="W-t">
      <action name="Execute"><command>foot</command></action>
    </keybind>
    <keybind key="Print">
      <action name="Execute">
        <command>sh -c 'mkdir -p ~/Pictures/Screenshots &amp;&amp; grim ~/Pictures/Screenshots/$(date +%Y%m%d_%H%M%S).png'</command>
      </action>
    </keybind>
  </keyboard>
  <windowRules>
    <!-- Exact title: the shell only. "Yantrik*" also caught every app titled "Yantrik ..." -->
    <windowRule title="Yantrik OS">
      <action name="ToggleDecorations" />
      <action name="ToggleFullscreen" />
    </windowRule>
  </windowRules>
</labwc_config>
RCXML

# ── foot terminal config ──
FOOT_DIR="$ROOTFS/home/yantrik/.config/foot"
sudo mkdir -p "$FOOT_DIR"
sudo tee "$FOOT_DIR/foot.ini" > /dev/null <<'FOOTINI'
[main]
font=DejaVu Sans Mono:size=11
pad=8x4
dpi-aware=no

[scrollback]
lines=10000

[cursor]
style=beam
blink=yes

[colors]
background=0c0b10
foreground=c8c8d0
regular0=1a1a2e
regular1=e86b6b
regular2=5ac8a0
regular3=d4a04a
regular4=5ac8d4
regular5=a87bd4
regular6=5ac8d4
regular7=c8c8d0
bright0=2e2e48
bright1=f09090
bright2=7ee0c0
bright3=e0c070
bright4=80d8e8
bright5=c0a0e0
bright6=80d8e8
bright7=e0e0e8
FOOTINI

# No mako config is written.
#
# It used to be: a themed popup style for a daemon we no longer start. Leaving the file behind
# would be an instruction for a program that is not installed, and the next person to read it
# would reasonably conclude mako was meant to be running.

# Fix ownership
sudo chown -R 1000:1000 "$ROOTFS/home/yantrik"
sudo chown -R 1000:1000 "$ROOTFS/opt/yantrik"

ok "labwc + foot configured"

# ── Auto-login via systemd ──
# Override getty@tty1 to auto-login as yantrik and start labwc
sudo mkdir -p "$ROOTFS/etc/systemd/system/getty@tty1.service.d"
sudo tee "$ROOTFS/etc/systemd/system/getty@tty1.service.d/autologin.conf" > /dev/null <<'AUTOLOGIN'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin yantrik --noclear %I $TERM
AUTOLOGIN

# .bash_profile — auto-start labwc on tty1, with installer mode + crash guard
sudo tee "$ROOTFS/home/yantrik/.bash_profile" > /dev/null <<'PROFILE'
# Auto-start Yantrik desktop on tty1
if [ "$(tty)" = "/dev/tty1" ] && [ -z "$WAYLAND_DISPLAY" ]; then
    export XDG_RUNTIME_DIR="/run/user/$(id -u)"
    mkdir -p "$XDG_RUNTIME_DIR"

    # Nothing about graphics here. yantrik-session decides GPU or software at every login --
    # the Safe Mode entry's nomodeset included -- and labwc reads ~/.config/labwc/environment
    # itself, before it chooses a renderer. This used to source that file to force pixman.

    # Check if installer mode was requested via kernel param
    if grep -q 'yantrik.install=true' /proc/cmdline 2>/dev/null; then
        # Create marker so yantrik-ui shows disk install fields in onboarding
        touch /opt/yantrik/.installer-mode
    fi

    # Crash guard: if labwc crashed recently, don't loop — drop to shell
    CRASH_FILE="/tmp/.yantrik-labwc-crash"
    if [ -f "$CRASH_FILE" ]; then
        LAST_CRASH=$(cat "$CRASH_FILE" 2>/dev/null || echo 0)
        NOW=$(date +%s)
        # If crashed less than 10 seconds ago, stop looping
        if [ $((NOW - LAST_CRASH)) -lt 10 ]; then
            echo
            echo "============================================="
            echo "  Yantrik OS — Desktop failed to start"
            echo "============================================="
            echo
            echo "  labwc (Wayland compositor) crashed on startup."
            echo "  Common fixes:"
            echo "    1. Reboot and select 'Safe Mode' from the menu"
            echo "    2. Check GPU: lspci | grep -i vga"
            echo "    3. Force software: echo YANTRIK_GRAPHICS=software >> ~/.config/labwc/environment"
            echo "    4. View logs: cat /opt/yantrik/logs/labwc.log"
            echo
            exec /bin/bash --login
        fi
    fi

    # Start the session every Yantrik machine runs (shipped compositor config, fullscreen shell),
    # and record a crash timestamp if it exits quickly
    START_TIME=$(date +%s)
    /opt/yantrik/bin/yantrik-session 2>>/opt/yantrik/logs/labwc.log
    EXIT_TIME=$(date +%s)

    # If labwc ran less than 5 seconds, it probably crashed
    if [ $((EXIT_TIME - START_TIME)) -lt 5 ]; then
        echo "$EXIT_TIME" > "$CRASH_FILE"
    else
        rm -f "$CRASH_FILE"
        # A session that ran and then ended (labwc crashed hours in, or was killed) ends the
        # login, so tty1's autologin starts the desktop again. Left to fall through, it left a
        # bash prompt on tty1 and no desktop until a reboot. A quick crash still falls through to
        # the prompt, as it always has.
        exit 0
    fi
fi
PROFILE
sudo chown 1000:1000 "$ROOTFS/home/yantrik/.bash_profile"

# Ensure XDG runtime dir exists on boot
sudo tee "$ROOTFS/etc/tmpfiles.d/yantrik-xdg.conf" > /dev/null <<'TMPFILES'
d /run/user/1000 0700 yantrik yantrik -
TMPFILES

ok "Auto-login → labwc → Yantrik configured"

# ═══════════════════════════════════════════════════════════════
# STEP 8: Configure offline LLM server (systemd)
# ═══════════════════════════════════════════════════════════════
step "[8/10] Configuring offline LLM server..."

if $INCLUDE_LLM; then
    # ── Install pre-built llama-server variants ──
    # Cache directory contains pre-built binaries:
    #   llama-server-debian-amd64       — CPU-only (always works)
    #   llama-server-debian-amd64-cuda  — NVIDIA GPU (CUDA)
    #   llama-server-debian-amd64-rocm  — AMD GPU (ROCm)
    #   llama-server-debian-amd64-vulkan — Intel Arc / generic Vulkan
    #
    # Build these with deploy/yantrik-os/cache/build-llama-variants.sh
    CACHE_DIR="$SCRIPT_DIR/cache"
    LLAMA_DIR="$ROOTFS/usr/local/lib/llama"
    sudo mkdir -p "$LLAMA_DIR"

    # CPU variant (required — always the fallback)
    CACHED_CPU="$CACHE_DIR/llama-server-debian-amd64"
    if [ -f "$CACHED_CPU" ]; then
        sudo cp "$CACHED_CPU" "$LLAMA_DIR/llama-server-cpu"
        sudo chmod +x "$LLAMA_DIR/llama-server-cpu"
        # Default symlink to CPU
        sudo ln -sf /usr/local/lib/llama/llama-server-cpu "$ROOTFS/usr/local/bin/llama-server"
        ok "llama-server CPU (pre-built cache)"
    else
        info "No cached CPU binary — building from source..."
        sudo chroot "$ROOTFS" /bin/bash <<'CHROOT_LLAMA'
set -e
export DEBIAN_FRONTEND=noninteractive
apt-get install -y -qq build-essential cmake git
cd /tmp
git clone --depth 1 https://github.com/ggerganov/llama.cpp.git 2>/dev/null || true
if [ -d llama.cpp ]; then
    cd llama.cpp
    cmake -B build -DGGML_BLAS=OFF -DGGML_CUDA=OFF \
        -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF \
        -DLLAMA_BUILD_SERVER=ON 2>/dev/null
    cmake --build build --target llama-server -j$(nproc) 2>/dev/null && {
        mkdir -p /usr/local/lib/llama
        cp build/bin/llama-server /usr/local/lib/llama/llama-server-cpu
        ln -sf /usr/local/lib/llama/llama-server-cpu /usr/local/bin/llama-server
    } || echo "llama-server build failed"
    cd /tmp && rm -rf llama.cpp
fi
apt-get remove -y -qq build-essential cmake git
apt-get autoremove -y -qq
CHROOT_LLAMA
    fi

    # GPU variants (optional — installed alongside CPU)
    for variant in cuda rocm vulkan; do
        CACHED_GPU="$CACHE_DIR/llama-server-debian-amd64-${variant}"
        if [ -f "$CACHED_GPU" ]; then
            sudo cp "$CACHED_GPU" "$LLAMA_DIR/llama-server-${variant}"
            sudo chmod +x "$LLAMA_DIR/llama-server-${variant}"
            ok "llama-server ${variant} (pre-built cache)"
        fi
    done

    # ── GPU auto-detect script (runs at boot) ──
    sudo tee "$ROOTFS/usr/local/bin/llama-select-gpu" > /dev/null <<'GPUSELECT'
#!/bin/sh
# Detect GPU and symlink the best llama-server variant
LLAMA_DIR="/usr/local/lib/llama"
TARGET="/usr/local/bin/llama-server"

# Default to CPU
BEST="$LLAMA_DIR/llama-server-cpu"

if command -v lspci >/dev/null 2>&1; then
    GPU=$(lspci 2>/dev/null | grep -iE 'vga|3d|display' || true)

    if echo "$GPU" | grep -qi nvidia; then
        # Check for NVIDIA driver
        if command -v nvidia-smi >/dev/null 2>&1 && [ -f "$LLAMA_DIR/llama-server-cuda" ]; then
            BEST="$LLAMA_DIR/llama-server-cuda"
            echo "llama-select-gpu: NVIDIA GPU detected, using CUDA variant"
        fi
    elif echo "$GPU" | grep -qi 'amd\|radeon'; then
        if [ -f "$LLAMA_DIR/llama-server-rocm" ]; then
            BEST="$LLAMA_DIR/llama-server-rocm"
            echo "llama-select-gpu: AMD GPU detected, using ROCm variant"
        fi
    elif echo "$GPU" | grep -qi 'intel.*arc'; then
        if [ -f "$LLAMA_DIR/llama-server-vulkan" ]; then
            BEST="$LLAMA_DIR/llama-server-vulkan"
            echo "llama-select-gpu: Intel Arc detected, using Vulkan variant"
        fi
    fi
fi

if [ -f "$BEST" ]; then
    ln -sf "$BEST" "$TARGET"
    echo "llama-select-gpu: active → $(basename "$BEST")"
else
    echo "llama-select-gpu: no suitable variant found, keeping CPU"
fi
GPUSELECT
    sudo chmod +x "$ROOTFS/usr/local/bin/llama-select-gpu"

    # Run GPU detection before llama-server starts
    sudo mkdir -p "$ROOTFS/etc/systemd/system/llama-server.service.d"
    sudo tee "$ROOTFS/etc/systemd/system/llama-server.service.d/gpu-detect.conf" > /dev/null <<'GPUCONF'
[Service]
ExecStartPre=/usr/local/bin/llama-select-gpu
GPUCONF

    # Create systemd service for llama-server
    sudo tee "$ROOTFS/etc/systemd/system/llama-server.service" > /dev/null <<'LLAMA_SVC'
[Unit]
Description=Yantrik Offline LLM Server (Qwen 3.5 4B)
After=network.target

[Service]
Type=simple
User=yantrik
ExecStart=/usr/local/bin/llama-server \
    --model /opt/yantrik/models/llm/yantrik-4b.gguf \
    --host 127.0.0.1 --port 8341 \
    --ctx-size 4096 --threads 2 --no-mmap
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
LLAMA_SVC

    # Enable llama-server on boot
    sudo chroot "$ROOTFS" systemctl enable llama-server 2>/dev/null || true

    ok "llama-server configured (systemd, port 8341)"
else
    info "Skipping LLM server (no offline model)"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 9: Configure Calamares installer (optional)
# ═══════════════════════════════════════════════════════════════
step "[9/10] Configuring installer..."

# Check if Calamares was installed
# Calamares is gone. It cost 169 MB across 113 KDE/Qt packages to deliver a 10.8 MB
# installer that nothing on the boot path ever reached: both GRUB entries pass
# yantrik.install=true, which puts the Slint onboarding into installer mode, and
# yantrik-install.sh below is the text fallback. An OS should not carry a second
# desktop framework so that a third installer can exist.
info "Installer: Slint onboarding (GRUB default) with yantrik-install as the text fallback"

# Always install the text-based installer (used by GRUB "Install" option)
sudo cp "$SCRIPT_DIR/yantrik-install.sh" "$ROOTFS/opt/yantrik/bin/yantrik-install"
# LF, for the same reason build-release.sh normalises the bundle: copied out of a working
# tree edited on Windows, a CRLF shebang makes this installer unrunnable on the machine it
# is meant to install.
sudo sed -i 's/\r$//' "$ROOTFS/opt/yantrik/bin/yantrik-install"
sudo chmod +x "$ROOTFS/opt/yantrik/bin/yantrik-install"
sudo chroot "$ROOTFS" bash -n /opt/yantrik/bin/yantrik-install \
    || fail "yantrik-install does not parse — the ISO's only text installer would not run"
ok "Text installer ready (parses)"

# ═══════════════════════════════════════════════════════════════
# STEP 10: Build ISO image
# ═══════════════════════════════════════════════════════════════
step "[10/10] Building ISO image..."

# Enable NetworkManager
sudo chroot "$ROOTFS" systemctl enable NetworkManager 2>/dev/null || true
# ── SSH: installed, not enabled ──
#
# This used to enable sshd and force `PasswordAuthentication yes` on every image. Read it
# together with the user created in step 3 — `yantrik`, password `yantrik`, passwordless sudo —
# and what shipped was root on any machine that booted this ISO, to anyone on its network who
# could guess a password printed in the build script. The installer copies this filesystem, so
# every installed machine kept it.
#
# "For debugging" is a real need and it belongs to the person who is debugging, not to every
# stranger who downloads the image. YANTRIK_ISO_SSH=1 puts it back for a build you make for
# yourself; the default build has sshd on disk, off at boot, and no config override.
if [ "${YANTRIK_ISO_SSH:-0}" = "1" ]; then
    sudo chroot "$ROOTFS" systemctl enable ssh 2>/dev/null || true
    sudo mkdir -p "$ROOTFS/etc/ssh/sshd_config.d"
    echo "PasswordAuthentication yes" | sudo tee "$ROOTFS/etc/ssh/sshd_config.d/yantrik.conf" > /dev/null
    warn "SSH ENABLED with password auth (YANTRIK_ISO_SSH=1) — user yantrik / password yantrik."
    warn "Do not publish this image. Anyone who can reach it on a network owns it."
else
    sudo chroot "$ROOTFS" systemctl disable ssh 2>/dev/null || true
    sudo rm -f "$ROOTFS/etc/ssh/sshd_config.d/yantrik.conf"
    ok "SSH installed but not enabled (YANTRIK_ISO_SSH=1 to enable for a private build)"
fi

# Load VM display drivers at boot (VBox vmwgfx, virtio-gpu, etc.)
echo -e "vmwgfx\nvirtio-gpu\ndrm" | sudo tee "$ROOTFS/etc/modules-load.d/yantrik-display.conf" > /dev/null

# Ensure live-config uses our yantrik user instead of creating "user"
sudo mkdir -p "$ROOTFS/etc/live/config.conf.d"
sudo tee "$ROOTFS/etc/live/config.conf.d/yantrik.conf" > /dev/null <<'LIVECONF'
LIVE_USERNAME="yantrik"
LIVE_USER_FULLNAME="Yantrik"
LIVE_USER_DEFAULT_GROUPS="audio video sudo input"
LIVE_NOCONFIGS="user-setup"
LIVECONF

# Clean up chroot
sudo chroot "$ROOTFS" apt-get clean
sudo rm -rf "$ROOTFS/tmp/"*
# The build host's resolv.conf was copied in for the chroot's apt; it must not ship.
#
# It used to be replaced with "nameserver 8.8.8.8" — every machine built from this image
# resolving every name it ever looks up through Google, chosen by a build script rather than
# by the person using the machine or by the network they joined. NetworkManager is installed
# and enabled and writes this file from DHCP; leaving it as NM's symlink is both the correct
# behaviour and the one that keeps nobody's DNS queries.
sudo rm -f "$ROOTFS/etc/resolv.conf"
sudo ln -sf /run/NetworkManager/resolv.conf "$ROOTFS/etc/resolv.conf"

# One version, taken from the build that is actually in the image.
#
# `.version` used to be written from $YANTRIK_VERSION — this script's own `git describe` — while
# /opt/yantrik/BUILD came from the release bundle unpacked above. Those two agree only when the
# ISO is built in the same checkout the bundle was, and for five months $YANTRIK_VERSION was the
# literal "0.3.0" (see the note where it is set), which is how a machine ended up reporting
# `.version` 0.3.0 beside `BUILD` version=v0.1.0-179-g6fc8b13. The marker is what the desktop,
# yantrik-install.sh and yantrik-update all read, so the marker is the answer; this script's own
# describe is the fallback for an image built with no bundle at all.
#
# `|| true`: this file is script-written and may not be there, and under `set -o pipefail` a
# failing sed would abort forty minutes of debootstrap over a missing marker.
INSTALLED_VERSION="$(sed -n 's/^version=//p' "$ROOTFS/opt/yantrik/BUILD" 2>/dev/null | head -1 || true)"
[ -n "$INSTALLED_VERSION" ] || INSTALLED_VERSION="$YANTRIK_VERSION"
echo "$INSTALLED_VERSION" | sudo tee "$ROOTFS/opt/yantrik/.version" > /dev/null

# The live image says what it is. Only yantrik-install wrote /etc/os-release, so the image
# people actually download called itself "Debian GNU/Linux 13" — on the getty banner, to every
# tool that asks, and in the first line of any bug report. Same fields the installer writes,
# same source for the version. /etc/os-release is a symlink into /usr/lib on Debian; it is
# replaced, not written through, so base-files' own copy stays what the package shipped.
sudo rm -f "$ROOTFS/etc/os-release"
printf 'PRETTY_NAME="Yantrik OS"
NAME="Yantrik OS"
ID=yantrik
ID_LIKE=debian
VERSION_ID="%s"
HOME_URL="https://yantrikos.com"
'     "$INSTALLED_VERSION" | sudo tee "$ROOTFS/etc/os-release" > /dev/null

# Unmount chroot filesystems
sudo umount "$ROOTFS/dev/pts" 2>/dev/null || true
sudo umount "$ROOTFS/dev" 2>/dev/null || true
sudo umount "$ROOTFS/proc" 2>/dev/null || true
sudo umount "$ROOTFS/sys" 2>/dev/null || true
trap - EXIT

# ── Copy kernel + initrd to ISO ──
VMLINUZ=$(ls "$ROOTFS/boot/vmlinuz-"* 2>/dev/null | head -1)
INITRD=$(ls "$ROOTFS/boot/initrd.img-"* 2>/dev/null | head -1)

if [ -z "$VMLINUZ" ] || [ -z "$INITRD" ]; then
    fail "Kernel or initrd not found in rootfs"
fi

sudo cp "$VMLINUZ" "$ISO_DIR/live/vmlinuz"
sudo cp "$INITRD" "$ISO_DIR/live/initrd"

# ── Create squashfs ──
info "Compressing rootfs (this takes a while)..."
sudo mksquashfs "$ROOTFS" "$ISO_DIR/live/filesystem.squashfs" \
    -comp xz -Xbcj x86 -noappend -quiet -xattrs \
    -e "$ROOTFS/boot/vmlinuz-*" \
    -e "$ROOTFS/boot/initrd.img-*"

ok "Squashfs created ($(du -h "$ISO_DIR/live/filesystem.squashfs" | cut -f1))"

# ── GRUB config (BIOS + EFI) ──
sudo tee "$ISO_DIR/boot/grub/grub.cfg" > /dev/null <<'GRUBCFG'
set timeout=5
set default=0

insmod all_video
insmod gfxterm
set gfxmode=auto
terminal_output gfxterm

# Speak on the serial line AS WELL AS the screen, and accept input from both.
# `--append`, so this adds the serial line to the screen rather than replacing it.
#
# Without this the menu exists only on a display. Every automated boot test — qemu -nographic,
# a headless VM, a server with a BMC — sees a blank serial port and cannot tell "the ISO hangs
# in GRUB" from "the ISO has no bootloader", which is the difference the test is being run to
# establish. The kernel lines already said console=ttyS0; the bootloader did not.
insmod serial
serial --unit=0 --speed=115200
terminal_output --append serial
terminal_input --append serial

set menu_color_normal=cyan/black
set menu_color_highlight=white/blue

# `yantrik.install=true` is what .bash_profile reads to put the Slint onboarding into
# installer mode. Both entries used to pass it, so there was no way to boot this image and
# simply try the desktop — an ISO that can only be installed is one nobody evaluates first.
menuentry "Install Yantrik OS" {
    linux /live/vmlinuz boot=live yantrik.install=true live-config.username=yantrik live-config.user-fullname=yantrik console=tty1 console=ttyS0,115200 quiet
    initrd /live/initrd
}

menuentry "Try Yantrik OS (live, no install)" {
    linux /live/vmlinuz boot=live live-config.username=yantrik live-config.user-fullname=yantrik console=tty1 console=ttyS0,115200 quiet
    initrd /live/initrd
}

menuentry "Install Yantrik OS (Safe Mode — software rendering)" {
    linux /live/vmlinuz boot=live yantrik.install=true live-config.username=yantrik live-config.user-fullname=yantrik console=tty1 console=ttyS0,115200 nomodeset quiet
    initrd /live/initrd
}

# No `quiet`, so the kernel and systemd say what they are doing on the serial line. This is
# the entry a bug report is made from, and the one an automated boot check selects.
menuentry "Try Yantrik OS (verbose, serial console)" {
    linux /live/vmlinuz boot=live live-config.username=yantrik live-config.user-fullname=yantrik console=tty1 console=ttyS0,115200 nomodeset systemd.log_level=info
    initrd /live/initrd
}
GRUBCFG

# ── Build ISO with grub-mkrescue (BIOS + EFI hybrid) ──
info "Creating hybrid ISO (BIOS + EFI)..."
# stderr kept. `2>/dev/null` here meant a grub-mkrescue that failed for a missing mtools or a
# missing EFI module said nothing, produced no file, and the next line died on `du` of a path
# that does not exist — forty minutes of debootstrap and apt thrown away with no reason given.
grub-mkrescue -o "$OUTPUT" "$ISO_DIR" -- -volid "YANTRIK_OS" \
    || fail "grub-mkrescue failed — no ISO was written (missing mtools, grub-efi-amd64-bin or grub-pc-bin?)"
[ -f "$OUTPUT" ] || fail "grub-mkrescue reported success but wrote no $OUTPUT"

# What was actually produced, read back off the file rather than assumed.
ISO_SIZE=$(du -h "$OUTPUT" | cut -f1)
ISO_SHA=$(sha256sum "$OUTPUT" | cut -d' ' -f1)
sha256sum "$OUTPUT" > "$OUTPUT.sha256"
# Asked of the rootfs's own dpkg database rather than through a chroot: by this point /proc
# and /dev have been unmounted, and this is the same answer without needing them back.
PKG_COUNT=$(dpkg-query --admindir="$ROOTFS/var/lib/dpkg" -f '${binary:Package}\n' -W 2>/dev/null | wc -l)
cat > "$OUTPUT.manifest" <<ISOMANIFEST
iso=$OUTPUT
version=$YANTRIK_VERSION
built=$(date -u +%Y-%m-%dT%H:%M:%SZ)
suite=$DEBIAN_SUITE
arch=$ARCH
sha256=$ISO_SHA
size=$(stat -c%s "$OUTPUT")
debian_packages=$PKG_COUNT
yantrik_binaries=$INSTALLED
offline_llm=$INCLUDE_LLM
ssh_enabled=${YANTRIK_ISO_SSH:-0}
mind=$([ "${YANTRIK_ISO_WITHOUT_MIND:-0}" = "1" ] && echo absent || echo present)
ISOMANIFEST
ok "$PKG_COUNT Debian packages · $INSTALLED Yantrik binaries · sha256 ${ISO_SHA:0:16}…"

echo
echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"
echo -e "${GREEN}  ISO built: ${BOLD}$OUTPUT${NC} ${GREEN}($ISO_SIZE)${NC}"
echo -e "${CYAN}═══════════════════════════════════════════════════${NC}"
echo
echo -e "  ${BOLD}Test with QEMU:${NC}"
echo -e "    qemu-system-x86_64 -m 4G -cdrom $OUTPUT -boot d \\"
echo -e "      -enable-kvm -display gtk -device virtio-vga"
echo
echo -e "  ${BOLD}Test with VirtualBox:${NC}"
echo -e "    1. New VM → Linux/Debian 64-bit → 4GB RAM"
echo -e "    2. Storage → IDE → Add optical → Select $OUTPUT"
echo -e "    3. Start"
echo
echo -e "  ${BOLD}Write to USB:${NC}"
echo -e "    sudo dd if=$OUTPUT of=/dev/sdX bs=4M status=progress"
echo
echo -e "  ${DIM}All packages are baked in — no internet needed for installation.${NC}"
# The command, as it is actually spelled. This said `yantrik-upgrade --check` for as long as
# the line has existed and there has never been a program by that name — so the last thing the
# ISO build told anyone was a command that ENOENTs.
echo -e "  ${DIM}Check for updates with:  yantrik-update check${NC}"
echo -e "  ${DIM}Change the channel with: yantrik-update set-channel nightly|beta|stable${NC}"
echo
