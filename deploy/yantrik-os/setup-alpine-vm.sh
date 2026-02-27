#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# setup-alpine-vm.sh — Create a fresh Alpine VM for Yantrik OS
# ═══════════════════════════════════════════════════════════════
#
# Works on: macOS (Apple Silicon), Windows WSL2, Linux
# Creates a QEMU disk, boots Alpine ISO, you install it, done.
#
# Usage:
#   ./setup-alpine-vm.sh
#
# This creates the disk image and boots the Alpine installer.
# After installation, use boot-phone.sh to start the phone.
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

VM_DIR="${YANTRIK_VM_DIR:-$HOME/yantrik-vm}"
DISK_SIZE="16G"
DISK_IMG="$VM_DIR/yantrik-os.qcow2"
RAM="4G"

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Alpine VM Setup"
echo "═══════════════════════════════════════════════"
echo

mkdir -p "$VM_DIR"

# ── Detect architecture ──
ARCH=$(uname -m)
case "$ARCH" in
    arm64|aarch64)
        ISO_ARCH="aarch64"
        QEMU_BIN="qemu-system-aarch64"
        ;;
    x86_64|amd64)
        ISO_ARCH="x86_64"
        QEMU_BIN="qemu-system-x86_64"
        ;;
    *)
        echo "Error: Unsupported arch: $ARCH"
        exit 1
        ;;
esac

# ── Download Alpine ISO if needed ──
ISO=$(ls "$VM_DIR"/alpine-virt-*-${ISO_ARCH}.iso 2>/dev/null | head -1)
if [ -z "$ISO" ]; then
    echo "Downloading Alpine Linux virtual ISO for $ISO_ARCH..."
    ALPINE_VERSION="3.23.3"
    ISO="$VM_DIR/alpine-virt-${ALPINE_VERSION}-${ISO_ARCH}.iso"
    curl -L -o "$ISO" \
        "https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/${ISO_ARCH}/alpine-virt-${ALPINE_VERSION}-${ISO_ARCH}.iso"
fi
echo "ISO: $ISO"

# ── Create disk image ──
if [ -f "$DISK_IMG" ]; then
    echo "Disk image already exists: $DISK_IMG"
    read -p "Overwrite? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 0
    fi
fi

echo "Creating $DISK_SIZE disk image..."
qemu-img create -f qcow2 "$DISK_IMG" "$DISK_SIZE"

# ── Build QEMU command ──
COMMON_ARGS=(
    -m "$RAM"
    -smp 4
    -drive "file=$DISK_IMG,if=virtio"
    -cdrom "$ISO"
    -boot d
    -device virtio-net-pci,netdev=net0
    -netdev "user,id=net0,hostfwd=tcp::2222-:22"
    -device virtio-rng-pci
    -display gtk
    -name "Yantrik OS Setup"
)

if [ "$ARCH" = "arm64" ] || [ "$ARCH" = "aarch64" ]; then
    # macOS Apple Silicon
    EFI_CODE=""
    for f in \
        /opt/homebrew/share/qemu/edk2-aarch64-code.fd \
        /usr/local/share/qemu/edk2-aarch64-code.fd; do
        if [ -f "$f" ]; then EFI_CODE="$f"; break; fi
    done

    if [ -z "$EFI_CODE" ]; then
        echo "Error: UEFI firmware not found. Install: brew install qemu"
        exit 1
    fi

    ACCEL="-accel hvf"
    if [ "$(uname -s)" != "Darwin" ]; then
        ACCEL="-enable-kvm"
        [ ! -e /dev/kvm ] && ACCEL="-accel tcg"
    fi

    echo
    echo "Booting Alpine installer (aarch64)..."
    echo
    echo "════════════════════════════════════════════"
    echo "  INSTALLATION GUIDE"
    echo "════════════════════════════════════════════"
    echo "  1. Login as 'root' (no password)"
    echo "  2. Run: setup-alpine"
    echo "  3. Hostname: yantrik"
    echo "  4. Network: eth0, dhcp"
    echo "  5. Root password: 1234 (or your choice)"
    echo "  6. Timezone: your timezone"
    echo "  7. Mirror: 1 (or nearest)"
    echo "  8. User: yantrik (password: 1234)"
    echo "  9. SSH: openssh"
    echo "  10. Disk: vda, sys mode"
    echo "  11. After install: poweroff"
    echo "  12. Then use: ./boot-phone.sh $DISK_IMG"
    echo "════════════════════════════════════════════"
    echo

    $QEMU_BIN \
        -machine virt,highmem=on \
        -cpu host \
        $ACCEL \
        -bios "$EFI_CODE" \
        "${COMMON_ARGS[@]}"

else
    # x86_64 (WSL2 / Linux)
    ACCEL="-enable-kvm"
    [ ! -e /dev/kvm ] && ACCEL="-accel tcg"

    echo
    echo "Booting Alpine installer (x86_64)..."
    echo
    echo "════════════════════════════════════════════"
    echo "  INSTALLATION GUIDE"
    echo "════════════════════════════════════════════"
    echo "  1. Login as 'root' (no password)"
    echo "  2. Run: setup-alpine"
    echo "  3. Hostname: yantrik"
    echo "  4. Network: eth0, dhcp"
    echo "  5. Root password: 1234 (or your choice)"
    echo "  6. Timezone: your timezone"
    echo "  7. Mirror: 1 (or nearest)"
    echo "  8. User: yantrik (password: 1234)"
    echo "  9. SSH: openssh"
    echo "  10. Disk: vda, sys mode"
    echo "  11. After install: poweroff"
    echo "  12. Then use: ./boot-phone.sh $DISK_IMG"
    echo "════════════════════════════════════════════"
    echo

    $QEMU_BIN \
        -machine q35 \
        $ACCEL \
        "${COMMON_ARGS[@]}"
fi

echo
echo "Installation complete!"
echo "Boot the phone: ./boot-phone.sh $DISK_IMG"
