#!/bin/bash
# Deploy Yantrik OS within WSL (no SSH, no VBox)
# Usage: bash deploy-wsl.sh [--skip-build]

set -euo pipefail

WSL_TARGET="/home/yantrik/target-yantrik"
REMOTE_BIN="/opt/yantrik/bin"
CONFIG="/opt/yantrik/config.yaml"
LOG="/opt/yantrik/logs/yantrik-os.log"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

step() { echo -e "${GREEN}==> $1${NC}"; }
warn() { echo -e "${YELLOW}    $1${NC}"; }
fail() { echo -e "${RED}!!! $1${NC}"; exit 1; }

# Step 1: Build via WSL2
if [ "${1:-}" != "--skip-build" ]; then
    step "Clearing fingerprints..."
    wsl.exe -d Ubuntu -- bash -lc \
        'rm -rf /home/yantrik/target-yantrik/release/.fingerprint/yantrik-companion-* \
                /home/yantrik/target-yantrik/release/.fingerprint/yantrik-ml-* \
                /home/yantrik/target-yantrik/release/.fingerprint/yantrik-ui-* \
                /home/yantrik/target-yantrik/release/.fingerprint/yantrik-[0-9a-f]*'

    step "Building via WSL2..."
    wsl.exe -d Ubuntu -- bash -lc \
        'cd /mnt/c/Users/sync/codes/yantrik-os && \
         RUSTFLAGS="-A warnings" CARGO_TARGET_DIR=/home/yantrik/target-yantrik \
         cargo build --release -p yantrik-ui -p yantrik 2>&1'

    wsl.exe -d Ubuntu -- bash -lc \
        'test -f /home/yantrik/target-yantrik/release/yantrik-ui && \
         test -f /home/yantrik/target-yantrik/release/yantrik' \
        || fail "Build claimed success but binaries not found!"

    step "Build succeeded."
else
    step "Skipping build (--skip-build)"
fi

# Step 2: Kill old process
step "Stopping yantrik-ui..."
wsl.exe -d Ubuntu -- bash -lc "
    for pid in \$(pgrep -f '/opt/yantrik/bin/yantrik-ui' 2>/dev/null); do
        kill \$pid 2>/dev/null || true
    done
    sleep 2
    for pid in \$(pgrep -f '/opt/yantrik/bin/yantrik-ui' 2>/dev/null); do
        kill -9 \$pid 2>/dev/null || true
    done
" || true
sleep 1

# Step 3: Copy new binaries
step "Deploying binaries..."
wsl.exe -d Ubuntu -- bash -lc "
    cp $WSL_TARGET/release/yantrik-ui $REMOTE_BIN/yantrik-ui && \
    cp $WSL_TARGET/release/yantrik    $REMOTE_BIN/yantrik && \
    chmod +x $REMOTE_BIN/yantrik-ui $REMOTE_BIN/yantrik
" || fail "Failed to copy binaries."

# Step 3b: Deploy i18n
I18N_SRC="/mnt/c/Users/sync/codes/yantrik-os/crates/yantrik-ui/i18n"
wsl.exe -d Ubuntu -- bash -lc "
    if [ -d '$I18N_SRC' ]; then
        mkdir -p $REMOTE_BIN/i18n
        cp $I18N_SRC/*.yaml $REMOTE_BIN/i18n/ 2>/dev/null || true
    fi
"

# Step 3c: Deploy skills
SKILLS_SRC="/mnt/c/Users/sync/codes/yantrik-os/skills"
wsl.exe -d Ubuntu -- bash -lc "
    if [ -d '$SKILLS_SRC' ]; then
        mkdir -p /opt/yantrik/skills
        cp $SKILLS_SRC/*.yaml /opt/yantrik/skills/ 2>/dev/null || true
    fi
"

# Step 4: Start new process
step "Starting yantrik-ui..."
wsl.exe -d Ubuntu -- bash -lc "
    su - yantrik -c '
        DISPLAY=:0 \
        WAYLAND_DISPLAY=wayland-0 \
        XDG_RUNTIME_DIR=/run/user/1000 \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
        SLINT_BACKEND=winit-software \
        LD_PRELOAD=/usr/lib/libgcompat_shim.so \
        nohup $REMOTE_BIN/yantrik-ui $CONFIG >> $LOG 2>&1 &
    '
"

# Step 5: Verify
sleep 3
step "Verifying..."
PROCS=$(wsl.exe -d Ubuntu -- bash -lc "ps aux | grep 'yantrik-ui.*config' | grep -v grep | wc -l")
if [ "$PROCS" -ge 1 ]; then
    PID=$(wsl.exe -d Ubuntu -- bash -lc "pgrep -f '/opt/yantrik/bin/yantrik-ui' | head -1")
    step "Deploy successful! yantrik-ui running (PID $PID)"
    # Show last few log lines
    wsl.exe -d Ubuntu -- bash -lc "tail -3 $LOG"
else
    fail "yantrik-ui not running after deploy. Check: wsl.exe -d Ubuntu -- bash -lc 'tail -50 $LOG'"
fi

step "Done."
