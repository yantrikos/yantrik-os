#!/bin/sh
# Wire DeepSeek to this desktop: deepseek.sh
#
# The harness is stdlib Python in the image, so there is nothing to fetch. `deepseek.json` naming
# the gateway endpoint, with the gateway token in its key env, is the model step's (#675). The
# desktop's tools come through yos-mcp and YantrikDB's memory tools through yantrikdb-mcp, both
# offered by the harness itself (yantrik_harness.WithMemory).
set -eu
. "$(dirname "$0")/common.sh"
HARNESS=deepseek

ours="$HOME/.config/yantrik/deepseek.json"
backup_once "$ours"

# ── Model ──
wire_model deepseek

# ── Memory: YantrikDB, from the image ──
# On by default wherever the image carries yantrikdb-mcp (yantrik_deepseek.py, `memory`); said in
# deepseek.json too when there is one, so the file shows it and Revert takes it back.
[ -x "$YANTRIKDB_MCP" ] || fail "this image has no yantrikdb-mcp at $YANTRIKDB_MCP"
memory_dir deepseek
if [ ! -f "$ours" ]; then
    # No model step on this image and no deepseek.json of the person's: nothing to answer with.
    say "DeepSeek will remember in YantrikDB; it needs its model first: set up deepseek.json on this row, then press Start"
    exit 0
fi
json_merge "$ours" '{"memory":"yantrikdb"}'
say "DeepSeek remembers in YantrikDB"

# ── The desktop channel ──
start_unit deepseek

# ── Proofs ──
prove deepseek 120
say "DeepSeek is installed: it answers, remembers in YantrikDB and is paired with this desktop"
