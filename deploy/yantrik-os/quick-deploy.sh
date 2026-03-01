#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# quick-deploy.sh — Build + deploy yantrik-ui to running QEMU VM
# ═══════════════════════════════════════════════════════════════
#
# Run from WSL2 host. Assumes:
#   - QEMU VM is running with port 2222 forwarded to SSH
#   - Root password is "root" (dev VM)
#   - Ollama is running on the host (port 11434)
#
# Usage:
#   ./quick-deploy.sh              # build + deploy with Ollama backend
#   ./quick-deploy.sh --candle     # build + deploy with Candle backend
#   ./quick-deploy.sh --skip-build # deploy without rebuilding
#
# What it does:
#   1. Cross-compiles yantrik-ui (release, ~3 min)
#   2. SCPs binary to VM at /opt/yantrik/bin/yantrik-ui
#   3. SCPs config (ollama or candle) to /opt/yantrik/config.yaml
#   4. Kills any running yantrik-ui / test-brain on the VM
#   5. Restarts labwc (which auto-starts yantrik-ui)
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_DIR="$PROJECT_ROOT/config"

SSH_PORT=2222
SSH_HOST="root@localhost"
SSH_CMD="sshpass -p root ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 -p $SSH_PORT $SSH_HOST"
SCP_CMD="sshpass -p root scp -o StrictHostKeyChecking=no -P $SSH_PORT"

BACKEND="ollama"
SKIP_BUILD=false
BUILD_FEATURES="api-llm"
TARGET_DIR="${CARGO_TARGET_DIR:-/home/yantrik/target-yantrik}"

for arg in "$@"; do
    case "$arg" in
        --candle)   BACKEND="candle"; BUILD_FEATURES="cuda,api-llm" ;;
        --skip-build) SKIP_BUILD=true ;;
        --help|-h)
            echo "Usage: $0 [--candle] [--skip-build]"
            exit 0
            ;;
    esac
done

echo "═══════════════════════════════════════════════"
echo "  Yantrik OS — Quick Deploy"
echo "═══════════════════════════════════════════════"
echo
echo "  Backend:  $BACKEND"
echo "  Build:    $(if $SKIP_BUILD; then echo "SKIP"; else echo "release"; fi)"
echo

# ── Step 0: Check VM is reachable ──
echo "[0/5] Checking VM connectivity..."
if ! $SSH_CMD "echo ok" >/dev/null 2>&1; then
    echo "ERROR: Cannot reach VM at localhost:$SSH_PORT"
    echo "  Boot the VM first: ./boot-desktop.sh"
    exit 1
fi
echo "  VM is alive."

# ── Step 1: Build ──
if ! $SKIP_BUILD; then
    echo
    echo "[1/5] Building yantrik-ui (release, features: $BUILD_FEATURES)..."
    echo "  This may take a few minutes on first build."
    cd "$PROJECT_ROOT"
    CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release -p yantrik-ui --features "$BUILD_FEATURES"
    echo "  Build complete."
else
    echo "[1/5] Skipping build (--skip-build)."
fi

BINARY="$TARGET_DIR/release/yantrik-ui"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    exit 1
fi

BINARY_SIZE=$(du -h "$BINARY" | cut -f1)
echo "  Binary size: $BINARY_SIZE"

# ── Step 2: Kill running instances ──
echo
echo "[2/5] Stopping running yantrik processes on VM..."
$SSH_CMD "pkill -f yantrik-ui 2>/dev/null; pkill -f test-brain 2>/dev/null; sleep 1" || true
echo "  Done."

# ── Step 3: Upload binary ──
echo
echo "[3/5] Uploading binary to VM..."
$SCP_CMD "$BINARY" "$SSH_HOST:/opt/yantrik/bin/yantrik-ui"
$SSH_CMD "chmod +x /opt/yantrik/bin/yantrik-ui"
echo "  Uploaded."

# ── Step 4: Upload config ──
echo
echo "[4/5] Uploading config ($BACKEND backend)..."
if [ "$BACKEND" = "ollama" ]; then
    $SCP_CMD "$CONFIG_DIR/yantrik-ollama.yaml" "$SSH_HOST:/opt/yantrik/config.yaml"
else
    $SCP_CMD "$CONFIG_DIR/yantrik-os.yaml" "$SSH_HOST:/opt/yantrik/config.yaml"
fi
echo "  Config deployed."

# ── Step 5: Restart desktop ──
echo
echo "[5/5] Restarting Yantrik OS desktop..."
# Kill labwc (will be restarted by agetty → .profile → yantrik-start.sh)
$SSH_CMD "pkill labwc 2>/dev/null || true"
sleep 2
echo "  Desktop restarting. It may take a few seconds to appear."

echo
echo "═══════════════════════════════════════════════"
echo "  Deploy complete!"
echo "═══════════════════════════════════════════════"
echo
echo "  Backend: $BACKEND"
if [ "$BACKEND" = "ollama" ]; then
    echo "  Ensure Ollama is running: ollama serve"
    echo "  Model needed: qwen2.5:3b-instruct-q4_K_M"
    echo "    ollama pull qwen2.5:3b-instruct-q4_K_M"
fi
echo
echo "  SSH into VM:  sshpass -p root ssh -p 2222 root@localhost"
echo "  View logs:    ... tail -f /opt/yantrik/logs/yantrik-os.log"
