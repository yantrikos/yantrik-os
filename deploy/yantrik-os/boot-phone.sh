#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# boot-phone.sh — Boot Yantrik OS in a phone-sized QEMU window
# ═══════════════════════════════════════════════════════════════
#
# This script boots a postmarketOS/Alpine image in QEMU with:
# - Phone-sized display (720x1280)
# - Port forwarding for SSH and Web UI
# - 4GB RAM, 4 CPUs
#
# Usage:
#   ./boot-phone.sh [path-to-disk-image]
#
# If no image is specified, uses the default location.
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

# ── Configuration ──
RAM="4G"
CPUS=4
SSH_PORT=2222
WEB_PORT=8340
VNC_PORT=5900

# ── Detect architecture ──
ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64)
        QEMU_BIN="qemu-system-x86_64"
        ACCEL="-enable-kvm"
        # Check if KVM is available
        if [ ! -e /dev/kvm ]; then
            ACCEL="-accel tcg"
            echo "Warning: KVM not available, using software emulation (slower)"
        fi
        MACHINE="-machine q35"
        ;;
    arm64|aarch64)
        QEMU_BIN="qemu-system-aarch64"
        # macOS uses HVF, Linux uses KVM
        if [ "$(uname -s)" = "Darwin" ]; then
            ACCEL="-accel hvf"
            # macOS needs UEFI firmware
            EFI_CODE=""
            for f in \
                /opt/homebrew/share/qemu/edk2-aarch64-code.fd \
                /usr/local/share/qemu/edk2-aarch64-code.fd \
                /usr/share/AAVMF/AAVMF_CODE.fd; do
                if [ -f "$f" ]; then
                    EFI_CODE="$f"
                    break
                fi
            done
            if [ -z "$EFI_CODE" ]; then
                echo "Error: UEFI firmware not found for aarch64"
                echo "Install: brew install qemu (includes EDK2 firmware)"
                exit 1
            fi
            MACHINE="-machine virt,highmem=on -cpu host -bios $EFI_CODE"
        else
            ACCEL="-enable-kvm"
            if [ ! -e /dev/kvm ]; then
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

# ── Find disk image ──
if [ $# -ge 1 ]; then
    DISK_IMG="$1"
else
    # Default locations
    for f in \
        "$HOME/yantrik-os-image/"*.img \
        "$HOME/yantrik-vm/"*.qcow2 \
        "$HOME/yantrik-vm/"*.img; do
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
    echo "To create a disk image:"
    echo "  1. WSL2: Run setup-wsl2.sh (builds postmarketOS)"
    echo "  2. Manual: Download Alpine and install to disk:"
    echo "     qemu-img create -f qcow2 yantrik.qcow2 16G"
    echo "     $QEMU_BIN ... -cdrom alpine-virt.iso -boot d"
    exit 1
fi

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Phone Simulator"
echo "═══════════════════════════════════════════════"
echo
echo "  Arch:    $ARCH"
echo "  Image:   $DISK_IMG"
echo "  RAM:     $RAM"
echo "  CPUs:    $CPUS"
echo "  Display: 720x1280 (phone portrait)"
echo
echo "  Access:"
echo "    SSH:     ssh -p $SSH_PORT yantrik@localhost"
echo "    Web UI:  http://localhost:$WEB_PORT"
echo "    VNC:     localhost:$VNC_PORT"
echo
echo "Starting..."

# ── Launch QEMU ──
$QEMU_BIN \
    $MACHINE \
    $ACCEL \
    -m "$RAM" \
    -smp "$CPUS" \
    -drive file="$DISK_IMG",if=virtio \
    -device virtio-gpu-pci \
    -display gtk,window-close=poweroff \
    -device virtio-keyboard-pci \
    -device virtio-tablet-pci \
    -device virtio-net-pci,netdev=net0 \
    -netdev user,id=net0,hostfwd=tcp::${SSH_PORT}-:22,hostfwd=tcp::${WEB_PORT}-:8340,hostfwd=tcp::${VNC_PORT}-:5900 \
    -device virtio-rng-pci \
    -boot c \
    -name "Yantrik OS" \
    "$@"
