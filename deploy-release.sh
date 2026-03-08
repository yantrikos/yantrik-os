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

    wsl.exe -d Ubuntu -- bash -lc \
        "cd /mnt/c/Users/sync/codes/yantrik-os && \
         RUSTFLAGS=\"-A warnings\" CARGO_TARGET_DIR=$WSL_TARGET \
         cargo build --release -p yantrik-ui -p yantrik 2>&1" \
        || fail "Build failed"

    # Verify binaries exist
    wsl.exe -d Ubuntu -- bash -lc \
        "test -f $WSL_TARGET/release/yantrik-ui && \
         test -f $WSL_TARGET/release/yantrik" \
        || fail "Binaries not found after build"

    ok "Build succeeded"

    # Copy to staging
    step "Staging binaries..."
    mkdir -p "$STAGING"
    wsl.exe -d Ubuntu -- bash -lc \
        "cp $WSL_TARGET/release/yantrik-ui /mnt/c/tmp/yantrik-release/yantrik-ui && \
         cp $WSL_TARGET/release/yantrik    /mnt/c/tmp/yantrik-release/yantrik"
    ok "Binaries staged at $STAGING"

elif [ "$CHANNEL" = "nightly" ] && [ "$SKIP_BUILD" = true ]; then
    step "Skipping build (--skip-build)"
    mkdir -p "$STAGING"

    # Use existing WSL binaries
    wsl.exe -d Ubuntu -- bash -lc \
        "cp $WSL_TARGET/release/yantrik-ui /mnt/c/tmp/yantrik-release/yantrik-ui && \
         cp $WSL_TARGET/release/yantrik    /mnt/c/tmp/yantrik-release/yantrik"
    ok "Using existing binaries"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 2: Promote (beta/stable — copy from previous channel)
# ═══════════════════════════════════════════════════════════════
if [ "$CHANNEL" = "beta" ]; then
    step "Promoting nightly → beta..."
    ssh $SSH_OPTS root@$RELEASES_IP \
        "cp /var/www/releases/nightly/yantrik-ui /var/www/releases/beta/yantrik-ui && \
         cp /var/www/releases/nightly/yantrik    /var/www/releases/beta/yantrik && \
         cp /var/www/releases/nightly/sha256sums.txt /var/www/releases/beta/sha256sums.txt 2>/dev/null || true" \
        || fail "Promotion failed"
    ok "Nightly promoted to beta"
fi

if [ "$CHANNEL" = "stable" ]; then
    step "Promoting beta → stable..."
    ssh $SSH_OPTS root@$RELEASES_IP \
        "cp /var/www/releases/beta/yantrik-ui /var/www/releases/stable/yantrik-ui && \
         cp /var/www/releases/beta/yantrik    /var/www/releases/stable/yantrik && \
         cp /var/www/releases/beta/sha256sums.txt /var/www/releases/stable/sha256sums.txt 2>/dev/null || true" \
        || fail "Promotion failed"
    ok "Beta promoted to stable"
fi

# ═══════════════════════════════════════════════════════════════
# STEP 3: Upload (nightly only — promotions just copy on server)
# ═══════════════════════════════════════════════════════════════
if [ "$CHANNEL" = "nightly" ]; then
    step "Uploading to releases server..."

    # Generate checksums
    (cd "$STAGING" && sha256sum yantrik-ui yantrik > sha256sums.txt)
    ok "SHA256 checksums generated"

    # Upload
    scp $SSH_OPTS \
        "$STAGING/yantrik-ui" \
        "$STAGING/yantrik" \
        "$STAGING/sha256sums.txt" \
        "root@$RELEASES_IP:/var/www/releases/nightly/" \
        || fail "Upload failed"
    ok "Binaries uploaded to nightly/"
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
