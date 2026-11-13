#!/usr/bin/env bash
# Download the Stage Piper voices (all permissively licensed, see voices.json) into voices/models. Run once at install
# time; render time needs no network. The source is pinned to one commit of
# rhasspy/piper-voices (voices.json "source.revision") and every file must match
# the sha256 stored in voices.json, or it is deleted and the install stops.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
dest="${1:-$here/models}"
os_voices="${STAGE_PIPER_DIR:-/opt/yantrik/models/tts}"   # a voice the OS already ships (same sha256) is used from there, not fetched again
mkdir -p "$dest"
node -e '
const j = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
const base = `https://huggingface.co/${j.source.repo}/resolve/${j.source.revision}`;
for (const v of Object.values(j.models)) if (v.url) for (const e of ["onnx", "onnx.json"]) console.log(`${base}/${v.url}.${e} ${v.url.split("/").pop()}.${e} ${v.sha256[e]}`);
' "$here/voices.json" | while read -r url name sum; do
  if [ -s "$os_voices/$name" ] && [ "$(sha256sum "$os_voices/$name" | cut -d' ' -f1)" = "$sum" ]; then continue; fi
  if [ -s "$dest/$name" ] && [ "$(sha256sum "$dest/$name" | cut -d' ' -f1)" = "$sum" ]; then continue; fi
  curl -fsSL --proto '=https' -o "$dest/$name.part" "$url"
  got="$(sha256sum "$dest/$name.part" | cut -d' ' -f1)"
  if [ "$got" != "$sum" ]; then
    rm -f "$dest/$name.part"
    echo "fetch-voices: $name does not match its pinned sha256 (got $got). Not installed." >&2
    exit 1
  fi
  mv -f "$dest/$name.part" "$dest/$name"
  echo "voice ready: $name"
done
# drop models an earlier Stage fetched that voices.json no longer lists (they cost 60 MB each)
node -e '
const j = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
for (const v of Object.values(j.models)) if (v.url) for (const e of ["onnx", "onnx.json"]) console.log(`${v.url.split("/").pop()}.${e}`);
' "$here/voices.json" > "$dest/.keep"
for f in "$dest"/*.onnx "$dest"/*.onnx.json; do
  [ -e "$f" ] || continue
  grep -qxF "$(basename "$f")" "$dest/.keep" || { rm -f "$f"; echo "voice removed (no longer used): $(basename "$f")"; }
done
rm -f "$dest/.keep"
