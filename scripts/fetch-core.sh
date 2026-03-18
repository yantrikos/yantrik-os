#!/bin/bash
# Fetch pre-built component binaries from bin.yantrikos.com
# Usage: ./scripts/fetch-core.sh [channel] [--install]
#
# Downloads pre-built binaries and optionally installs them to /opt/yantrik/bin

set -euo pipefail

CHANNEL="${1:-nightly}"
INSTALL=false
if [ "${2:-}" = "--install" ]; then INSTALL=true; fi

REGISTRY="http://bin.yantrikos.com"
CACHE_DIR="${YANTRIK_CACHE:-$HOME/.cache/yantrik/binaries}"
INSTALL_DIR="/opt/yantrik/bin"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

step() { echo -e "${GREEN}==> $1${NC}"; }
warn() { echo -e "${YELLOW}    $1${NC}"; }
fail() { echo -e "${RED}!!! $1${NC}"; exit 1; }

# Fetch channel manifest
step "Fetching $CHANNEL channel manifest..."
MANIFEST=$(curl -fsSL "$REGISTRY/channels/$CHANNEL.json") || fail "Failed to fetch manifest"

MANIFEST_DATE=$(echo "$MANIFEST" | python3 -c "import sys,json; print(json.load(sys.stdin).get('date','unknown'))")
MANIFEST_SHA=$(echo "$MANIFEST" | python3 -c "import sys,json; print(json.load(sys.stdin).get('git_sha','unknown'))")
step "Channel: $CHANNEL | Date: $MANIFEST_DATE | Git: $MANIFEST_SHA"

# Parse components
COMPONENTS=$(echo "$MANIFEST" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for name, info in data.get('components', {}).items():
    print(f\"{name}|{info['version']}|{info['url']}|{info['sha256']}|{info['size']}\")
")

mkdir -p "$CACHE_DIR"

DOWNLOADED=0
CACHED=0
FAILED=0

while IFS='|' read -r name version url sha256 size; do
    DEST="$CACHE_DIR/$name/$version"
    TARBALL="$DEST/$(basename "$url")"

    # Check cache
    if [ -f "$TARBALL.sha256" ] && [ "$(cat "$TARBALL.sha256")" = "$sha256" ]; then
        CACHED=$((CACHED + 1))
        continue
    fi

    # Download
    mkdir -p "$DEST"
    SIZE_MB=$(echo "scale=1; $size / 1048576" | bc 2>/dev/null || echo "?")
    echo "  Downloading $name v$version (${SIZE_MB}MB)..."
    if curl -fsSL "$url" -o "$TARBALL" 2>/dev/null; then
        # Verify SHA256
        ACTUAL_SHA=$(sha256sum "$TARBALL" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$TARBALL" | awk '{print $1}')
        if [ "$ACTUAL_SHA" = "$sha256" ]; then
            echo "$sha256" > "$TARBALL.sha256"
            DOWNLOADED=$((DOWNLOADED + 1))
        else
            warn "SHA256 mismatch for $name! Expected $sha256, got $ACTUAL_SHA"
            rm -f "$TARBALL"
            FAILED=$((FAILED + 1))
        fi
    else
        warn "Failed to download $name"
        FAILED=$((FAILED + 1))
    fi
done <<< "$COMPONENTS"

step "Downloaded: $DOWNLOADED | Cached: $CACHED | Failed: $FAILED"

# Install if requested
if [ "$INSTALL" = true ]; then
    step "Installing to $INSTALL_DIR..."
    sudo mkdir -p "$INSTALL_DIR"

    while IFS='|' read -r name version url sha256 size; do
        DEST="$CACHE_DIR/$name/$version"
        TARBALL="$DEST/$(basename "$url")"
        if [ -f "$TARBALL" ]; then
            # Extract tarball
            TMP="/tmp/yantrik-install-$$"
            mkdir -p "$TMP"
            zstd -d "$TARBALL" -o "$TMP/$name.tar" 2>/dev/null && \
                tar -xf "$TMP/$name.tar" -C "$TMP" 2>/dev/null

            # Find and install binary
            BINARY=$(find "$TMP" -type f -executable -name "$name" 2>/dev/null | head -1)
            if [ -n "$BINARY" ]; then
                sudo cp "$BINARY" "$INSTALL_DIR/$name"
                sudo chmod +x "$INSTALL_DIR/$name"
                echo "  Installed $name v$version"
            fi
            rm -rf "$TMP"
        fi
    done <<< "$COMPONENTS"

    step "Installation complete."
fi

# Save manifest to cache for reference
echo "$MANIFEST" > "$CACHE_DIR/current-$CHANNEL.json"
step "Done. Manifest saved to $CACHE_DIR/current-$CHANNEL.json"
