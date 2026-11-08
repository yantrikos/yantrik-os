#!/usr/bin/env bash
# One-time Stage install (needs network once). After this, builds run offline.
#   install.sh [DEST]     default DEST=/opt/yantrik/share/stage
# Steps, each explicit:
#   1. copy Stage into DEST
#   2. npm ci --ignore-scripts: exact lockfile versions (all sha512-checked), no
#      package may run code at install time. esbuild ships its binary as a platform
#      package, so no postinstall is needed.
#   3. fetch the pinned chrome-headless-shell (the only step with no lockfile integrity):
#      the zip is checked against browser.zip.sha256, the binary against browser.sha256
#   4. fetch the extra Piper voices, pinned to one commit and sha256-checked
# Ends root-owned and world-readable. Licences: see THIRD_PARTY.md.
set -euo pipefail
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="${1:-/opt/yantrik/share/stage}"
mkdir -p "$DEST"
if [ "$SRC" != "$DEST" ]; then
  rm -rf "$DEST/src" "$DEST/scripts"   # an update must not keep files the new version dropped
  tar -C "$SRC" --exclude=node_modules --exclude=out --exclude=voices/models -cf - . | tar -C "$DEST" -xf -
fi
cd "$DEST"
npm ci --ignore-scripts --no-audit --no-fund

# the render browser: Chrome for Testing's headless shell, pinned by version, zip sha256 and binary sha256
read -r CFT_VERSION ZIP_SHA < browser.zip.sha256
ZIP_URL="https://storage.googleapis.com/chrome-for-testing-public/${CFT_VERSION}/linux64/chrome-headless-shell-linux64.zip"
BROWSER_TMP="$(mktemp -d)"; trap 'rm -rf "$BROWSER_TMP"' EXIT
curl -fsSL --proto '=https' -o "$BROWSER_TMP/browser.zip" "$ZIP_URL"
if [ "$(sha256sum "$BROWSER_TMP/browser.zip" | cut -d' ' -f1)" != "$ZIP_SHA" ]; then
  echo "install: the render browser download does not match the pinned sha256 in browser.zip.sha256. Not installed." >&2
  exit 1
fi
rm -rf browser; mkdir -p browser
if command -v unzip >/dev/null 2>&1; then unzip -q "$BROWSER_TMP/browser.zip" -d browser
else python3 -m zipfile -e "$BROWSER_TMP/browser.zip" browser; fi
BROWSER="$(find browser -type f -name chrome-headless-shell | head -1)"
[ -n "$BROWSER" ] || { echo "install: the render browser was not found in the download." >&2; exit 1; }
chmod +x "$BROWSER"
WANT="$(cut -d' ' -f1 browser.sha256)"
GOT="$(sha256sum "$BROWSER" | cut -d' ' -f1)"
if [ "$WANT" != "$GOT" ]; then
  echo "install: the render browser does not match the pinned sha256 in browser.sha256." >&2
  echo "         pinned $WANT, got $GOT. Removing it." >&2
  rm -rf browser
  exit 1
fi

bash voices/fetch-voices.sh
chmod +x build.sh install.sh sealed-smoke.sh voices/fetch-voices.sh scripts/*.mjs
chmod -R a+rX,go-w "$DEST"
echo "Stage installed at $DEST"
