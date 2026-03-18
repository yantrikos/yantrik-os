#!/usr/bin/env bash
# Snapshot cargo build cache for core heavy crates and upload to bin.yantrikos.com
# This captures the compiled .rlib/.rmeta/.d files + fingerprints so that
# subsequent builds can skip recompiling these crates.
#
# Usage: bash scripts/snapshot-cache.sh
# Restore: bash scripts/restore-cache.sh
set -eu

TARGET="/home/yantrik/target-yantrik/release"
CACHE_DIR="/tmp/yantrik-build-cache"
PROXMOX="192.168.4.152"
CT=124
KEY="/home/yantrik/.ssh/id_deploy"
REGISTRY_ROOT="/var/www/bin"

# Core crates that are expensive to compile (the dependency chain)
CORE_CRATES="yantrik_ml yantrik_companion yantrik_companion_core yantrik_companion_instincts yantrikdb_core yantrikdb_server yantrik_shell_core yantrik_os"

# Also cache heavy third-party deps
HEAVY_DEPS="candle_core candle_nn candle_transformers tokenizers slint"

rm -rf "$CACHE_DIR"
mkdir -p "$CACHE_DIR/deps" "$CACHE_DIR/fingerprint" "$CACHE_DIR/build"

echo "=== Snapshotting core crate artifacts ==="

# Collect deps (rlib, rmeta, d files)
for crate in $CORE_CRATES $HEAVY_DEPS; do
    # Find latest rlib
    latest=$(ls -t "$TARGET/deps/lib${crate}-"*.rlib 2>/dev/null | head -1)
    if [ -z "$latest" ]; then
        echo "  SKIP: $crate (no rlib found)"
        continue
    fi
    hash=$(basename "$latest" | sed "s/lib${crate}-//" | sed "s/\.rlib//")

    # Copy rlib, rmeta, d files with this hash
    for ext in rlib rmeta d; do
        src="$TARGET/deps/lib${crate}-${hash}.${ext}"
        if [ -f "$src" ]; then
            cp "$src" "$CACHE_DIR/deps/"
        fi
    done

    size=$(du -sh "$latest" | cut -f1)
    echo "  OK: $crate ($hash) $size"
done

# Collect fingerprints for core crates
echo ""
echo "=== Snapshotting fingerprints ==="
for crate in $CORE_CRATES; do
    crate_dash=$(echo "$crate" | tr '_' '-')
    # Find latest fingerprint dir
    latest_fp=$(ls -td "$TARGET/.fingerprint/${crate_dash}-"* 2>/dev/null | head -1)
    if [ -n "$latest_fp" ]; then
        cp -r "$latest_fp" "$CACHE_DIR/fingerprint/"
        echo "  OK: $crate_dash fingerprint"
    fi
done

# Also snapshot build script outputs for crates that have them
echo ""
echo "=== Snapshotting build script outputs ==="
for crate in $CORE_CRATES; do
    crate_dash=$(echo "$crate" | tr '_' '-')
    latest_build=$(ls -td "$TARGET/build/${crate_dash}-"* 2>/dev/null | head -1)
    if [ -n "$latest_build" ]; then
        cp -r "$latest_build" "$CACHE_DIR/build/"
        echo "  OK: $crate_dash build output"
    fi
done

# Get rustc version for compatibility check
rustc --version > "$CACHE_DIR/rustc-version.txt"
echo ""
cat "$CACHE_DIR/rustc-version.txt"

# Package
echo ""
echo "=== Packaging cache ==="
cd /tmp
RUSTC_VER=$(rustc --version | awk '{print $2}')
ARCHIVE="yantrik-build-cache-${RUSTC_VER}.tar.zst"
tar cf - yantrik-build-cache/ | zstd -3 -o "/tmp/$ARCHIVE"
ARCHIVE_SIZE=$(du -sh "/tmp/$ARCHIVE" | cut -f1)
echo "Archive: $ARCHIVE ($ARCHIVE_SIZE)"

# Upload to registry
echo ""
echo "=== Uploading to bin.yantrikos.com ==="
ssh -i "$KEY" -o StrictHostKeyChecking=no "root@$PROXMOX" \
    "pct exec $CT -- mkdir -p $REGISTRY_ROOT/cache/" 2>/dev/null

cat "/tmp/$ARCHIVE" | ssh -i "$KEY" -o StrictHostKeyChecking=no "root@$PROXMOX" \
    "pct exec $CT -- tee $REGISTRY_ROOT/cache/$ARCHIVE > /dev/null" 2>/dev/null

# Also upload as "latest"
cat "/tmp/$ARCHIVE" | ssh -i "$KEY" -o StrictHostKeyChecking=no "root@$PROXMOX" \
    "pct exec $CT -- tee $REGISTRY_ROOT/cache/latest.tar.zst > /dev/null" 2>/dev/null

echo "Uploaded to bin.yantrikos.com/cache/$ARCHIVE"
echo "Also available as bin.yantrikos.com/cache/latest.tar.zst"

# Cleanup
rm -rf "$CACHE_DIR"
echo ""
echo "Done!"
