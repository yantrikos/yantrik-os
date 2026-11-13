#!/usr/bin/env bash
# Sealed-run smoke test for the workshop (#1124): render the sample with no
# network, no HOME and no XDG dirs, with every write inside one scratch folder.
#   sealed-smoke.sh [episode.json]     exit 0 = rendered and nothing escaped
set -euo pipefail
STAGE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EP="${1:-$STAGE_DIR/examples/episode.json}"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
NODE="$(command -v node)"; FFMPEG_DIR="$(dirname "$(command -v ffmpeg)")"
RUN=(env -i PATH="$(dirname "$NODE"):$FFMPEG_DIR:/usr/bin:/bin" STAGE_OUT="$SCRATCH/out" STAGE_CONCURRENCY=1)
# no network (only loopback): a private network namespace when unshare allows it, else run as-is and say so
if unshare -rn true 2>/dev/null; then NET=(unshare -rn sh -c 'ip link set lo up; exec "$0" "$@"');  # loopback stays up: Chrome's own pipes and DevTools need it
   else NET=(); echo "sealed-smoke: cannot unshare the network here; running with it on" >&2; fi
"${NET[@]}" "${RUN[@]}" bash "$STAGE_DIR/build.sh" "$EP"
[ -s "$SCRATCH/out/contact.png" ] || { echo "sealed-smoke: no contact.png" >&2; exit 1; }
LEFT="$(find "$SCRATCH/out/work" -maxdepth 1 \( -name bundle -o -name tmp \) | wc -l)"
[ "$LEFT" = 0 ] || { echo "sealed-smoke: bundle or tmp left behind in work" >&2; exit 1; }
echo "sealed-smoke: ok"
