#!/bin/bash
# Build and deploy Yantrik OS to VirtualBox VM
# Usage: ./deploy.sh [--skip-build]
#
# Prerequisites:
#   - WSL2 with Ubuntu and Rust toolchain
#   - VBox VM "Yantrik-OS" running with SSH on port 2222
#   - SSH key at ~/.ssh/id_deploy

set -euo pipefail

SSH_KEY="/c/Users/sync/.ssh/id_deploy"
SSH_COMMON="-o StrictHostKeyChecking=no -i $SSH_KEY"
SSH_OPTS="$SSH_COMMON -p 2222"
SCP_OPTS="$SSH_COMMON -P 2222"
SSH_HOST="root@127.0.0.1"
REMOTE_BIN="/opt/yantrik/bin"
STAGING="/c/Users/sync"
WSL_TARGET="/home/yantrik/target-yantrik"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

step() { echo -e "${GREEN}==> $1${NC}"; }
warn() { echo -e "${YELLOW}    $1${NC}"; }
fail() { echo -e "${RED}!!! $1${NC}"; exit 1; }

# Step 1: Build via WSL2
if [ "${1:-}" != "--skip-build" ]; then
    step "Clearing fingerprints (WSL timestamp sync)..."
    wsl.exe -d Ubuntu -- bash -lc \
        'rm -rf /home/yantrik/target-yantrik/release/.fingerprint/yantrikdb-companion-* \
                /home/yantrik/target-yantrik/release/.fingerprint/yantrik-ui-* \
                /home/yantrik/target-yantrik/release/.fingerprint/yantrik-[0-9a-f]*'

    step "Building via WSL2..."
    BUILD_LOG="$STAGING/yantrik-build.log"
    wsl.exe -d Ubuntu -- bash -lc \
        'cd /mnt/c/Users/sync/codes/yantrik-os && \
         RUSTFLAGS="-A warnings" CARGO_TARGET_DIR=/home/yantrik/target-yantrik \
         cargo build --release -p yantrik-ui -p yantrik 2>&1' | tee "$BUILD_LOG"
    BUILD_EXIT=${PIPESTATUS[0]}

    if [ "$BUILD_EXIT" -ne 0 ]; then
        fail "Build failed (exit code $BUILD_EXIT). See log above."
    fi

    # Verify binaries actually exist and were recently modified
    wsl.exe -d Ubuntu -- bash -lc \
        'test -f /home/yantrik/target-yantrik/release/yantrik-ui && \
         test -f /home/yantrik/target-yantrik/release/yantrik' \
        || fail "Build claimed success but binaries not found!"

    step "Build succeeded."
    rm -f "$BUILD_LOG"
else
    step "Skipping build (--skip-build)"
fi

# Step 2: Copy binaries from WSL to Windows staging area
step "Copying binaries to staging..."
wsl.exe -d Ubuntu -- bash -lc \
    'cp /home/yantrik/target-yantrik/release/yantrik-ui /mnt/c/Users/sync/yantrik-ui && \
     cp /home/yantrik/target-yantrik/release/yantrik /mnt/c/Users/sync/yantrik' \
    || fail "Failed to copy binaries from WSL to staging."

# Verify staging files exist
[ -f "$STAGING/yantrik-ui" ] || fail "Staged yantrik-ui not found at $STAGING"
[ -f "$STAGING/yantrik" ] || fail "Staged yantrik not found at $STAGING"

# Step 3: SCP to VBox
step "Deploying to VBox..."
scp $SCP_OPTS \
    "$STAGING/yantrik-ui" "$STAGING/yantrik" \
    "$SSH_HOST:/usr/local/bin/" \
    || fail "SCP to VM failed. Is the VM running? (port 2222)"

# Step 3b: Deploy i18n translation files
I18N_SRC="/c/Users/sync/codes/yantrik-os/crates/yantrik-ui/i18n"
if [ -d "$I18N_SRC" ]; then
    step "Deploying i18n translations..."
    ssh $SSH_OPTS $SSH_HOST "mkdir -p /opt/yantrik/bin/i18n"
    scp $SCP_OPTS "$I18N_SRC"/*.yaml "$SSH_HOST:/opt/yantrik/bin/i18n/" \
        || warn "Failed to deploy i18n files (non-fatal)"
fi

# Step 4: Kill old process, update binaries, restart
step "Restarting yantrik-ui..."
# Kill old process
ssh $SSH_OPTS $SSH_HOST "
    for pid in \$(pgrep -f '/opt/yantrik/bin/yantrik-ui' 2>/dev/null); do
        kill \$pid 2>/dev/null
    done
    sleep 2
    for pid in \$(pgrep -f '/opt/yantrik/bin/yantrik-ui' 2>/dev/null); do
        kill -9 \$pid 2>/dev/null
    done
" || true
sleep 1

# Copy to canonical location
ssh $SSH_OPTS $SSH_HOST "
    cp /usr/local/bin/yantrik-ui $REMOTE_BIN/yantrik-ui &&
    cp /usr/local/bin/yantrik    $REMOTE_BIN/yantrik &&
    chmod +x $REMOTE_BIN/yantrik-ui $REMOTE_BIN/yantrik
" || fail "Failed to copy binaries on VM."

# Start new process
ssh $SSH_OPTS $SSH_HOST "
    su - yantrik -c '
        WAYLAND_DISPLAY=wayland-0 \
        XDG_RUNTIME_DIR=/run/user/1000 \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
        SLINT_BACKEND=winit-software \
        LD_PRELOAD=/usr/lib/libgcompat_shim.so \
        nohup /opt/yantrik/bin/yantrik-ui /opt/yantrik/config.yaml \
            >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
    '
" || fail "Failed to start yantrik-ui on VM."

# Step 5: Verify
sleep 3
step "Verifying..."
PROCS=$(ssh $SSH_OPTS $SSH_HOST "ps aux | grep yantrik-ui | grep -v grep | wc -l")
if [ "$PROCS" -eq 1 ]; then
    step "Deploy successful! yantrik-ui running. (PID verified)"
elif [ "$PROCS" -gt 1 ]; then
    warn "Warning: $PROCS instances running. Kill extras manually."
else
    fail "yantrik-ui not running after deploy. Check: ssh -p 2222 root@127.0.0.1 'tail -50 /opt/yantrik/logs/yantrik-os.log'"
fi

# Cleanup staging
rm -f "$STAGING/yantrik-ui" "$STAGING/yantrik" 2>/dev/null
step "Done."
