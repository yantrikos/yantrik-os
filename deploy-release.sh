#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Yantrik OS — Release Publisher
# ═══════════════════════════════════════════════════════════════
#
# Builds Yantrik binaries and publishes them to releases.yantrikos.com
#
# Usage:
#   ./deploy-release.sh nightly          Build + push to nightly channel
#   ./deploy-release.sh beta             Promote nightly → beta
#   ./deploy-release.sh stable           Promote beta → stable
#   ./deploy-release.sh nightly --skip-build   Push existing binaries
#
# The script:
#   1. Builds via WSL2 (unless --skip-build)
#   2. Generates SHA256 checksums
#   3. Uploads binaries to releases.yantrikos.com
#   4. Updates manifest.json with new version info
#
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

# ── Config ──
RELEASES_HOST="releases.yantrikos.com"
RELEASES_IP="192.168.4.28"
SSH_KEY="/c/Users/sync/.ssh/id_deploy"
SSH_OPTS="-o StrictHostKeyChecking=no -i $SSH_KEY"
WSL_TARGET="/home/yantrik/target-yantrik"
STAGING="/c/tmp/yantrik-release"

# Colors
CYAN='\033[0;36m'
GREEN='\033[0;32m'
AMBER='\033[0;33m'
RED='\033[0;31m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

step()  { echo -e "${CYAN}::${NC} ${BOLD}$1${NC}"; }
ok()    { echo -e "   ${GREEN}✓${NC} $1"; }
warn()  { echo -e "   ${AMBER}!${NC} $1"; }
fail()  { echo -e "   ${RED}✗${NC} $1"; exit 1; }

# ── Parse args ──
CHANNEL="${1:-}"
SKIP_BUILD=false
if [ "${2:-}" = "--skip-build" ]; then
    SKIP_BUILD=true
fi

if [ -z "$CHANNEL" ]; then
    echo -e "${BOLD}Usage:${NC} $0 <channel> [--skip-build]"
    echo
    echo "  Channels:"
    echo "    nightly   — Build and push latest (default for dev)"
    echo "    beta      — Promote nightly to beta"
    echo "    stable    — Promote beta to stable"
    echo
    exit 1
fi

case "$CHANNEL" in
    nightly|beta|stable) ;;
    *) fail "Unknown channel: $CHANNEL (use nightly, beta, or stable)" ;;
esac

echo
echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
echo -e "${BOLD}  Yantrik OS — Release Publisher${NC}"
echo -e "${DIM}  Channel: ${CHANNEL}${NC}"
echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
echo

# ── Get version from Cargo.toml ──
VERSION=$(grep '^version' crates/yantrik-ui/Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
GIT_HASH=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
BUILD_DATE=$(date +%Y-%m-%d)

if [ "$CHANNEL" = "nightly" ]; then
    BUILD_NUM=$(date +%Y%m%d%H%M)
    FULL_VERSION="${VERSION}-nightly.${BUILD_NUM}"
else
    FULL_VERSION="$VERSION"
fi

# ── Collect component versions ──
read_cargo_version() {
    grep '^version' "$1" 2>/dev/null | head -1 | sed 's/.*"\(.*\)"/\1/' || echo "0.0.0"
}

COMP_ML_VER=$(read_cargo_version crates/yantrik-ml/Cargo.toml)
COMP_DB_VER=$(read_cargo_version crates/yantrikdb-core/Cargo.toml)
COMP_COMPANION_VER=$(read_cargo_version crates/yantrik-companion/Cargo.toml)
COMP_OS_VER=$(read_cargo_version crates/yantrik-os/Cargo.toml)
COMP_UI_VER=$(read_cargo_version crates/yantrik-ui/Cargo.toml)

step "Version: $FULL_VERSION ($GIT_HASH)"
ok "yantrik-ml: $COMP_ML_VER"
ok "yantrikdb: $COMP_DB_VER"
ok "yantrik-companion: $COMP_COMPANION_VER"
ok "yantrik-os: $COMP_OS_VER"
ok "yantrik-ui: $COMP_UI_VER"

# ═══════════════════════════════════════════════════════════════
# STEP 1: Build (nightly only, unless --skip-build)
# ═══════════════════════════════════════════════════════════════
if [ "$CHANNEL" = "nightly" ] && [ "$SKIP_BUILD" = false ]; then
    step "Building via WSL2..."

    # Clear fingerprints
    wsl.exe -d Ubuntu -- bash -lc \
        "rm -rf $WSL_TARGET/release/.fingerprint/yantrik-companion-* \
                $WSL_TARGET/release/.fingerprint/yantrik-ui-* \
                $WSL_TARGET/release/.fingerprint/yantrik-[0-9a-f]*"

    # Build 1: Default (CPU-only, works everywhere)
    step "Building CPU variant..."
    wsl.exe -d Ubuntu -- bash -lc \
        "cd /mnt/c/Users/sync/codes/yantrik-os && \
         RUSTFLAGS=\"-A warnings\" CARGO_TARGET_DIR=$WSL_TARGET \
         cargo build --release -p yantrik-ui -p yantrik \
            -p weather-service -p system-monitor-service -p notes-service \
            -p notifications-service -p calendar-service -p network-service \
            -p email-service \
            -p yantrik-notes -p yantrik-email -p yantrik-calendar \
            -p yantrik-weather -p yantrik-system-monitor -p yantrik-terminal \
            -p yantrik-music-player -p yantrik-text-editor -p yantrik-image-viewer \
            -p yantrik-spreadsheet -p yantrik-document-editor -p yantrik-presentation \
            -p yantrik-network-manager -p yantrik-container-manager \
            -p yantrik-download-manager -p yantrik-snippet-manager 2>&1" \
        || fail "CPU build failed"

    wsl.exe -d Ubuntu -- bash -lc \
        "test -f $WSL_TARGET/release/yantrik-ui && \
         test -f $WSL_TARGET/release/yantrik" \
        || fail "CPU binaries not found"
    ok "CPU build succeeded"

    # Copy CPU binaries + services to staging
    mkdir -p "$STAGING"
    wsl.exe -d Ubuntu -- bash -lc \
        "cp $WSL_TARGET/release/yantrik-ui /mnt/c/tmp/yantrik-release/yantrik-ui && \
         cp $WSL_TARGET/release/yantrik    /mnt/c/tmp/yantrik-release/yantrik"

    # Copy service binaries to staging
    SERVICES="weather-service system-monitor-service notes-service notifications-service calendar-service network-service email-service"
    for svc in $SERVICES; do
        wsl.exe -d Ubuntu -- bash -lc \
            "test -f $WSL_TARGET/release/$svc && \
             cp $WSL_TARGET/release/$svc /mnt/c/tmp/yantrik-release/$svc" \
            && ok "Staged $svc" \
            || warn "Service $svc not found (non-fatal)"
    done

    # Copy standalone app binaries to staging
    APP_NAMES="yantrik-notes yantrik-email yantrik-calendar yantrik-weather yantrik-system-monitor yantrik-terminal yantrik-music-player yantrik-text-editor yantrik-image-viewer yantrik-spreadsheet yantrik-document-editor yantrik-presentation yantrik-network-manager yantrik-container-manager yantrik-download-manager yantrik-snippet-manager"
    for app in $APP_NAMES; do
        wsl.exe -d Ubuntu -- bash -lc \
            "test -f $WSL_TARGET/release/$app && \
             cp $WSL_TARGET/release/$app /mnt/c/tmp/yantrik-release/$app" \
            && ok "Staged $app" \
            || warn "App $app not found (non-fatal)"
    done

    # GPU-accelerated variants
    # Format: name:feature_flag:extra_env
    # CUDA = NVIDIA (GeForce, Quadro, Tesla)
    # ROCm = AMD (Radeon RX, Instinct)
    # Vulkan = Universal (Intel Arc, AMD, NVIDIA — portable but slightly slower)
    # Prerequisites: CUDA needs /usr/local/cuda symlinks (see docs/getting-started.md)
    #   sudo mkdir -p /usr/local/cuda
    #   sudo ln -sf /usr/lib/x86_64-linux-gnu /usr/local/cuda/lib64
    #   sudo ln -sf /usr/include /usr/local/cuda/include
    GPU_VARIANTS=(
        "cuda:llamacpp-cuda"
        "rocm:llamacpp-rocm"
        "vulkan:llamacpp-vulkan"
    )
    for variant_spec in "${GPU_VARIANTS[@]}"; do
        IFS=':' read -r variant_name feature_flag <<< "$variant_spec"
        step "Building $variant_name variant..."

        wsl.exe -d Ubuntu -- bash -lc \
            "rm -rf $WSL_TARGET/release/.fingerprint/yantrik-ui-* \
                    $WSL_TARGET/release/.fingerprint/yantrik-ml-* \
                    $WSL_TARGET/release/.fingerprint/llama-cpp-sys-2-* \
                    $WSL_TARGET/release/build/llama-cpp-sys-2-*"

        if wsl.exe -d Ubuntu -- bash -lc \
            "cd /mnt/c/Users/sync/codes/yantrik-os && \
             RUSTFLAGS=\"-A warnings\" CARGO_TARGET_DIR=$WSL_TARGET \
             cargo build --release -p yantrik-ui --features $feature_flag 2>&1"; then
            wsl.exe -d Ubuntu -- bash -lc \
                "cp $WSL_TARGET/release/yantrik-ui /mnt/c/tmp/yantrik-release/yantrik-ui-$variant_name"
            ok "$variant_name build succeeded"
        else
            warn "$variant_name build failed — $variant_name variant will not be published"
        fi
    done

    ok "All binaries staged at $STAGING"

elif [ "$CHANNEL" = "nightly" ] && [ "$SKIP_BUILD" = true ]; then
    step "Skipping build (--skip-build)"
    mkdir -p "$STAGING"

    # Use existing WSL binaries
    wsl.exe -d Ubuntu -- bash -lc \
        "cp $WSL_TARGET/release/yantrik-ui /mnt/c/tmp/yantrik-release/yantrik-ui && \
         cp $WSL_TARGET/release/yantrik    /mnt/c/tmp/yantrik-release/yantrik"
    # Copy CUDA variant if it exists
    wsl.exe -d Ubuntu -- bash -lc \
        "test -f $WSL_TARGET/release/yantrik-ui && \
         cp $WSL_TARGET/release/yantrik-ui /mnt/c/tmp/yantrik-release/yantrik-ui-cuda 2>/dev/null || true"
    # Copy service binaries
    SERVICES="weather-service system-monitor-service notes-service notifications-service calendar-service network-service email-service"
    for svc in $SERVICES; do
        wsl.exe -d Ubuntu -- bash -lc \
            "test -f $WSL_TARGET/release/$svc && \
             cp $WSL_TARGET/release/$svc /mnt/c/tmp/yantrik-release/$svc" 2>/dev/null || true
    done
    # Copy app binaries
    APP_NAMES="yantrik-notes yantrik-email yantrik-calendar yantrik-weather yantrik-system-monitor yantrik-terminal yantrik-music-player yantrik-text-editor yantrik-image-viewer yantrik-spreadsheet yantrik-document-editor yantrik-presentation yantrik-network-manager yantrik-container-manager yantrik-download-manager yantrik-snippet-manager"
    for app in $APP_NAMES; do
        wsl.exe -d Ubuntu -- bash -lc \
            "test -f $WSL_TARGET/release/$app && \
             cp $WSL_TARGET/release/$app /mnt/c/tmp/yantrik-release/$app" 2>/dev/null || true
    done
    ok "Using existing binaries"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 2: Promote (beta/stable — copy from previous channel)
# ═══════════════════════════════════════════════════════════════
GPU_VARIANT_NAMES="cuda rocm vulkan"
SERVICE_NAMES="weather-service system-monitor-service notes-service notifications-service calendar-service network-service email-service"
APP_NAMES="yantrik-notes yantrik-email yantrik-calendar yantrik-weather yantrik-system-monitor yantrik-terminal yantrik-music-player yantrik-text-editor yantrik-image-viewer yantrik-spreadsheet yantrik-document-editor yantrik-presentation yantrik-network-manager yantrik-container-manager yantrik-download-manager yantrik-snippet-manager"

if [ "$CHANNEL" = "beta" ]; then
    step "Promoting nightly → beta..."
    PROMOTE_CMD="cp /var/www/releases/nightly/yantrik-ui /var/www/releases/beta/yantrik-ui && \
         cp /var/www/releases/nightly/yantrik /var/www/releases/beta/yantrik && \
         cp /var/www/releases/nightly/sha256sums.txt /var/www/releases/beta/sha256sums.txt 2>/dev/null || true"
    for gv in $GPU_VARIANT_NAMES; do
        PROMOTE_CMD="$PROMOTE_CMD; cp /var/www/releases/nightly/yantrik-ui-$gv /var/www/releases/beta/yantrik-ui-$gv 2>/dev/null || true"
    done
    for svc in $SERVICE_NAMES; do
        PROMOTE_CMD="$PROMOTE_CMD; cp /var/www/releases/nightly/$svc /var/www/releases/beta/$svc 2>/dev/null || true"
    done
    for app in $APP_NAMES; do
        PROMOTE_CMD="$PROMOTE_CMD; cp /var/www/releases/nightly/$app /var/www/releases/beta/$app 2>/dev/null || true"
    done
    ssh $SSH_OPTS root@$RELEASES_IP "$PROMOTE_CMD" || fail "Promotion failed"
    ok "Nightly promoted to beta"
fi

if [ "$CHANNEL" = "stable" ]; then
    step "Promoting beta → stable..."
    PROMOTE_CMD="cp /var/www/releases/beta/yantrik-ui /var/www/releases/stable/yantrik-ui && \
         cp /var/www/releases/beta/yantrik /var/www/releases/stable/yantrik && \
         cp /var/www/releases/beta/sha256sums.txt /var/www/releases/stable/sha256sums.txt 2>/dev/null || true"
    for gv in $GPU_VARIANT_NAMES; do
        PROMOTE_CMD="$PROMOTE_CMD; cp /var/www/releases/beta/yantrik-ui-$gv /var/www/releases/stable/yantrik-ui-$gv 2>/dev/null || true"
    done
    for svc in $SERVICE_NAMES; do
        PROMOTE_CMD="$PROMOTE_CMD; cp /var/www/releases/beta/$svc /var/www/releases/stable/$svc 2>/dev/null || true"
    done
    for app in $APP_NAMES; do
        PROMOTE_CMD="$PROMOTE_CMD; cp /var/www/releases/beta/$app /var/www/releases/stable/$app 2>/dev/null || true"
    done
    ssh $SSH_OPTS root@$RELEASES_IP "$PROMOTE_CMD" || fail "Promotion failed"
    ok "Beta promoted to stable"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 3: Upload (nightly only — promotions just copy on server)
# ═══════════════════════════════════════════════════════════════
if [ "$CHANNEL" = "nightly" ]; then
    step "Uploading to releases server..."

    # Generate checksums for all binaries in staging
    (cd "$STAGING" && sha256sum yantrik-ui yantrik yantrik-ui-* *-service yantrik-notes yantrik-email yantrik-calendar yantrik-weather yantrik-system-monitor yantrik-terminal yantrik-music-player yantrik-text-editor yantrik-image-viewer yantrik-spreadsheet yantrik-document-editor yantrik-presentation yantrik-network-manager yantrik-container-manager yantrik-download-manager yantrik-snippet-manager 2>/dev/null > sha256sums.txt)
    ok "SHA256 checksums generated"

    # Upload all binaries
    UPLOAD_FILES=("$STAGING/yantrik-ui" "$STAGING/yantrik" "$STAGING/sha256sums.txt")
    for gv in $GPU_VARIANT_NAMES; do
        if [ -f "$STAGING/yantrik-ui-$gv" ]; then
            UPLOAD_FILES+=("$STAGING/yantrik-ui-$gv")
        fi
    done
    # Include service binaries
    SERVICES="weather-service system-monitor-service notes-service notifications-service calendar-service network-service email-service"
    for svc in $SERVICES; do
        if [ -f "$STAGING/$svc" ]; then
            UPLOAD_FILES+=("$STAGING/$svc")
        fi
    done
    # Include standalone app binaries
    for app in $APP_NAMES; do
        if [ -f "$STAGING/$app" ]; then
            UPLOAD_FILES+=("$STAGING/$app")
        fi
    done
    scp $SSH_OPTS \
        "${UPLOAD_FILES[@]}" \
        "root@$RELEASES_IP:/var/www/releases/nightly/" \
        || fail "Upload failed"
    ok "Binaries uploaded to nightly/"
    for gv in $GPU_VARIANT_NAMES; do
        if [ -f "$STAGING/yantrik-ui-$gv" ]; then
            ok "  includes $gv variant (yantrik-ui-$gv)"
        fi
    done
    for svc in $SERVICES; do
        if [ -f "$STAGING/$svc" ]; then
            ok "  includes service: $svc"
        fi
    done
fi

# ═══════════════════════════════════════════════════════════════
# STEP 4: Update manifest.json
# ═══════════════════════════════════════════════════════════════
step "Updating manifest..."

# Read current manifest, update the target channel with per-component versions
ssh $SSH_OPTS root@$RELEASES_IP "python3 -c \"
import json
with open('/var/www/releases/manifest.json') as f:
    m = json.load(f)
ch = m.setdefault('channels', {}).setdefault('$CHANNEL', {})
ch['version'] = '$FULL_VERSION'
ch['date'] = '$BUILD_DATE'
ch['git'] = '$GIT_HASH'
ch['notes'] = 'Build $GIT_HASH ($BUILD_DATE)'
ch['components'] = {
    'yantrik-ml':        {'version': '$COMP_ML_VER',        'git': '$GIT_HASH'},
    'yantrikdb':         {'version': '$COMP_DB_VER',        'git': '$GIT_HASH'},
    'yantrik-companion': {'version': '$COMP_COMPANION_VER', 'git': '$GIT_HASH'},
    'yantrik-os':        {'version': '$COMP_OS_VER',        'git': '$GIT_HASH'},
    'yantrik-ui':        {'version': '$COMP_UI_VER',        'git': '$GIT_HASH'},
}
with open('/var/www/releases/manifest.json', 'w') as f:
    json.dump(m, f, indent=2)
print('Manifest updated')
\"" || fail "Manifest update failed"

ok "Manifest updated for $CHANNEL"

# ═══════════════════════════════════════════════════════════════
# STEP 5: Verify
# ═══════════════════════════════════════════════════════════════
step "Verifying..."

# Check manifest
MANIFEST_VER=$(curl -sf "http://$RELEASES_HOST/manifest.json" | python3 -c "import sys,json; print(json.load(sys.stdin)['channels']['$CHANNEL']['version'])" 2>/dev/null || echo "FAILED")

if [ "$MANIFEST_VER" = "$FULL_VERSION" ]; then
    ok "Manifest verified: $CHANNEL = $FULL_VERSION"
else
    warn "Manifest check returned: $MANIFEST_VER (expected $FULL_VERSION)"
fi

# Check binary is downloadable (nightly)
if [ "$CHANNEL" = "nightly" ]; then
    HTTP_CODE=$(curl -sf -o /dev/null -w "%{http_code}" "http://$RELEASES_HOST/nightly/yantrik-ui" 2>/dev/null || echo "000")
    if [ "$HTTP_CODE" = "200" ]; then
        REMOTE_SIZE=$(curl -sI "http://$RELEASES_HOST/nightly/yantrik-ui" | grep -i content-length | awk '{print $2}' | tr -d '\r')
        ok "Binary downloadable (${REMOTE_SIZE} bytes)"
    else
        warn "Binary download check returned HTTP $HTTP_CODE"
    fi
fi

# Cleanup staging
rm -rf "$STAGING" 2>/dev/null || true

echo
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Release published: $CHANNEL = $FULL_VERSION${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo
echo -e "  ${BOLD}Channel:${NC}  $CHANNEL"
echo -e "  ${BOLD}Version:${NC}  $FULL_VERSION"
echo -e "  ${BOLD}Git:${NC}      $GIT_HASH"
echo -e "  ${BOLD}Date:${NC}     $BUILD_DATE"
echo -e "  ${BOLD}URL:${NC}      http://$RELEASES_HOST/$CHANNEL/"
echo -e "  ${BOLD}Manifest:${NC} http://$RELEASES_HOST/manifest.json"
echo
