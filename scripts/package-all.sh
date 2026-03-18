#!/usr/bin/env bash
# Package all built binaries as component tarballs
# Run inside WSL: bash /home/yantrik/src/yantrik-os/scripts/package-all.sh
set -eu

TARGET_DIR=/home/yantrik/target-yantrik/release
STAGING=/tmp/ypub-all
OUTPUT_DIR=/tmp/yantrik-components
rm -rf "$STAGING" "$OUTPUT_DIR"
mkdir -p "$STAGING" "$OUTPUT_DIR"

BINARIES="yantrik-ui yantrik weather-service system-monitor-service notes-service notifications-service calendar-service network-service email-service yantrik-notes yantrik-email yantrik-calendar yantrik-weather yantrik-system-monitor yantrik-terminal yantrik-music-player yantrik-text-editor yantrik-image-viewer yantrik-spreadsheet yantrik-document-editor yantrik-presentation yantrik-network-manager yantrik-container-manager yantrik-download-manager yantrik-snippet-manager"
VERSION="0.3.0"
TARGET="x86_64-unknown-linux-gnu"
GIT_SHA=$(cd /home/yantrik/src/yantrik-os && git rev-parse --short HEAD 2>/dev/null || echo "unknown")

COMPONENTS_JSON="{"
FIRST=true

for name in $BINARIES; do
    BIN="$TARGET_DIR/$name"
    if [ ! -f "$BIN" ]; then
        echo "SKIP: $name"
        continue
    fi

    # Stage the binary
    mkdir -p "$STAGING/$name"
    cp "$BIN" "$STAGING/$name/"

    # Compute checksums
    SHA=$(sha256sum "$BIN" | cut -d' ' -f1)
    SIZE=$(stat -c%s "$BIN")

    # Write metadata
    cat > "$STAGING/$name/metadata.json" << EOF
{"name":"$name","version":"$VERSION","target":"$TARGET","git_sha":"$GIT_SHA","sha256":"$SHA","size":$SIZE,"built":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
EOF

    # Create tarball
    cd "$STAGING"
    tar cf - "$name/" | zstd -3 -o "$OUTPUT_DIR/$name.tar.zst" 2>/dev/null
    cd /

    SIZE_MB=$(echo "scale=1; $SIZE/1048576" | bc 2>/dev/null || echo "?")
    echo "OK: $name (${SIZE_MB}MB)"

    # Build manifest entry
    URL="http://bin.yantrikos.com/components/$name/$VERSION/$TARGET.tar.zst"
    if [ "$FIRST" = true ]; then
        FIRST=false
    else
        COMPONENTS_JSON="$COMPONENTS_JSON,"
    fi
    COMPONENTS_JSON="$COMPONENTS_JSON\"$name\":{\"version\":\"$VERSION\",\"target\":\"$TARGET\",\"url\":\"$URL\",\"sha256\":\"$SHA\",\"size\":$SIZE}"
done

COMPONENTS_JSON="$COMPONENTS_JSON}"

# Write channel manifest
cat > "$OUTPUT_DIR/nightly.json" << EOF
{"channel":"nightly","date":"$(date -u +%Y-%m-%d)","git_sha":"$GIT_SHA","target":"$TARGET","components":$COMPONENTS_JSON}
EOF

echo "---"
echo "Packaged $(ls "$OUTPUT_DIR"/*.tar.zst | wc -l) components"
du -sh "$OUTPUT_DIR"
echo "Manifest: $OUTPUT_DIR/nightly.json"

# Cleanup staging
rm -rf "$STAGING"
