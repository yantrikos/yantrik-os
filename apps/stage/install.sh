#!/usr/bin/env bash
# One-time Stage install (needs network once). After this, builds run offline.
#   install.sh [DEST]     default DEST=/opt/yantrik/share/stage
# Copies Stage into DEST, installs the pinned node_modules from the lockfile,
# lets Remotion fetch its headless browser into node_modules/.remotion, and
# downloads the extra Piper voices. Ends root-owned and world-readable.
set -euo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${1:-/opt/yantrik/share/stage}"
mkdir -p "$DEST"
if [ "$SRC" != "$DEST" ]; then
  tar -C "$SRC" --exclude=node_modules --exclude=out --exclude=voices/models -cf - . | tar -C "$DEST" -xf -
fi
cd "$DEST"
npm ci --no-audit --no-fund
npx remotion browser ensure
bash voices/fetch-voices.sh
chmod +x build.sh install.sh voices/fetch-voices.sh scripts/*.mjs
chmod -R a+rX,go-w "$DEST"
echo "Stage installed at $DEST"
