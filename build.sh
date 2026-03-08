#!/bin/bash
# ═══════════════════════════════════════════════════════════════
# Yantrik OS — Component Build System
# ═══════════════════════════════════════════════════════════════
#
# Builds Yantrik components individually or all together.
# Each component can be built, tagged, and released independently.
#
# Usage:
#   ./build.sh                    Build everything (default)
#   ./build.sh --component ml     Build only yantrik-ml
#   ./build.sh --component db     Build only yantrikdb-core
#   ./build.sh --component comp   Build only yantrik-companion
#   ./build.sh --component os     Build only yantrik-os
#   ./build.sh --component ui     Build only yantrik-ui
#   ./build.sh --tag v0.2.0       Build + tag all components
#   ./build.sh --check            Just check versions, don't build
#   ./build.sh --bump patch       Bump version (patch/minor/major)
#
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

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

WSL_TARGET="/home/yantrik/target-yantrik"

# Component registry: name, crate_path, cargo_package
declare -A COMPONENTS=(
    [ml]="crates/yantrik-ml"
    [db]="crates/yantrikdb-core"
    [comp]="crates/yantrik-companion"
    [os]="crates/yantrik-os"
    [ui]="crates/yantrik-ui"
    [cli]="crates/yantrik"
)

declare -A COMPONENT_NAMES=(
    [ml]="yantrik-ml"
    [db]="yantrikdb-core"
    [comp]="yantrik-companion"
    [os]="yantrik-os"
    [ui]="yantrik-ui"
    [cli]="yantrik"
)

# ── Parse args ──
COMPONENT=""
TAG=""
CHECK_ONLY=false
BUMP=""

while [ $# -gt 0 ]; do
    case "$1" in
        --component|-c) COMPONENT="$2"; shift 2 ;;
        --tag|-t)       TAG="$2"; shift 2 ;;
        --check)        CHECK_ONLY=true; shift ;;
        --bump|-b)      BUMP="$2"; shift 2 ;;
        *)              fail "Unknown argument: $1" ;;
    esac
done

echo
echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
echo -e "${BOLD}  Yantrik OS — Component Build System${NC}"
echo -e "${CYAN}═══════════════════════════════════════════════${NC}"
echo

# ═══════════════════════════════════════════════════════════════
# Show all component versions
# ═══════════════════════════════════════════════════════════════
read_cargo_version() {
    grep '^version' "$1" 2>/dev/null | head -1 | sed 's/.*"\(.*\)"/\1/' || echo "0.0.0"
}

step "Component Versions"
for key in ml db comp os ui cli; do
    path="${COMPONENTS[$key]}/Cargo.toml"
    name="${COMPONENT_NAMES[$key]}"
    ver=$(read_cargo_version "$path")
    echo -e "   ${DIM}${name}${NC}  ${BOLD}v${ver}${NC}"
done
echo

if [ "$CHECK_ONLY" = true ]; then
    exit 0
fi

# ═══════════════════════════════════════════════════════════════
# Bump version (if requested)
# ═══════════════════════════════════════════════════════════════
bump_version() {
    local toml="$1"
    local bump_type="$2"
    local current
    current=$(read_cargo_version "$toml")

    IFS='.' read -r major minor patch <<< "$current"
    case "$bump_type" in
        patch) patch=$((patch + 1)) ;;
        minor) minor=$((minor + 1)); patch=0 ;;
        major) major=$((major + 1)); minor=0; patch=0 ;;
        *) fail "Unknown bump type: $bump_type (use patch, minor, major)" ;;
    esac

    local new_ver="${major}.${minor}.${patch}"
    sed -i "0,/^version = \"${current}\"/s//version = \"${new_ver}\"/" "$toml"
    echo "$new_ver"
}

if [ -n "$BUMP" ]; then
    step "Bumping versions ($BUMP)"

    if [ -n "$COMPONENT" ]; then
        # Bump single component
        path="${COMPONENTS[$COMPONENT]}/Cargo.toml"
        name="${COMPONENT_NAMES[$COMPONENT]}"
        new_ver=$(bump_version "$path" "$BUMP")
        ok "$name → v$new_ver"
    else
        # Bump all components
        for key in ml db comp os ui cli; do
            path="${COMPONENTS[$key]}/Cargo.toml"
            name="${COMPONENT_NAMES[$key]}"
            new_ver=$(bump_version "$path" "$BUMP")
            ok "$name → v$new_ver"
        done
    fi
    echo
fi

# ═══════════════════════════════════════════════════════════════
# Clear fingerprints (WSL timestamp sync workaround)
# ═══════════════════════════════════════════════════════════════
step "Clearing fingerprints..."

if [ -n "$COMPONENT" ]; then
    name="${COMPONENT_NAMES[$COMPONENT]}"
    wsl.exe -d Ubuntu -- bash -lc \
        "rm -rf $WSL_TARGET/release/.fingerprint/${name}-*"
    ok "Cleared fingerprints for $name"
else
    wsl.exe -d Ubuntu -- bash -lc \
        "rm -rf $WSL_TARGET/release/.fingerprint/yantrik-ml-* \
                $WSL_TARGET/release/.fingerprint/yantrikdb-core-* \
                $WSL_TARGET/release/.fingerprint/yantrik-companion-* \
                $WSL_TARGET/release/.fingerprint/yantrik-os-* \
                $WSL_TARGET/release/.fingerprint/yantrik-ui-* \
                $WSL_TARGET/release/.fingerprint/yantrik-[0-9a-f]*"
    ok "Cleared all fingerprints"
fi

# ═══════════════════════════════════════════════════════════════
# Build
# ═══════════════════════════════════════════════════════════════
if [ -n "$COMPONENT" ]; then
    name="${COMPONENT_NAMES[$COMPONENT]}"
    step "Building $name..."
    wsl.exe -d Ubuntu -- bash -lc \
        "cd /mnt/c/Users/sync/codes/yantrik-os && \
         RUSTFLAGS=\"-A warnings\" CARGO_TARGET_DIR=$WSL_TARGET \
         cargo build --release -p $name 2>&1" \
        || fail "Build failed for $name"
    ok "$name built successfully"
else
    # Build the two output binaries (which pull in all components)
    step "Building all components..."
    wsl.exe -d Ubuntu -- bash -lc \
        "cd /mnt/c/Users/sync/codes/yantrik-os && \
         RUSTFLAGS=\"-A warnings\" CARGO_TARGET_DIR=$WSL_TARGET \
         cargo build --release -p yantrik-ui -p yantrik 2>&1" \
        || fail "Build failed"
    ok "All components built successfully"
fi

# ═══════════════════════════════════════════════════════════════
# Tag (if requested)
# ═══════════════════════════════════════════════════════════════
if [ -n "$TAG" ]; then
    step "Tagging $TAG..."

    if [ -n "$COMPONENT" ]; then
        name="${COMPONENT_NAMES[$COMPONENT]}"
        ver=$(read_cargo_version "${COMPONENTS[$COMPONENT]}/Cargo.toml")
        git tag -a "$TAG" -m "$name $TAG" 2>/dev/null || warn "Tag $TAG already exists"
        ok "Tagged $name as $TAG (v$ver)"
    else
        git tag -a "$TAG" -m "Yantrik OS $TAG" 2>/dev/null || warn "Tag $TAG already exists"
        ok "Tagged workspace as $TAG"
    fi
fi

# ═══════════════════════════════════════════════════════════════
# Summary
# ═══════════════════════════════════════════════════════════════
echo
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Build complete${NC}"
echo -e "${GREEN}═══════════════════════════════════════════════${NC}"
echo

step "Component Versions (after build)"
for key in ml db comp os ui cli; do
    path="${COMPONENTS[$key]}/Cargo.toml"
    name="${COMPONENT_NAMES[$key]}"
    ver=$(read_cargo_version "$path")
    echo -e "   ${DIM}${name}${NC}  ${BOLD}v${ver}${NC}"
done
echo

echo -e "${DIM}Next steps:${NC}"
echo -e "  ${BOLD}Deploy to VM:${NC}     ./deploy.sh --skip-build"
echo -e "  ${BOLD}Publish release:${NC}  ./deploy-release.sh nightly --skip-build"
echo -e "  ${BOLD}Bump & rebuild:${NC}   ./build.sh --bump patch"
echo
