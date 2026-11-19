#!/usr/bin/env bash
# Fetch the Piper voice the OS ships (en_US-ljspeech-medium) into a models/tts folder.
#
# Usage (as root, from build-debian-iso.sh): fetch-piper-voice.sh <tts-dir> <work-dir>
#
# The voice comes from rhasspy/piper-voices at a pinned commit, and both files are checked
# against pinned sha256 sums: a moved or tampered download fails the build instead of shipping.
# The model card at that commit says "License: public domain" and "Trained from scratch"; that
# claim is for the owner to read and confirm before an image ships (see THIRD-PARTY-NOTICES.md).
#
# Any other Piper voice left in the folder (an older build shipped en_US-lessac-medium) is
# removed, because the runtime (PiperTTS::from_dir) loads the first .onnx it finds there.
set -euo pipefail

TTS_DIR="${1:?usage: fetch-piper-voice.sh <tts-dir> <work-dir>}"
WORK_DIR="${2:?usage: fetch-piper-voice.sh <tts-dir> <work-dir>}"

VOICE="en_US-ljspeech-medium"
REV="e7229f7d33a10e145e109b886d661984201ac3c4"   # rhasspy/piper-voices commit
URL="https://huggingface.co/rhasspy/piper-voices/resolve/$REV/en/en_US/ljspeech/medium"
SHA_ONNX="6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22"
SHA_JSON="141d612cc0a95ed7efc1ca936b845c2364967f2e9217c5dbfcf69fc4d6c65860"

mkdir -p "$TTS_DIR"
if [ ! -f "$TTS_DIR/$VOICE.onnx" ]; then
    STAGE="$WORK_DIR/piper-voice-stage"
    rm -rf "$STAGE"
    mkdir -p "$STAGE"
    for ext in onnx onnx.json; do
        wget -q "$URL/$VOICE.$ext" -O "$STAGE/$VOICE.$ext"
    done
    echo "$SHA_ONNX  $STAGE/$VOICE.onnx" | sha256sum -c --quiet - \
        || { echo "Piper voice model does not match its pinned sha256" >&2; exit 1; }
    echo "$SHA_JSON  $STAGE/$VOICE.onnx.json" | sha256sum -c --quiet - \
        || { echo "Piper voice config does not match its pinned sha256" >&2; exit 1; }
    cp "$STAGE/$VOICE.onnx" "$STAGE/$VOICE.onnx.json" "$TTS_DIR/"
    rm -rf "$STAGE"
fi

for stale in "$TTS_DIR"/*.onnx "$TTS_DIR"/*.onnx.json; do
    [ -e "$stale" ] || continue
    case "$(basename "$stale")" in
        "$VOICE".onnx|"$VOICE".onnx.json) ;;
        *) rm -f "$stale" ;;
    esac
done
