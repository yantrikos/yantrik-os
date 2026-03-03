#!/bin/bash
# Build and deploy Yantrik OS to VirtualBox VM
# Usage: ./deploy.sh [--skip-build]
#
# Prerequisites:
#   - WSL2 with Ubuntu and Rust toolchain
#   - VBox VM "Yantrik-OS" running with SSH on port 2222
#   - SSH key at ~/.ssh/id_deploy

set -e

SSH_KEY="/c/Users/sync/.ssh/id_deploy"
SSH_COMMON="-o StrictHostKeyChecking=no -i $SSH_KEY"
SSH_OPTS="$SSH_COMMON -p 2222"
SCP_OPTS="$SSH_COMMON -P 2222"
SSH_HOST="root@127.0.0.1"
REMOTE_BIN="/opt/yantrik/bin"
STAGING="/c/Users/sync"

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

step() { echo -e "${GREEN}==> $1${NC}"; }
warn() { echo -e "${YELLOW}    $1${NC}"; }

# Step 1: Build via WSL2
if [ "$1" != "--skip-build" ]; then
    step "Building via WSL2..."
    wsl.exe -d Ubuntu -- bash -lc \
        'cd /mnt/c/Users/sync/OneDrive/Documents/GitHub/yantrik-os && \
         RUSTFLAGS="-A warnings" CARGO_TARGET_DIR=/home/yantrik/target-yantrik \
         cargo build --release -p yantrik-ui -p yantrik 2>&1 | tail -5'
    echo ""
else
    step "Skipping build (--skip-build)"
fi

# Step 2: Copy binaries from WSL to Windows staging area
step "Copying binaries to staging..."
wsl.exe -d Ubuntu -- bash -lc \
    'cp /home/yantrik/target-yantrik/release/yantrik-ui /mnt/c/Users/sync/yantrik-ui && \
     cp /home/yantrik/target-yantrik/release/yantrik /mnt/c/Users/sync/yantrik'

# Step 3: SCP to VBox
step "Deploying to VBox..."
scp $SCP_OPTS \
    "$STAGING/yantrik-ui" "$STAGING/yantrik" \
    "$SSH_HOST:/usr/local/bin/"

# Step 4: Kill old process, update binaries, restart
step "Restarting yantrik-ui..."
ssh $SSH_OPTS $SSH_HOST "
    # Kill existing yantrik-ui (Alpine musl wrapper process name)
    for pid in \$(ps aux | grep 'yantrik-ui' | grep -v grep | awk '{print \$1}'); do
        kill \$pid 2>/dev/null
    done
    sleep 1

    # Copy to canonical location
    cp /usr/local/bin/yantrik-ui $REMOTE_BIN/yantrik-ui
    cp /usr/local/bin/yantrik    $REMOTE_BIN/yantrik
    chmod +x $REMOTE_BIN/yantrik-ui $REMOTE_BIN/yantrik

    # Restart as yantrik user with correct env
    su - yantrik -c '
        WAYLAND_DISPLAY=wayland-0 \
        XDG_RUNTIME_DIR=/run/user/1000 \
        DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
        SLINT_BACKEND=winit-software \
        LD_PRELOAD=/usr/lib/libgcompat_shim.so \
        nohup $REMOTE_BIN/yantrik-ui /opt/yantrik/config.yaml \
            >> /opt/yantrik/logs/yantrik-os.log 2>&1 &
    '
"

# Step 5: Verify
sleep 3
step "Verifying..."
PROCS=$(ssh $SSH_OPTS $SSH_HOST "ps aux | grep yantrik-ui | grep -v grep | wc -l")
if [ "$PROCS" -eq 1 ]; then
    step "Deploy successful! yantrik-ui running."
elif [ "$PROCS" -gt 1 ]; then
    warn "Warning: $PROCS instances running. Kill extras manually."
else
    warn "Warning: yantrik-ui not detected. Check /opt/yantrik/logs/yantrik-os.log"
fi

# Cleanup staging
rm -f "$STAGING/yantrik-ui" "$STAGING/yantrik" 2>/dev/null
step "Done."
