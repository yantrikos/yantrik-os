#!/bin/bash
# Build and deploy Yantrik OS within WSL Ubuntu
# Usage: ./deploy.sh [--skip-build] [--debug]
#
# Prerequisites:
#   - WSL2 with Ubuntu and Rust toolchain
#   - sccache installed (cargo install sccache)

set -euo pipefail

REMOTE_BIN="/opt/yantrik/bin"
WSL_TARGET="/home/yantrik/target-yantrik"
WSL_SRC="/home/yantrik/src/yantrik-os"
WIN_SRC="/mnt/c/Users/sync/codes/yantrik-os"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

step() { echo -e "${GREEN}==> $1${NC}"; }
warn() { echo -e "${YELLOW}    $1${NC}"; }
fail() { echo -e "${RED}!!! $1${NC}"; exit 1; }

# Determine build profile
PROFILE="release"
PROFILE_FLAG="--release"
if [ "${1:-}" = "--debug" ] || [ "${2:-}" = "--debug" ]; then
    PROFILE="debug"
    PROFILE_FLAG=""
fi

# Step 1: Build via WSL2
if [ "${1:-}" != "--skip-build" ]; then
    step "Syncing source to native FS..."
    wsl.exe -d Ubuntu -- bash -lc \
        "rsync -a --delete $WIN_SRC/ $WSL_SRC/ \
            --exclude target --exclude .git/objects --exclude .claude/worktrees \
            --exclude '*.gguf' --exclude training/"

    step "Building ($PROFILE) via WSL2..."
    wsl.exe -d Ubuntu -- bash -lc \
        "cd $WSL_SRC && \
         export RUSTC_WRAPPER=sccache 2>/dev/null || true && \
         RUSTFLAGS=\"-A warnings\" CARGO_TARGET_DIR=$WSL_TARGET \
         cargo build $PROFILE_FLAG \
            -p yantrik-ui -p yantrik \
            -p weather-service -p system-monitor-service -p notes-service \
            -p notifications-service -p calendar-service -p network-service \
            -p email-service \
         2>&1"

    # Verify binaries exist
    wsl.exe -d Ubuntu -- bash -lc \
        "test -f $WSL_TARGET/$PROFILE/yantrik-ui && \
         test -f $WSL_TARGET/$PROFILE/yantrik" \
        || fail "Build claimed success but binaries not found!"

    step "Build succeeded."
else
    step "Skipping build (--skip-build)"
fi

# Step 2: Deploy binaries within WSL
step "Deploying binaries..."
wsl.exe -d Ubuntu -- bash -lc "
    sudo mkdir -p $REMOTE_BIN &&
    sudo cp $WSL_TARGET/$PROFILE/yantrik-ui $REMOTE_BIN/yantrik-ui &&
    sudo cp $WSL_TARGET/$PROFILE/yantrik    $REMOTE_BIN/yantrik &&
    sudo chmod +x $REMOTE_BIN/yantrik-ui $REMOTE_BIN/yantrik
" || fail "Failed to deploy binaries."

# Step 2a: Deploy service binaries
step "Deploying services..."
SERVICES="weather-service system-monitor-service notes-service notifications-service calendar-service network-service email-service"
wsl.exe -d Ubuntu -- bash -lc "
    for svc in $SERVICES; do
        if [ -f $WSL_TARGET/$PROFILE/\$svc ]; then
            sudo cp $WSL_TARGET/$PROFILE/\$svc $REMOTE_BIN/\$svc &&
            sudo chmod +x $REMOTE_BIN/\$svc
        fi
    done
" || warn "Failed to deploy some services (non-fatal)"

# Step 2b: Deploy i18n translation files
I18N_SRC="$WSL_SRC/crates/yantrik-ui/i18n"
wsl.exe -d Ubuntu -- bash -lc "
    if [ -d '$I18N_SRC' ]; then
        sudo mkdir -p $REMOTE_BIN/i18n &&
        sudo cp $I18N_SRC/*.yaml $REMOTE_BIN/i18n/ 2>/dev/null
    fi
" || warn "Failed to deploy i18n files (non-fatal)"

# Step 2c: Deploy skill manifests
SKILLS_SRC="$WSL_SRC/skills"
wsl.exe -d Ubuntu -- bash -lc "
    if [ -d '$SKILLS_SRC' ]; then
        sudo mkdir -p /opt/yantrik/skills &&
        sudo cp $SKILLS_SRC/*.yaml /opt/yantrik/skills/ 2>/dev/null
    fi
" || warn "Failed to deploy skill manifests (non-fatal)"

# Step 3: Restart yantrik-ui
step "Restarting yantrik-ui..."
wsl.exe -d Ubuntu -- bash -lc \
    "pgrep -f '/opt/yantrik/bin/yantrik-ui' | xargs -r kill 2>/dev/null; sleep 2; pgrep -f '/opt/yantrik/bin/yantrik-ui' | xargs -r kill -9 2>/dev/null" \
    || true
sleep 1

# Start new process
wsl.exe -d Ubuntu -- bash -lc \
    "sudo -u yantrik bash -c '
        WAYLAND_DISPLAY=wayland-0 \
        XDG_RUNTIME_DIR=/run/user/1000 \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
        SLINT_BACKEND=winit-software \
        LD_PRELOAD=\"/lib/libgcompat.so.0 /usr/lib/libgcompat_shim.so\" \
        nohup /opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml \
            >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
    '" || fail "Failed to start yantrik-ui."

# Step 4: Verify
sleep 3
step "Verifying..."
PROCS=$(wsl.exe -d Ubuntu -- bash -lc "ps aux | grep yantrik-ui | grep -v grep | wc -l")
if [ "$PROCS" -eq 1 ]; then
    step "Deploy successful! yantrik-ui running. (PID verified)"
elif [ "$PROCS" -gt 1 ]; then
    warn "Warning: $PROCS instances running. Kill extras manually."
else
    fail "yantrik-ui not running after deploy. Check: wsl.exe -d Ubuntu -- tail -50 /opt/yantrik/logs/yantrik-os.log"
fi

step "Done."
