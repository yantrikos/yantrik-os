#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# boot-desktop.sh — Boot Yantrik OS in a desktop-sized QEMU window
# ═══════════════════════════════════════════════════════════════
#
# This script boots an Alpine Linux image in QEMU with:
# - Desktop display (1920x1080 landscape)
# - Port forwarding for SSH, Web UI, and VNC
# - 4GB RAM, 4 CPUs
# - Automatic WSL2, Linux, and macOS detection
#
# Usage:
#   ./boot-desktop.sh [path-to-disk-image]
#
# If no image is specified, looks in ~/yantrik-vm/
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

# ── Configuration ──
RAM="8G"
CPUS=4
SSH_PORT=2222
WEB_PORT=8340
VNC_PORT=5900

# ── Detect platform ──
OS_TYPE="linux"
IS_WSL=false

case "$(uname -s)" in
    Darwin)  OS_TYPE="macos" ;;
    Linux)
        if grep -qi microsoft /proc/version 2>/dev/null; then
            IS_WSL=true
            OS_TYPE="wsl2"
        fi
        ;;
esac

# ── Detect architecture ──
ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64)
        QEMU_BIN="qemu-system-x86_64"
        ACCEL=""
        MACHINE="-machine q35"

        if [ "$OS_TYPE" = "wsl2" ] || [ "$OS_TYPE" = "linux" ]; then
            if [ -e /dev/kvm ]; then
                ACCEL="-enable-kvm"
            else
                ACCEL="-accel tcg"
                echo "Warning: KVM not available, using software emulation (slower)"
                echo "  WSL2: Enable 'nestedVirtualization' in .wslconfig"
            fi
        fi
        ;;
    arm64|aarch64)
        QEMU_BIN="qemu-system-aarch64"
        if [ "$OS_TYPE" = "macos" ]; then
            ACCEL="-accel hvf"
            EFI_CODE=""
            for f in \
                /opt/homebrew/share/qemu/edk2-aarch64-code.fd \
                /usr/local/share/qemu/edk2-aarch64-code.fd; do
                [ -f "$f" ] && EFI_CODE="$f" && break
            done
            if [ -z "$EFI_CODE" ]; then
                echo "Error: UEFI firmware not found. Install: brew install qemu"
                exit 1
            fi
            MACHINE="-machine virt,highmem=on -cpu host -bios $EFI_CODE"
        else
            if [ -e /dev/kvm ]; then
                ACCEL="-enable-kvm"
            else
                ACCEL="-accel tcg"
            fi
            MACHINE="-machine virt -cpu cortex-a72"
        fi
        ;;
    *)
        echo "Error: Unsupported architecture: $ARCH"
        exit 1
        ;;
esac

# ── Determine display backend ──
DISPLAY_ARGS=""
case "$OS_TYPE" in
    wsl2)
        if [ -n "${DISPLAY:-}" ] || [ -d "/mnt/wslg" ]; then
            DISPLAY_ARGS="-display gtk"
        else
            DISPLAY_ARGS="-display none -vnc :0"
            echo "Note: No display detected. Using VNC on port $VNC_PORT"
            echo "  Connect with a VNC viewer: localhost:$VNC_PORT"
        fi
        ;;
    macos)
        DISPLAY_ARGS="-display cocoa"
        ;;
    linux)
        DISPLAY_ARGS="-display gtk"
        ;;
esac

# ── Find disk image ──
DISK_IMG=""
if [ $# -ge 1 ]; then
    DISK_IMG="$1"
else
    for f in \
        "$HOME/yantrik-vm/yantrik-os.qcow2" \
        "$HOME/yantrik-vm/"*.qcow2 \
        "$HOME/yantrik-vm/"*.img \
        "$HOME/yantrik-os-image/"*.img; do
        if [ -f "$f" ]; then
            DISK_IMG="$f"
            break
        fi
    done
fi

if [ -z "${DISK_IMG:-}" ] || [ ! -f "$DISK_IMG" ]; then
    echo "Error: No disk image found"
    echo
    echo "Usage: $0 [path-to-disk-image]"
    echo
    echo "Create one first:"
    echo "  ./setup-alpine-vm.sh"
    exit 1
fi

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Desktop"
echo "═══════════════════════════════════════════════"
echo
echo "  Platform: $OS_TYPE ($ARCH)"
echo "  Image:    $DISK_IMG"
echo "  RAM:      $RAM"
echo "  CPUs:     $CPUS"
echo "  Display:  1920x1080 (desktop)"
echo
echo "  Access:"
echo "    SSH:     ssh -p $SSH_PORT yantrik@localhost"
echo "    Web UI:  http://localhost:$WEB_PORT"
if [[ "$DISPLAY_ARGS" == *vnc* ]]; then
    echo "    VNC:     localhost:$VNC_PORT"
fi
echo
echo "Starting..."

# ── Launch QEMU ──
# shellcheck disable=SC2086
$QEMU_BIN \
    $MACHINE \
    $ACCEL \
    -m "$RAM" \
    -smp "$CPUS" \
    -drive file="$DISK_IMG",if=virtio \
    -device virtio-gpu-pci \
    $DISPLAY_ARGS \
    -device virtio-keyboard-pci \
    -device qemu-xhci -device usb-tablet \
    -device virtio-net-pci,netdev=net0 \
    -netdev user,id=net0,hostfwd=tcp::${SSH_PORT}-:22,hostfwd=tcp::${WEB_PORT}-:8340,hostfwd=tcp::${VNC_PORT}-:5900 \
    -device virtio-rng-pci \
    -device intel-hda -device hda-duplex \
    -boot c \
    -name "Yantrik OS" \
    "$@"
