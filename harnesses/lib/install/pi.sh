#!/bin/sh
# Wire Pi to this desktop once npm.sh has installed it: pi.sh
#
# Pi has no MCP client and no memory provider of its own, so both arrive through the extension
# the harness passes it with `-e` (harnesses/pi/extension/yantrik-os.ts): the desktop's tools
# through yos-mcp, and YantrikDB's memory tools through yantrikdb-mcp, baked into the image.
# `pi.json` naming the gateway as Pi's provider is the model step's (#675).
set -eu
. "$(dirname "$0")/common.sh"
HARNESS=pi

command -v pi >/dev/null 2>&1 || fail "there is no pi in ~/.local/bin"
ours="$HOME/.config/yantrik/pi.json"
backup_once "$ours"

# ── Model ──
wire_model pi

# ── Memory: YantrikDB, from the image ──
# On by default wherever the image carries yantrikdb-mcp (yantrik_pi.py, `memory`); said in pi.json
# too when there is one, so the file shows it and Revert takes it back.
[ -x "$YANTRIKDB_MCP" ] || fail "this image has no yantrikdb-mcp at $YANTRIKDB_MCP"
memory_dir pi
if [ ! -f "$ours" ]; then
    # No model step on this image and no pi.json of the person's: Pi would run on its own default
    # provider (google), which is rarely what anyone meant. Not installed until it has a model.
    say "Pi will remember in YantrikDB; it needs its model first: set up pi.json on this row, then press Start"
    exit 0
fi
json_merge "$ours" '{"memory":"yantrikdb"}'
say "Pi remembers in YantrikDB"

# ── The desktop channel ──
start_unit pi

# ── Proofs ──
prove pi 120
say "Pi is installed: it answers, remembers in YantrikDB and is paired with this desktop"
