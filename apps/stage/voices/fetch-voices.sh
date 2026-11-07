#!/usr/bin/env bash
# Download the extra free Piper voices (rhasspy/piper-voices on Hugging Face)
# into voices/models. Run once at install time; render time needs no network.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
dest="${1:-$here/models}"
base="https://huggingface.co/rhasspy/piper-voices/resolve/main"
mkdir -p "$dest"
node -e '
const m = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).models;
for (const v of Object.values(m)) if (v.url) console.log(v.url);
' "$here/voices.json" | while read -r path; do
  name="$(basename "$path")"
  for ext in onnx onnx.json; do
    if [ ! -s "$dest/$name.$ext" ]; then
      curl -fsSL -o "$dest/$name.$ext.part" "$base/$path.$ext"
      mv -f "$dest/$name.$ext.part" "$dest/$name.$ext"
    fi
  done
  echo "voice ready: $name"
done
