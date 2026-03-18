#!/usr/bin/env bash
# Test cache restore + rebuild timing
set -eu

TARGET="/home/yantrik/target-yantrik/release"

echo "=== Step 1: Delete core crate artifacts ==="
for crate in yantrik-ml yantrik-companion yantrik-companion-core yantrik-companion-instincts yantrikdb-core yantrikdb-server yantrik-shell-core yantrik-os; do
    rm -rf "$TARGET/.fingerprint/${crate}-"*
done
for crate in yantrik_ml yantrik_companion yantrik_companion_core yantrik_companion_instincts yantrikdb_core yantrikdb_server yantrik_shell_core yantrik_os; do
    rm -f "$TARGET/deps/lib${crate}-"*.rlib "$TARGET/deps/lib${crate}-"*.rmeta "$TARGET/deps/lib${crate}-"*.d
done
echo "Deleted."

echo ""
echo "=== Step 2: Restore from cache ==="
CACHE_DIR="/tmp/yantrik-build-cache"
rm -rf "$CACHE_DIR"
curl -fsSL "http://bin.yantrikos.com/cache/latest.tar.zst" -o /tmp/cache-dl.tar.zst
cd /tmp
zstd -d cache-dl.tar.zst -o cache-dl.tar 2>/dev/null
tar xf cache-dl.tar
rm -f cache-dl.tar cache-dl.tar.zst

# Inject deps
RESTORED=0
for f in "$CACHE_DIR/deps/"*; do
    fname=$(basename "$f")
    cp "$f" "$TARGET/deps/$fname"
    RESTORED=$((RESTORED + 1))
done
echo "  Deps restored: $RESTORED files"

# Inject fingerprints
FP=0
for d in "$CACHE_DIR/fingerprint/"*/; do
    dirname=$(basename "$d")
    cp -r "$d" "$TARGET/.fingerprint/$dirname"
    FP=$((FP + 1))
done
echo "  Fingerprints restored: $FP dirs"

rm -rf "$CACHE_DIR"

echo ""
echo "=== Step 3: Timed rebuild ==="
cd /home/yantrik/src/yantrik-os
echo "Build started at $(date)"
time RUSTFLAGS="-A warnings" CARGO_TARGET_DIR=/home/yantrik/target-yantrik cargo build --release -p yantrik-ui -p yantrik 2>&1 | grep -E "Compiling|Finished"
echo "Build finished at $(date)"
