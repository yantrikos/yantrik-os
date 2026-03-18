#!/usr/bin/env bash
# Restore cargo build cache from bin.yantrikos.com
# Downloads pre-compiled core crate artifacts and injects them into the target directory.
# After restoration, cargo will skip recompiling these crates.
#
# Usage: bash scripts/restore-cache.sh
set -eu

TARGET="/home/yantrik/target-yantrik/release"
CACHE_DIR="/tmp/yantrik-build-cache"
REGISTRY="http://bin.yantrikos.com"
LOCAL_CACHE="$HOME/.cache/yantrik/build-cache"

mkdir -p "$TARGET/deps" "$TARGET/.fingerprint" "$TARGET/build" "$LOCAL_CACHE"

# Check rustc version compatibility
CURRENT_RUSTC=$(rustc --version | awk '{print $2}')
echo "Current rustc: $CURRENT_RUSTC"

# Download cache if not already present or outdated
CACHE_FILE="$LOCAL_CACHE/latest.tar.zst"
echo "Fetching build cache from registry..."
curl -fsSL "$REGISTRY/cache/latest.tar.zst" -o "$CACHE_FILE.tmp" 2>/dev/null
if [ $? -eq 0 ] && [ -s "$CACHE_FILE.tmp" ]; then
    mv "$CACHE_FILE.tmp" "$CACHE_FILE"
    echo "Downloaded build cache ($(du -sh "$CACHE_FILE" | cut -f1))"
else
    rm -f "$CACHE_FILE.tmp"
    if [ -f "$CACHE_FILE" ]; then
        echo "Using cached version"
    else
        echo "ERROR: No build cache available"
        exit 1
    fi
fi

# Extract
rm -rf "$CACHE_DIR"
cd /tmp
zstd -d "$CACHE_FILE" -o /tmp/cache.tar 2>/dev/null
tar xf /tmp/cache.tar
rm -f /tmp/cache.tar

# Verify rustc version
CACHE_RUSTC=$(cat "$CACHE_DIR/rustc-version.txt" 2>/dev/null | awk '{print $2}')
if [ "$CURRENT_RUSTC" != "$CACHE_RUSTC" ]; then
    echo "WARNING: Cache built with rustc $CACHE_RUSTC, current is $CURRENT_RUSTC"
    echo "         Artifacts may not be compatible. Proceeding anyway..."
fi

# Inject deps
echo "Restoring compiled artifacts..."
RESTORED=0

if [ -d "$CACHE_DIR/deps" ]; then
    for f in "$CACHE_DIR/deps/"*; do
        fname=$(basename "$f")
        dest="$TARGET/deps/$fname"
        if [ ! -f "$dest" ] || [ "$f" -nt "$dest" ]; then
            cp "$f" "$dest"
            RESTORED=$((RESTORED + 1))
        fi
    done
    echo "  Deps: $RESTORED files restored"
fi

# Inject fingerprints
FP_RESTORED=0
if [ -d "$CACHE_DIR/fingerprint" ]; then
    for d in "$CACHE_DIR/fingerprint/"*/; do
        dirname=$(basename "$d")
        dest="$TARGET/.fingerprint/$dirname"
        if [ ! -d "$dest" ]; then
            cp -r "$d" "$dest"
            FP_RESTORED=$((FP_RESTORED + 1))
        fi
    done
    echo "  Fingerprints: $FP_RESTORED dirs restored"
fi

# Inject build outputs
BUILD_RESTORED=0
if [ -d "$CACHE_DIR/build" ]; then
    for d in "$CACHE_DIR/build/"*/; do
        dirname=$(basename "$d")
        dest="$TARGET/build/$dirname"
        if [ ! -d "$dest" ]; then
            cp -r "$d" "$dest"
            BUILD_RESTORED=$((BUILD_RESTORED + 1))
        fi
    done
    echo "  Build outputs: $BUILD_RESTORED dirs restored"
fi

# Cleanup
rm -rf "$CACHE_DIR"

echo ""
echo "Build cache restored. Core crates should not be recompiled."
echo "Run: cargo build --release -p yantrik-ui -p yantrik"
