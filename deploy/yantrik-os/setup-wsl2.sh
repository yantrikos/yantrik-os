#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Yantrik OS — WSL2 + postmarketOS Phone Simulator Setup
# ═══════════════════════════════════════════════════════════════
#
# Run this INSIDE WSL2 (Ubuntu/Debian) on your Windows desktop.
# It installs pmbootstrap, builds a postmarketOS phone image,
# and boots it in QEMU with a phone-sized display.
#
# Usage:
#   chmod +x setup-wsl2.sh
#   ./setup-wsl2.sh
#
# Prerequisites (on Windows):
#   1. WSL2 installed:  wsl --install
#   2. Ubuntu from Microsoft Store
#   3. X server for display: install VcXsrv or use WSLg (Win11)
# ═══════════════════════════════════════════════════════════════

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Phone Simulator Setup (WSL2)"
echo "═══════════════════════════════════════════════"
echo

# ── Step 1: System packages ──
echo "[1/6] Installing system dependencies..."
sudo apt-get update -qq
sudo apt-get install -y -qq \
    python3 python3-pip python3-venv git openssl \
    qemu-system-x86 qemu-utils qemu-efi-aarch64 \
    curl wget unzip build-essential

# ── Step 2: Install pmbootstrap ──
echo "[2/6] Installing pmbootstrap..."
if ! command -v pmbootstrap &>/dev/null; then
    # Clone and install from git (most reliable method)
    PMBS_DIR="$HOME/.local/share/pmbootstrap-src"
    if [ ! -d "$PMBS_DIR" ]; then
        git clone --depth 1 https://gitlab.com/postmarketOS/pmbootstrap.git "$PMBS_DIR"
    fi
    mkdir -p "$HOME/.local/bin"
    ln -sf "$PMBS_DIR/pmbootstrap.py" "$HOME/.local/bin/pmbootstrap"
    export PATH="$HOME/.local/bin:$PATH"
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc"
fi
echo "  pmbootstrap: $(pmbootstrap --version 2>/dev/null || echo 'installed')"

# ── Step 3: Initialize pmbootstrap (non-interactive) ──
echo "[3/6] Initializing postmarketOS build environment..."

# Create pmbootstrap config for QEMU phone
PMBS_WORK="$HOME/.local/var/pmbootstrap"
mkdir -p "$PMBS_WORK"

# Auto-configure for qemu-amd64 with Phosh UI
pmbootstrap init \
    --config "$HOME/.config/pmbootstrap_v4.cfg" \
    --no-interactive \
    2>/dev/null || true

# If non-interactive init doesn't work, create config manually
if [ ! -f "$HOME/.config/pmbootstrap_v4.cfg" ]; then
    cat > "$HOME/.config/pmbootstrap.cfg" <<'PMCFG'
[pmbootstrap]
aports = /home/user/.local/var/pmbootstrap/cache_git/pmaports
boot_size = 256
build_pkgs_on_install = True
ccache_size = 5G
device = qemu-amd64
extra_packages = nano,htop,python3,py3-pip,py3-sqlite3,git,curl,sqlite
hostname = yantrik
is_default_channel = False
jobs = 4
kernel = edge
keymap =
locale = en_US.UTF-8
mirror_alpine = http://dl-cdn.alpinelinux.org/alpine/
nonfree_firmware = True
nonfree_userland = True
ssh_key_glob = ~/.ssh/id_*.pub
ssh_keys = True
sudo_timer = True
timezone = America/New_York
ui = phosh
ui_extras = True
user = yantrik
work = /home/user/.local/var/pmbootstrap
PMCFG
    # Replace /home/user with actual home
    sed -i "s|/home/user|$HOME|g" "$HOME/.config/pmbootstrap.cfg"
fi

echo "  Build environment ready"

# ── Step 4: Build the image ──
echo "[4/6] Building postmarketOS image (this takes 5-10 minutes)..."
pmbootstrap install --password 1234 2>&1 | tail -5 || {
    echo "  Note: If build fails, run manually:"
    echo "    pmbootstrap init  (choose qemu-amd64, phosh UI)"
    echo "    pmbootstrap install --password 1234"
}

# ── Step 5: Export image ──
echo "[5/6] Exporting bootable image..."
EXPORT_DIR="$HOME/yantrik-os-image"
mkdir -p "$EXPORT_DIR"
pmbootstrap export "$EXPORT_DIR" 2>&1 | tail -3 || true

# ── Step 6: Create boot script ──
echo "[6/6] Creating phone simulator boot script..."
cat > "$HOME/boot-yantrik.sh" <<'BOOTSCRIPT'
#!/bin/bash
# Boot Yantrik OS in a phone-sized QEMU window
# Display: 720x1280 (phone portrait)

EXPORT_DIR="$HOME/yantrik-os-image"
IMG=$(ls "$EXPORT_DIR"/*.img 2>/dev/null | head -1)

if [ -z "$IMG" ]; then
    echo "Error: No image found in $EXPORT_DIR"
    echo "Run: pmbootstrap export $EXPORT_DIR"
    exit 1
fi

# Ensure DISPLAY is set (for WSLg or VcXsrv)
export DISPLAY="${DISPLAY:-:0}"

echo "Starting Yantrik OS..."
echo "  Image: $IMG"
echo "  Display: 720x1280 (phone)"
echo "  SSH: localhost:2222"
echo "  Web UI: localhost:8340"
echo

qemu-system-x86_64 \
    -m 4G \
    -smp 4 \
    -enable-kvm \
    -drive file="$IMG",format=raw,if=virtio \
    -device virtio-gpu-pci \
    -display gtk,window-close=poweroff \
    -device virtio-keyboard-pci \
    -device virtio-mouse-pci \
    -device virtio-net-pci,netdev=net0 \
    -netdev user,id=net0,hostfwd=tcp::2222-:22,hostfwd=tcp::8340-:8340 \
    -device virtio-rng-pci \
    -boot c \
    -name "Yantrik OS" \
    2>/dev/null &

QEMU_PID=$!
echo "QEMU PID: $QEMU_PID"
echo
echo "Access:"
echo "  SSH:    ssh -p 2222 yantrik@localhost"
echo "  Web UI: http://localhost:8340"
echo
echo "Press Ctrl+C to stop"
wait $QEMU_PID
BOOTSCRIPT
chmod +x "$HOME/boot-yantrik.sh"

echo
echo "═══════════════════════════════════════════════"
echo "  Setup Complete!"
echo "═══════════════════════════════════════════════"
echo
echo "Next steps:"
echo "  1. Boot the phone:  ~/boot-yantrik.sh"
echo "  2. SSH into it:     ssh -p 2222 yantrik@localhost"
echo "  3. Deploy Yantrik:  scp deploy-stack.sh yantrik@localhost:"
echo "                      ssh -p 2222 yantrik@localhost ./deploy-stack.sh"
echo
