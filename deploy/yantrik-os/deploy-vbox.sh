#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# deploy-vbox.sh — Deploy Yantrik OS to a VirtualBox Alpine VM
# ═══════════════════════════════════════════════════════════════
#
# Run AFTER Alpine is installed in the VBox VM (via setup-vbox.sh).
# This script:
#   1. Builds the yantrik-ui binary (in WSL2)
#   2. Uploads binary + deploy-stack.sh to the VM
#   3. Runs deploy-stack.sh on the VM (installs everything)
#   4. Removes the Alpine ISO from the VM's DVD drive
#   5. Sets boot order to HDD first
#
# After this: power off and restart the VM → Yantrik OS desktop!
#
# Usage:
#   ./deploy-vbox.sh                    # build + deploy
#   ./deploy-vbox.sh --skip-build       # deploy pre-built binary
#   ./deploy-vbox.sh --password mypass  # custom root password
#   ./deploy-vbox.sh --name MyVM        # custom VM name
#
# Prerequisites:
#   - VirtualBox VM running with Alpine installed
#   - SSH accessible on port 2222 (NAT forwarding from setup-vbox.sh)
#   - sshpass installed (apt install sshpass)
#   - WSL2 with Rust toolchain (for building)
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_DIR="$PROJECT_ROOT/config"

# ── Configuration ──
VM_NAME="Yantrik-OS"
SSH_PORT=2222
ROOT_PASS="root"
SKIP_BUILD=false
BUILD_FEATURES="api-llm"
TARGET_DIR="${CARGO_TARGET_DIR:-/home/yantrik/target-yantrik}"

# ── Parse flags ──
while [ $# -gt 0 ]; do
    case "$1" in
        --skip-build) SKIP_BUILD=true; shift ;;
        --password)   ROOT_PASS="$2"; shift 2 ;;
        --name)       VM_NAME="$2"; shift 2 ;;
        --candle)     BUILD_FEATURES="cuda,api-llm"; shift ;;
        --help|-h)
            echo "Usage: $0 [--skip-build] [--password PASS] [--name VM] [--candle]"
            exit 0
            ;;
        *) echo "Unknown flag: $1"; exit 1 ;;
    esac
done

SSH_HOST="root@localhost"
SSH_OPTS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10"
SSH_CMD="sshpass -p $ROOT_PASS ssh $SSH_OPTS -p $SSH_PORT $SSH_HOST"
SCP_CMD="sshpass -p $ROOT_PASS scp $SSH_OPTS -P $SSH_PORT"

# ── Detect VBoxManage ──
VBOXMANAGE=""
for candidate in \
    "VBoxManage" \
    "VBoxManage.exe" \
    "/c/Program Files/Oracle/VirtualBox/VBoxManage.exe" \
    "/mnt/c/Program Files/Oracle/VirtualBox/VBoxManage.exe"; do
    if command -v "$candidate" &>/dev/null 2>&1; then
        VBOXMANAGE="$candidate"
        break
    fi
    if [ -f "$candidate" ]; then
        VBOXMANAGE="$candidate"
        break
    fi
done

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — VirtualBox Deployment"
echo "═══════════════════════════════════════════════"
echo
echo "  VM:       $VM_NAME"
echo "  SSH:      localhost:$SSH_PORT"
echo "  Build:    $(if $SKIP_BUILD; then echo "SKIP"; else echo "release ($BUILD_FEATURES)"; fi)"
echo

# ── Check prerequisites ──
if ! command -v sshpass &>/dev/null; then
    echo "ERROR: sshpass not found."
    echo "  Install: sudo apt install sshpass"
    exit 1
fi

# ── Step 0: Check VM is reachable ──
echo "[0/6] Checking VM connectivity..."
if ! $SSH_CMD "echo ok" >/dev/null 2>&1; then
    echo "ERROR: Cannot reach VM at localhost:$SSH_PORT"
    echo
    echo "  Make sure:"
    echo "    1. The VM is running (check VirtualBox Manager)"
    echo "    2. Alpine is installed and booted from disk"
    echo "    3. SSH is enabled: rc-update add sshd default && rc-service sshd start"
    echo "    4. Root password matches (--password flag)"
    exit 1
fi
echo "  VM is reachable."

# ── Step 1: Enable community repo + install SSH (if needed) ──
echo
echo "[1/6] Preparing VM..."
$SSH_CMD "
    # Enable community repo
    if grep -q '^#.*community' /etc/apk/repositories 2>/dev/null; then
        sed -i 's|^#\(.*community\)|\1|' /etc/apk/repositories
    fi
    apk update -q
    # Ensure openssh is installed and running
    apk add -q openssh curl
    rc-update add sshd default 2>/dev/null || true
    rc-service sshd start 2>/dev/null || true
"
echo "  VM prepared."

# ── Step 2: Build binary ──
if ! $SKIP_BUILD; then
    echo
    echo "[2/6] Building yantrik-ui (release, features: $BUILD_FEATURES)..."
    echo "  This may take several minutes on first build."
    cd "$PROJECT_ROOT"
    CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release -p yantrik-ui --features "$BUILD_FEATURES"
    echo "  Build complete."
else
    echo
    echo "[2/6] Skipping build (--skip-build)."
fi

BINARY="$TARGET_DIR/release/yantrik-ui"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    echo "  Build first: cargo build --release -p yantrik-ui --features api-llm"
    exit 1
fi
BINARY_SIZE=$(du -h "$BINARY" | cut -f1)
echo "  Binary: $BINARY ($BINARY_SIZE)"

# ── Step 3: Upload binary ──
echo
echo "[3/6] Uploading binary to VM (this may take a minute)..."
$SCP_CMD "$BINARY" "$SSH_HOST:/tmp/yantrik-ui"
$SSH_CMD "chmod +x /tmp/yantrik-ui"
echo "  Binary uploaded to /tmp/yantrik-ui"

# ── Step 4: Upload deploy-stack.sh + config ──
echo
echo "[4/6] Uploading deploy script and config..."
$SCP_CMD "$SCRIPT_DIR/deploy-stack.sh" "$SSH_HOST:/tmp/deploy-stack.sh"
$SSH_CMD "chmod +x /tmp/deploy-stack.sh"
echo "  deploy-stack.sh uploaded."

# ── Step 5: Run deploy-stack.sh on VM ──
echo
echo "[5/6] Running deploy-stack.sh on VM..."
echo "  This will install packages, download models (~1.8GB), and configure the desktop."
echo "  This takes 5-15 minutes depending on internet speed."
echo
$SSH_CMD "cd /tmp && ./deploy-stack.sh" 2>&1 | while IFS= read -r line; do
    echo "  [VM] $line"
done
echo
echo "  Deploy stack complete."

# ── Step 6: Detach ISO + set boot order ──
if [ -n "$VBOXMANAGE" ]; then
    echo
    echo "[6/6] Configuring VM boot order..."

    # Check if VM is running — need to power off to change settings
    VM_STATE=$("$VBOXMANAGE" showvminfo "$VM_NAME" --machinereadable 2>/dev/null | grep "VMState=" | cut -d'"' -f2 || echo "unknown")

    if [ "$VM_STATE" = "running" ]; then
        echo "  VM is running. Shutting down gracefully..."
        $SSH_CMD "poweroff" 2>/dev/null || true
        echo "  Waiting for VM to power off..."
        sleep 5
        # Wait up to 30 seconds for clean shutdown
        for i in $(seq 1 12); do
            VM_STATE=$("$VBOXMANAGE" showvminfo "$VM_NAME" --machinereadable 2>/dev/null | grep "VMState=" | cut -d'"' -f2 || echo "unknown")
            if [ "$VM_STATE" != "running" ]; then
                break
            fi
            sleep 3
        done
        # Force off if still running
        if [ "$VM_STATE" = "running" ]; then
            "$VBOXMANAGE" controlvm "$VM_NAME" poweroff 2>/dev/null || true
            sleep 2
        fi
    fi

    # Remove ISO from DVD drive
    "$VBOXMANAGE" storageattach "$VM_NAME" --storagectl "IDE" --port 0 --device 0 --type dvddrive --medium emptydrive 2>/dev/null || true

    # Set boot order: HDD first
    "$VBOXMANAGE" modifyvm "$VM_NAME" \
        --boot1 disk \
        --boot2 dvd \
        --boot3 none \
        --boot4 none 2>/dev/null || true

    echo "  ISO detached, boot order set to HDD first."

    # Start VM
    echo
    echo "  Starting Yantrik OS..."
    "$VBOXMANAGE" startvm "$VM_NAME"
else
    echo
    echo "[6/6] VBoxManage not found — manual steps needed:"
    echo "  1. Power off the VM"
    echo "  2. In VirtualBox: Settings → Storage → Remove Alpine ISO"
    echo "  3. Settings → System → Boot Order → move Hard Disk to top"
    echo "  4. Start the VM"
fi

echo
echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Deployment Complete!"
echo "═══════════════════════════════════════════════"
echo
echo "  The VM should boot into the Yantrik OS desktop."
echo "  If it boots to a login prompt instead, wait a"
echo "  moment — labwc starts automatically on tty1."
echo
echo "  Access:"
echo "    SSH:      ssh -p $SSH_PORT root@localhost"
echo
echo "  Quick redeploy (after code changes):"
echo "    ./quick-deploy.sh  (works for VBox too)"
echo
