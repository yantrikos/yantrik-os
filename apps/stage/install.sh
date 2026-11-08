#!/usr/bin/env bash
# One-time Stage install (needs network once). After this, builds run offline.
#   install.sh [DEST]     default DEST=/opt/yantrik/share/stage
# Steps, each explicit:
#   1. copy Stage into DEST
#   2. npm ci --ignore-scripts: exact lockfile versions (all sha512-checked), no
#      package may run code at install time. esbuild and the Remotion compositor
#      ship their binaries as platform packages, so no postinstall is needed.
#   3. fetch Remotion's headless browser (the only step with no lockfile
#      integrity), then verify the binary against the sha256 pinned in browser.sha256
#   4. fetch the extra Piper voices, pinned to one commit and sha256-checked
# Ends root-owned and world-readable. Licences: see THIRD_PARTY.md.
set -euo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${1:-/opt/yantrik/share/stage}"
mkdir -p "$DEST"
if [ "$SRC" != "$DEST" ]; then
  tar -C "$SRC" --exclude=node_modules --exclude=out --exclude=voices/models -cf - . | tar -C "$DEST" -xf -
fi
cd "$DEST"
npm ci --ignore-scripts --no-audit --no-fund

npx remotion browser ensure
BROWSER="$(find node_modules/.remotion -type f \( -name chrome-headless-shell -o -name headless_shell \) | head -1)"
[ -n "$BROWSER" ] || { echo "install: the render browser was not found after 'remotion browser ensure'." >&2; exit 1; }
WANT="$(cut -d' ' -f1 browser.sha256)"
GOT="$(sha256sum "$BROWSER" | cut -d' ' -f1)"
if [ "$WANT" != "$GOT" ]; then
  echo "install: the render browser does not match the pinned sha256 in browser.sha256." >&2
  echo "         pinned $WANT, got $GOT. Removing it; check which Remotion version fetched it." >&2
  rm -rf node_modules/.remotion
  exit 1
fi

bash voices/fetch-voices.sh
chmod +x build.sh install.sh sealed-smoke.sh voices/fetch-voices.sh scripts/*.mjs
chmod -R a+rX,go-w "$DEST"
echo "Stage installed at $DEST"
