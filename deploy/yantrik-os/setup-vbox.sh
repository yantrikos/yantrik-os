#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# setup-vbox.sh — Create a VirtualBox VM for Yantrik OS
# ═══════════════════════════════════════════════════════════════
#
# Creates an Alpine Linux VM in VirtualBox with:
#   - 4GB RAM, 2 CPUs, VMSVGA graphics
#   - 16GB dynamically allocated VDI disk
#   - NAT networking with port forwarding (SSH, Web UI)
#   - Alpine virtual ISO attached for installation
#
# Usage:
#   ./setup-vbox.sh                    # auto-detect everything
#   ./setup-vbox.sh --name MyVM        # custom VM name
#   ./setup-vbox.sh --ram 8192         # 8GB RAM
#
# After this script:
#   1. Install Alpine in the VBox window (see printed instructions)
#   2. Run: ./deploy-vbox.sh
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

# ── Configuration (overridable via flags) ──
VM_NAME="Yantrik-OS"
RAM_MB=4096
CPUS=2
VRAM_MB=128
DISK_SIZE_MB=16384
SSH_PORT=2222
WEB_PORT=8340

ALPINE_VERSION="3.21"
ALPINE_MINOR="3.21.3"
ALPINE_ARCH="x86_64"
CACHE_DIR="$HOME/yantrik-vm"

# ── Parse flags ──
while [ $# -gt 0 ]; do
    case "$1" in
        --name)  VM_NAME="$2"; shift 2 ;;
        --ram)   RAM_MB="$2"; shift 2 ;;
        --cpus)  CPUS="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: $0 [--name NAME] [--ram MB] [--cpus N]"
            exit 0
            ;;
        *) echo "Unknown flag: $1"; exit 1 ;;
    esac
done

# ── Detect VBoxManage ──
VBOXMANAGE=""

# Try common locations
for candidate in \
    "VBoxManage" \
    "VBoxManage.exe" \
    "/c/Program Files/Oracle/VirtualBox/VBoxManage.exe" \
    "/mnt/c/Program Files/Oracle/VirtualBox/VBoxManage.exe" \
    "C:/Program Files/Oracle/VirtualBox/VBoxManage.exe"; do
    if command -v "$candidate" &>/dev/null 2>&1; then
        VBOXMANAGE="$candidate"
        break
    fi
    # Try with full path (Git Bash / MSYS2)
    if [ -f "$candidate" ]; then
        VBOXMANAGE="$candidate"
        break
    fi
done

if [ -z "$VBOXMANAGE" ]; then
    echo "ERROR: VBoxManage not found."
    echo
    echo "Make sure Oracle VirtualBox is installed and VBoxManage is in your PATH."
    echo "  Windows: Add 'C:\\Program Files\\Oracle\\VirtualBox' to PATH"
    echo "  Or run from Git Bash: export PATH=\"\$PATH:/c/Program Files/Oracle/VirtualBox\""
    exit 1
fi

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — VirtualBox VM Setup"
echo "═══════════════════════════════════════════════"
echo
echo "  VBoxManage: $VBOXMANAGE"
echo "  VM Name:    $VM_NAME"
echo "  RAM:        ${RAM_MB}MB"
echo "  CPUs:       $CPUS"
echo "  Disk:       ${DISK_SIZE_MB}MB (dynamic VDI)"
echo "  Ports:      SSH=$SSH_PORT, Web=$WEB_PORT"
echo

# ── Check if VM already exists ──
if "$VBOXMANAGE" showvminfo "$VM_NAME" &>/dev/null 2>&1; then
    echo "WARNING: VM '$VM_NAME' already exists."
    echo "  To delete it: VBoxManage unregistervm \"$VM_NAME\" --delete"
    echo "  Or use a different name: $0 --name MyVM"
    exit 1
fi

# ── Download Alpine ISO ──
mkdir -p "$CACHE_DIR"
ALPINE_ISO="alpine-virt-${ALPINE_MINOR}-${ALPINE_ARCH}.iso"
ALPINE_URL="https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_VERSION}/releases/${ALPINE_ARCH}/${ALPINE_ISO}"
ISO_PATH="$CACHE_DIR/$ALPINE_ISO"

if [ -f "$ISO_PATH" ]; then
    echo "[1/5] Alpine ISO already cached: $ISO_PATH"
else
    echo "[1/5] Downloading Alpine ${ALPINE_MINOR} virtual ISO (~60MB)..."
    curl -L -o "$ISO_PATH" "$ALPINE_URL"
    echo "  Downloaded: $ISO_PATH"
fi

# Convert to Windows path if running in MSYS2/Git Bash
ISO_PATH_VBOX="$ISO_PATH"
VDI_DIR="$CACHE_DIR"
if [[ "$OSTYPE" == "msys"* ]] || [[ "$OSTYPE" == "mingw"* ]]; then
    # Git Bash: convert /c/Users/... to C:\Users\...
    ISO_PATH_VBOX="$(cygpath -w "$ISO_PATH" 2>/dev/null || echo "$ISO_PATH")"
    VDI_DIR="$(cygpath -w "$CACHE_DIR" 2>/dev/null || echo "$CACHE_DIR")"
elif grep -qi microsoft /proc/version 2>/dev/null; then
    # WSL2: convert /home/user/... to \\wsl$\... or use /mnt/c path
    # VBoxManage.exe runs on Windows, so paths must be Windows-native
    if [[ "$ISO_PATH" == /mnt/c/* ]]; then
        ISO_PATH_VBOX="$(echo "$ISO_PATH" | sed 's|^/mnt/c|C:|; s|/|\\|g')"
        VDI_DIR="$(echo "$CACHE_DIR" | sed 's|^/mnt/c|C:|; s|/|\\|g')"
    fi
fi

VDI_PATH="$VDI_DIR/${VM_NAME}.vdi"

# ── Create VM ──
echo "[2/5] Creating VirtualBox VM..."
"$VBOXMANAGE" createvm --name "$VM_NAME" --ostype "Linux_64" --register

# Configure VM hardware
"$VBOXMANAGE" modifyvm "$VM_NAME" \
    --memory "$RAM_MB" \
    --cpus "$CPUS" \
    --vram "$VRAM_MB" \
    --graphicscontroller vmsvga \
    --accelerate3d on \
    --audio-driver default \
    --audio-enabled on \
    --nic1 nat \
    --boot1 dvd \
    --boot2 disk \
    --boot3 none \
    --boot4 none \
    --clipboard-mode bidirectional \
    --draganddrop bidirectional \
    --uart1 off

echo "  VM created: $VM_NAME"

# ── Create disk ──
echo "[3/5] Creating ${DISK_SIZE_MB}MB VDI disk..."
"$VBOXMANAGE" createmedium disk --filename "$VDI_PATH" --size "$DISK_SIZE_MB" --format VDI --variant Standard

# Add SATA controller and attach disk
"$VBOXMANAGE" storagectl "$VM_NAME" --name "SATA" --add sata --controller IntelAhci --portcount 2
"$VBOXMANAGE" storageattach "$VM_NAME" --storagectl "SATA" --port 0 --device 0 --type hdd --medium "$VDI_PATH"

echo "  Disk: $VDI_PATH"

# ── Attach Alpine ISO ──
echo "[4/5] Attaching Alpine ISO..."
"$VBOXMANAGE" storagectl "$VM_NAME" --name "IDE" --add ide
"$VBOXMANAGE" storageattach "$VM_NAME" --storagectl "IDE" --port 0 --device 0 --type dvddrive --medium "$ISO_PATH_VBOX"

echo "  ISO: $ISO_PATH_VBOX"

# ── Port forwarding ──
echo "[5/5] Configuring network..."
"$VBOXMANAGE" modifyvm "$VM_NAME" \
    --natpf1 "ssh,tcp,,$SSH_PORT,,22" \
    --natpf1 "webui,tcp,,$WEB_PORT,,8340"

echo "  Port forwarding: SSH=$SSH_PORT→22, Web=$WEB_PORT→8340"

# ── Start VM ──
echo
echo "Starting VM..."
"$VBOXMANAGE" startvm "$VM_NAME"

echo
echo "═══════════════════════════════════════════════"
echo "  VM '$VM_NAME' is booting!"
echo "═══════════════════════════════════════════════"
echo
echo "  A VirtualBox window should appear with the"
echo "  Alpine Linux installer. Follow these steps:"
echo
echo "  ┌─────────────────────────────────────────┐"
echo "  │  1. Login: root (no password)           │"
echo "  │  2. Run:   setup-alpine                 │"
echo "  │     - Keyboard: us                      │"
echo "  │     - Hostname: yantrik                 │"
echo "  │     - Network:  eth0, dhcp              │"
echo "  │     - Password: root (or your choice)   │"
echo "  │     - Timezone: your timezone            │"
echo "  │     - Mirror:   1 (or nearest)          │"
echo "  │     - SSH:      openssh                 │"
echo "  │     - Disk:     sda, sys, y             │"
echo "  │  3. Run:   poweroff                     │"
echo "  │  4. Then:  ./deploy-vbox.sh             │"
echo "  └─────────────────────────────────────────┘"
echo
echo "  After Alpine is installed, deploy Yantrik:"
echo "    ./deploy-vbox.sh"
echo
echo "  SSH access (after install):"
echo "    ssh -p $SSH_PORT root@localhost"
echo
