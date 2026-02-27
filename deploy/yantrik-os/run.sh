#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# run.sh — One-command Yantrik OS phone simulator
# ═══════════════════════════════════════════════════════════════
#
# Run this in your terminal (not from Claude Code):
#   cd ~/codes/aidb/deploy/yantrik-os
#   chmod +x run.sh
#   ./run.sh
#
# First run: installs Alpine, second run onwards: boots Yantrik OS
# ═══════════════════════════════════════════════════════════════

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VM_DIR="$HOME/yantrik-vm"
DISK="$VM_DIR/yantrik-os.qcow2"
ISO="$VM_DIR/alpine-virt-3.23.3-aarch64.iso"
EFI="/opt/homebrew/share/qemu/edk2-aarch64-code.fd"

mkdir -p "$VM_DIR"

# ── Check dependencies ──
if ! command -v qemu-system-aarch64 &>/dev/null; then
    echo "Error: QEMU not found. Install: brew install qemu"
    exit 1
fi

if [ ! -f "$EFI" ]; then
    echo "Error: UEFI firmware not found at $EFI"
    echo "Install: brew install qemu"
    exit 1
fi

# ── Download ISO if needed ──
if [ ! -f "$ISO" ]; then
    echo "Downloading Alpine Linux 3.23.3 aarch64..."
    curl -L -o "$ISO" \
        "https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/aarch64/alpine-virt-3.23.3-aarch64.iso"
fi

# ── Determine mode: install or boot ──
INSTALLED_MARKER="$VM_DIR/.installed"

if [ ! -f "$DISK" ] || [ ! -f "$INSTALLED_MARKER" ]; then
    echo "═══════════════════════════════════════════════"
    echo "  Yantrik OS — First-Time Setup"
    echo "═══════════════════════════════════════════════"
    echo

    # Create disk
    [ ! -f "$DISK" ] && qemu-img create -f qcow2 "$DISK" 16G

    echo "This will boot the Alpine installer."
    echo
    echo "╔══════════════════════════════════════════════╗"
    echo "║  After QEMU window opens:                   ║"
    echo "║                                             ║"
    echo "║  1. Login: root (no password)               ║"
    echo "║  2. Run: setup-alpine                       ║"
    echo "║  3. Follow prompts:                         ║"
    echo "║     - Keyboard: us                          ║"
    echo "║     - Hostname: yantrik                     ║"
    echo "║     - Network: eth0, dhcp                   ║"
    echo "║     - Root password: 1234                   ║"
    echo "║     - Timezone: US/Eastern (or yours)       ║"
    echo "║     - Mirror: f (fastest)                   ║"
    echo "║     - User: yantrik, password: 1234         ║"
    echo "║     - SSH: openssh                          ║"
    echo "║     - Disk: vda, sys mode                   ║"
    echo "║  4. After install: poweroff                 ║"
    echo "║  5. Run this script again to boot!          ║"
    echo "╚══════════════════════════════════════════════╝"
    echo
    read -p "Press Enter to start installer..."

    qemu-system-aarch64 \
        -machine virt,highmem=on \
        -cpu host \
        -accel hvf \
        -bios "$EFI" \
        -m 4G \
        -smp 4 \
        -drive file="$DISK",if=virtio \
        -cdrom "$ISO" \
        -boot d \
        -device virtio-net-pci,netdev=net0 \
        -netdev user,id=net0,hostfwd=tcp::2222-:22 \
        -device virtio-gpu-pci \
        -display cocoa \
        -device virtio-keyboard-pci \
        -device virtio-tablet-pci \
        -device virtio-rng-pci \
        -name "Yantrik OS Setup"

    echo
    read -p "Did the installation complete successfully? [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        touch "$INSTALLED_MARKER"
        echo "Marked as installed. Run this script again to boot Yantrik OS!"
    else
        echo "Run this script again to retry installation."
    fi

else
    echo "═══════════════════════════════════════════════"
    echo "  Yantrik OS — Phone Simulator"
    echo "═══════════════════════════════════════════════"
    echo
    echo "  Display:  720x1440 (phone portrait)"
    echo "  SSH:      ssh -p 2222 yantrik@localhost"
    echo "  Web UI:   http://localhost:8340"
    echo

    qemu-system-aarch64 \
        -machine virt,highmem=on \
        -cpu host \
        -accel hvf \
        -bios "$EFI" \
        -m 4G \
        -smp 4 \
        -drive file="$DISK",if=virtio \
        -boot c \
        -device virtio-net-pci,netdev=net0 \
        -netdev user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::8340-:8340,hostfwd=tcp::5900-:5900 \
        -device virtio-gpu-pci \
        -display cocoa \
        -device virtio-keyboard-pci \
        -device virtio-tablet-pci \
        -device virtio-rng-pci \
        -name "Yantrik OS"
fi
