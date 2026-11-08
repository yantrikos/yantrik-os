#!/usr/bin/env bash
# The one command a Mind runs for an episode:   bash build.sh episode.json
# lint -> voices -> render. Writes out/<title>.mp4 and out/contact.png next to
# where you run it (or STAGE_OUT). Safe to re-run: unchanged lines keep their
# voices and an unchanged episode is not filmed twice. Needs no network and no
# npm install: node_modules were installed once with Stage.
set -euo pipefail
STAGE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EP="${1:-}"
if [ -z "$EP" ] || [ ! -f "$EP" ]; then
  echo "build.sh: give me the episode file, like: bash build.sh episode.json" >&2
  exit 2
fi
OUT="${STAGE_OUT:-$PWD/out}"
WORK="$OUT/work"
mkdir -p "$WORK"
: > "$OUT/progress"   # a mind reads the last line of this to see how far along the film is
if [ ! -d "$STAGE_DIR/node_modules/remotion" ]; then
  echo "build.sh: Stage is not fully installed (no node_modules). Run $STAGE_DIR/install.sh once." >&2
  exit 3
fi
T0=$(date +%s)

echo "== 1/3 lint =="
node "$STAGE_DIR/scripts/stage-lint.mjs" "$EP" --cache "$OUT/cache" --write "$WORK/episode.json" || {
  echo "build.sh: fix the problems above in $EP and run build.sh again." >&2
  exit 1
}

echo "== 2/3 voices =="
echo "stage: lint ok" >> "$OUT/progress"
node "$STAGE_DIR/scripts/voices.mjs" "$WORK/episode.json" "$WORK"
echo "stage: voices ok" >> "$OUT/progress"

echo "== 3/3 render =="
node "$STAGE_DIR/scripts/render.mjs" "$WORK" "$OUT"

MP4="$(ls -t "$OUT"/*.mp4 2>/dev/null | head -1)"
echo "== done in $(( $(date +%s) - T0 ))s =="
echo "video:   $MP4"
echo "contact: $OUT/contact.png"
