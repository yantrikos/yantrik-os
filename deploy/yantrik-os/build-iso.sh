#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# build-iso.sh — Create a bootable Yantrik OS live ISO
# ═══════════════════════════════════════════════════════════════
#
# Builds an Alpine-based live ISO (~500MB) containing:
#   - Alpine Linux 3.21 minimal
#   - labwc Wayland compositor
#   - Yantrik UI (pre-compiled binary)
#   - Foot terminal, grim/slurp, wl-clipboard, mako
#   - Pre-baked demo project with deliberate bug
#   - First-boot onboarding tutorial
#
# Output: yantrik-os-live.iso
#
# Requirements (run on Ubuntu/WSL2):
#   - alpine-make-rootfs or chroot tools
#   - xorriso (for ISO creation)
#   - syslinux/isolinux (for bootloader)
#   - yantrik-ui binary already built (release)
#
# Usage:
#   ./build-iso.sh                       # uses Ollama backend
#   ./build-iso.sh --candle              # embeds Candle (needs model files)
#   YANTRIK_BINARY=/path/to/bin ./build-iso.sh
#
# Quick test:
#   qemu-system-x86_64 -m 4G -cdrom yantrik-os-live.iso -boot d \
#     -enable-kvm -display gtk -device virtio-gpu-pci
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_DIR="$PROJECT_ROOT/config"
TARGET_DIR="${CARGO_TARGET_DIR:-/home/yantrik/target-yantrik}"

BACKEND="ollama"
BINARY="${YANTRIK_BINARY:-$TARGET_DIR/release/yantrik-ui}"
OUTPUT="yantrik-os-live.iso"
WORK_DIR="/tmp/yantrik-iso-build"
ROOTFS="$WORK_DIR/rootfs"

for arg in "$@"; do
    case "$arg" in
        --candle) BACKEND="candle" ;;
        --help|-h)
            echo "Usage: $0 [--candle]"
            echo "  Set YANTRIK_BINARY=/path/to/binary to use a pre-built binary."
            exit 0
            ;;
    esac
done

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Live ISO Builder"
echo "═══════════════════════════════════════════════"
echo
echo "  Backend:  $BACKEND"
echo "  Binary:   $BINARY"
echo "  Output:   $OUTPUT"
echo

# ── Verify prerequisites ──
for cmd in xorriso mksquashfs; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "ERROR: '$cmd' not found. Install:"
        echo "  sudo apt install xorriso squashfs-tools"
        exit 1
    fi
done

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    echo "  Build first: cargo build --release -p yantrik-ui --features api-llm"
    exit 1
fi

# ── Clean previous build ──
echo "[1/7] Preparing workspace..."
sudo rm -rf "$WORK_DIR"
mkdir -p "$ROOTFS" "$WORK_DIR/iso/boot/syslinux" "$WORK_DIR/iso/live"

# ── Step 2: Create minimal Alpine rootfs ──
echo "[2/7] Creating Alpine rootfs..."
# Download Alpine mini rootfs
ALPINE_VERSION="3.21"
ALPINE_ARCH="x86_64"
ALPINE_TAR="alpine-minirootfs-${ALPINE_VERSION}.0-${ALPINE_ARCH}.tar.gz"
ALPINE_URL="https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VERSION}/releases/${ALPINE_ARCH}/$ALPINE_TAR"

if [ ! -f "$WORK_DIR/$ALPINE_TAR" ]; then
    wget -q -O "$WORK_DIR/$ALPINE_TAR" "$ALPINE_URL"
fi
sudo tar xzf "$WORK_DIR/$ALPINE_TAR" -C "$ROOTFS"

# ── Step 3: Install packages via chroot ──
echo "[3/7] Installing packages in rootfs..."
sudo cp /etc/resolv.conf "$ROOTFS/etc/resolv.conf"
sudo mount -t proc proc "$ROOTFS/proc"
sudo mount -t sysfs sysfs "$ROOTFS/sys"
sudo mount --bind /dev "$ROOTFS/dev"

# Cleanup function
cleanup() {
    sudo umount "$ROOTFS/proc" 2>/dev/null || true
    sudo umount "$ROOTFS/sys" 2>/dev/null || true
    sudo umount "$ROOTFS/dev" 2>/dev/null || true
}
trap cleanup EXIT

sudo chroot "$ROOTFS" /bin/sh -c '
    # Enable community repo
    sed -i "s|#.*community|http://dl-cdn.alpinelinux.org/alpine/v3.21/community|" /etc/apk/repositories
    apk update

    # Core desktop packages
    apk add --no-cache \
        eudev mesa-dri-gallium mesa-egl \
        labwc foot wlr-randr \
        grim slurp wl-clipboard mako \
        dbus dbus-openrc ttf-dejavu \
        seatd seatd-openrc \
        linux-firmware-none

    # Create yantrik user
    adduser -D -s /bin/sh yantrik
    echo "yantrik:yantrik" | chpasswd

    # Enable services
    rc-update add dbus default
    rc-update add seatd default
    adduser yantrik seat

    # Auto-login on tty1
    sed -i "s|tty1::respawn.*|tty1::respawn:/bin/login -f yantrik|" /etc/inittab
'

# ── Step 4: Install Yantrik binary + config ──
echo "[4/7] Installing Yantrik OS binary and config..."
sudo mkdir -p "$ROOTFS/opt/yantrik/bin" "$ROOTFS/opt/yantrik/data" \
    "$ROOTFS/opt/yantrik/logs" "$ROOTFS/opt/yantrik/models/embedder"

sudo cp "$BINARY" "$ROOTFS/opt/yantrik/bin/yantrik-ui"
sudo chmod +x "$ROOTFS/opt/yantrik/bin/yantrik-ui"

if [ "$BACKEND" = "ollama" ]; then
    sudo cp "$CONFIG_DIR/yantrik-ollama.yaml" "$ROOTFS/opt/yantrik/config.yaml"
else
    sudo cp "$CONFIG_DIR/yantrik-os.yaml" "$ROOTFS/opt/yantrik/config.yaml"
fi

sudo chown -R 1000:1000 "$ROOTFS/opt/yantrik"

# ── Step 5: Configure desktop autostart ──
echo "[5/7] Configuring desktop environment..."

# labwc config for yantrik user
LABWC_DIR="$ROOTFS/home/yantrik/.config/labwc"
sudo mkdir -p "$LABWC_DIR"

# labwc environment
sudo tee "$LABWC_DIR/environment" > /dev/null << 'ENVEOF'
WLR_NO_HARDWARE_CURSORS=1
SLINT_BACKEND=winit-software
XDG_RUNTIME_DIR=/run/user/1000
ENVEOF

# labwc autostart
sudo tee "$LABWC_DIR/autostart" > /dev/null << 'AUTOEOF'
# Start foot terminal server
foot --server &

# Start notification daemon
mako &

# Start Yantrik OS
/opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml &
AUTOEOF
sudo chmod +x "$LABWC_DIR/autostart"

# User .profile — auto-start labwc on tty1
sudo tee "$ROOTFS/home/yantrik/.profile" > /dev/null << 'PROFEOF'
if [ "$(tty)" = "/dev/tty1" ]; then
    export XDG_RUNTIME_DIR="/run/user/$(id -u)"
    mkdir -p "$XDG_RUNTIME_DIR"
    exec labwc
fi
PROFEOF

sudo chown -R 1000:1000 "$ROOTFS/home/yantrik"

# ── Step 6: Create demo project (fix-this-error tutorial) ──
echo "[6/7] Creating demo project..."
DEMO_DIR="$ROOTFS/home/yantrik/demo-project"
sudo mkdir -p "$DEMO_DIR/src"

sudo tee "$DEMO_DIR/src/main.rs" > /dev/null << 'DEMOEOF'
// Demo Rust project — has a deliberate bug for the tutorial.
// Try: cargo build
// Then open Yantrik and type: "fix this error"

fn main() {
    let numbers = vec![1, 2, 3, 4, 5];
    let total = sum_all(numbers);
    println!("Sum: {}", total);

    // This function has a bug — can you spot it?
    let avg = average(&numbers);  // <-- error: value used after move
    println!("Average: {}", avg);
}

fn sum_all(nums: Vec<i32>) -> i32 {
    nums.iter().sum()
}

fn average(nums: &[i32]) -> f64 {
    let sum: i32 = nums.iter().sum();
    sum as f64 / nums.len() as f64
}
DEMOEOF

sudo tee "$DEMO_DIR/Cargo.toml" > /dev/null << 'CARGOEOF'
[package]
name = "demo-project"
version = "0.1.0"
edition = "2021"
CARGOEOF

sudo chown -R 1000:1000 "$DEMO_DIR"

# ── Step 7: Build ISO ──
echo "[7/7] Building ISO image..."

# Create squashfs
sudo mksquashfs "$ROOTFS" "$WORK_DIR/iso/live/filesystem.squashfs" \
    -comp xz -Xbcj x86 -noappend -quiet

# ISO metadata
cat > "$WORK_DIR/iso/boot/syslinux/syslinux.cfg" << 'SYSEOF'
DEFAULT yantrik
PROMPT 0
TIMEOUT 30

LABEL yantrik
    MENU LABEL Yantrik OS (Live)
    LINUX /boot/vmlinuz
    INITRD /boot/initrd
    APPEND root=live:CDLABEL=YANTRIK quiet
SYSEOF

# Build ISO with xorriso
xorriso -as mkisofs \
    -o "$OUTPUT" \
    -V "YANTRIK" \
    -J -r \
    "$WORK_DIR/iso"

ISO_SIZE=$(du -h "$OUTPUT" | cut -f1)
echo
echo "═══════════════════════════════════════════════"
echo "  ISO built: $OUTPUT ($ISO_SIZE)"
echo "═══════════════════════════════════════════════"
echo
echo "  Test with QEMU:"
echo "    qemu-system-x86_64 -m 4G -cdrom $OUTPUT -boot d \\"
echo "      -enable-kvm -display gtk -device virtio-gpu-pci"
echo
echo "  Write to USB:"
echo "    sudo dd if=$OUTPUT of=/dev/sdX bs=4M status=progress"
