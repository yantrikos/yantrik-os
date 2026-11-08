#!/usr/bin/env bash
# The one command a Mind runs for an episode:   bash build.sh episode.json
# lint -> voices -> render. Writes out/<title>.mp4 and out/contact.png next to
# where you run it (or STAGE_OUT). Safe to re-run: unchanged lines keep their
# voices and an unchanged episode is not filmed twice. Needs no network and no
# npm install: node_modules were installed once with Stage.
#
# Sealed workshop (#1124) assumptions, checked by sealed-smoke.sh:
#   - no network, and HOME may be unset;
#   - every write lands in STAGE_OUT (default $PWD/out, must be writable); STAGE_CACHE_DIR and
#     STAGE_NO_CACHE are no longer used (there is no bundle cache);
#   - TMPDIR is pointed inside STAGE_OUT/work, so /tmp is never filled;
#   - read-only binds needed: the Stage install (this folder), /opt/yantrik/models/tts
#     (piper, espeak-ng-data, the OS voice; or STAGE_PIPER_DIR) and ffmpeg on PATH;
#   - the render browser is the one fetched at install time (or STAGE_BROWSER); Chrome
#     needs /dev/shm or --disable-dev-shm-usage handled by the workshop's bwrap setup.
set -euo pipefail
STAGE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EP="${1:-}"
if [ -z "$EP" ] || [ ! -f "$EP" ]; then
  echo "build.sh: give me the episode file, like: bash build.sh episode.json" >&2
  exit 2
fi
OUT="${STAGE_OUT:-$PWD/out}"
WORK="$OUT/work"
mkdir -p "$WORK/tmp"
export TMPDIR="$WORK/tmp"
: > "$OUT/progress"   # a mind reads the last line of this to see how far along the film is
STEP="start"
# any failure leaves a last line saying which step failed, so a poller never sees "in progress" forever
trap 'echo "stage: failed at $STEP" >> "$OUT/progress"' ERR
step() { STEP="$1"; echo "== $2 =="; }
if [ ! -d "$STAGE_DIR/node_modules/puppeteer-core" ]; then
  echo "build.sh: Stage is not fully installed (no node_modules). Run $STAGE_DIR/install.sh once." >&2
  echo "stage: failed at install check" >> "$OUT/progress"
  exit 3
fi
T0=$(date +%s)
# the voice cache only grows by new lines; forget voices unused for 30 days
find "$OUT/cache" -type f -mtime +30 -delete 2>/dev/null || true

step lint "1/3 lint"
node "$STAGE_DIR/scripts/stage-lint.mjs" "$EP" --cache "$OUT/cache" --write "$WORK/episode.json" || {
  echo "build.sh: fix the problems above in $EP and run build.sh again." >&2
  echo "stage: failed at lint (see the problem list)" >> "$OUT/progress"
  trap - ERR
  exit 1
}
echo "stage: lint ok" >> "$OUT/progress"

step voices "2/3 voices"
node "$STAGE_DIR/scripts/voices.mjs" "$WORK/episode.json" "$WORK"
echo "stage: voices ok" >> "$OUT/progress"

step render "3/3 render"
node "$STAGE_DIR/scripts/render.mjs" "$WORK" "$OUT"
rm -rf "$WORK/tmp"

MP4="$(cat "$OUT/video")"
echo "== done in $(( $(date +%s) - T0 ))s =="
echo "video:   $MP4"
echo "contact: $OUT/contact.png"
