#!/bin/sh
# Wire OpenClaw to this desktop once npm.sh has installed it: openclaw.sh
#
# The design's table (design/onboarding-one-model-one-memory-2026-10-10.md), with no question
# asked: the model step (#675's, when present), YantrikDB as an MCP server, the desktop's tools
# through the bridge, the tool profile that lets OpenClaw see them and nothing of its own, the
# streaming chat route with a gateway token only this harness and OpenClaw hold, then the
# harness's unit and the three proofs.
#
# Memory: OpenClaw's memory slot takes a memory plugin (our openclaw-memory-yantrikdb), but that
# plugin talks to a YantrikDB server over HTTP and this image runs none; so YantrikDB reaches
# OpenClaw as an MCP server (yantrikdb-mcp, baked into the image), which needs nothing running.
set -eu
. "$(dirname "$0")/common.sh"
HARNESS=openclaw

command -v openclaw >/dev/null 2>&1 || fail "there is no openclaw in ~/.local/bin"
oc_config=$(timeout 30 openclaw config file </dev/null 2>/dev/null | tail -n 1) || oc_config=""
[ -n "$oc_config" ] || oc_config="$HOME/.openclaw/openclaw.json"
ours="$HOME/.config/yantrik/openclaw.json"
env_file="$HOME/.config/yantrik/openclaw.env"
backup_once "$oc_config"
backup_once "$ours"
backup_once "$env_file"

# ── Model ──
wire_model openclaw

# mcp_server NAME JSON — one MCP server in OpenClaw's registry, added or replaced. `mcp add` is
# what the design names; `mcp set` replaces one that an earlier install already added.
mcp_server() {
    name=$1 json=$2
    shift 2
    timeout 60 openclaw mcp add "$name" "$@" </dev/null >/dev/null 2>&1 \
        || timeout 60 openclaw mcp set "$name" "$json" </dev/null >/dev/null \
        || fail "OpenClaw would not take the $name MCP server"
    has_mcp_server "$name" || fail "OpenClaw said it took the $name MCP server, but does not list it"
}

# has_mcp_server NAME — OpenClaw's own config holds the server. Asked of OpenClaw rather than
# assumed from the command's exit: VM 520 was left with no yantrikdb server and an install that
# carried on (#758).
has_mcp_server() {
    timeout 60 openclaw mcp show "$1" --json </dev/null >/dev/null 2>&1
}

# ── Memory: YantrikDB, from the image ──
[ -x "$YANTRIKDB_MCP" ] || fail "this image has no yantrikdb-mcp at $YANTRIKDB_MCP"
db=$(harness_memory_db openclaw)
memory_dir openclaw
say "giving OpenClaw YantrikDB as its memory"
# Started once before OpenClaw is told about it. `mcp add` probes a server and refuses to save one
# that has not answered within 5 seconds, and yantrikdb-mcp's first start creates the database and
# loads the engine, which took longer than that against OpenClaw 2026.9.6 here. `mcp set` saves
# without a probe, so the server is still added, but warmed it answers in about a second.
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"yantrik-install","version":"1"}}}' \
    | YANTRIKDB_DB_PATH="$db" YANTRIKDB_EMBEDDER=bundled timeout 120 "$YANTRIKDB_MCP" 2>/dev/null \
    | grep -q '"id": *1' || fail "yantrikdb-mcp did not start (it did not answer initialize)"
mcp_server yantrikdb \
    "{\"command\":\"$YANTRIKDB_MCP\",\"env\":{\"YANTRIKDB_DB_PATH\":\"$db\",\"YANTRIKDB_EMBEDDER\":\"bundled\"}}" \
    --command "$YANTRIKDB_MCP" --env "YANTRIKDB_DB_PATH=$db" --env YANTRIKDB_EMBEDDER=bundled
# A mind's memory, not the store's administration.
timeout 60 openclaw mcp tools yantrikdb --include remember,recall,forget,correct </dev/null >/dev/null \
    || fail "OpenClaw would not hold YantrikDB to its memory tools"

# ── The desktop channel ──
say "giving OpenClaw the desktop's tools"
mcp_server yantrik-os \
    '{"command":"/opt/yantrik/bin/yos-mcp","env":{"YOS_MCP_REQUESTER":"OpenClaw"}}' \
    --command /opt/yantrik/bin/yos-mcp --env YOS_MCP_REQUESTER=OpenClaw

# The gateway's shared token: this harness's own, made here, kept in a private env file its unit
# reads (yantrik-openclaw.service, EnvironmentFile) and handed to OpenClaw over stdin, never on a
# command line. A token the person already gave OpenClaw cannot be read back (OpenClaw shows it
# only on a terminal), and replacing it would cut off whatever else uses it: then this harness
# keeps the per-turn `openclaw agent` route, which needs no token, and says so.
token=$(sed -n 's/^OPENCLAW_GATEWAY_TOKEN=//p' "$env_file" 2>/dev/null | tail -n 1)
route=gateway
if [ -z "$token" ] && timeout 30 openclaw config get gateway.auth.token --json </dev/null >/dev/null 2>&1; then
    route=cli
    say "OpenClaw already has a gateway token of yours, so this harness uses the per-turn route and leaves it alone"
fi
if [ "$route" = gateway ]; then
    if [ -z "$token" ]; then
        token=$(od -An -tx1 -N32 /dev/urandom | tr -d ' \n') || fail "could not make a gateway token"
        env_line "$env_file" OPENCLAW_GATEWAY_TOKEN "$token"
    fi
    printf '{"gateway":{"auth":{"mode":"token","token":"%s"},"http":{"endpoints":{"chatCompletions":{"enabled":true}}}}}\n' "$token" \
        | timeout 60 openclaw config patch --stdin >/dev/null \
        || fail "OpenClaw would not take its chat route and gateway token"
fi
unset token
# Profile `minimal` plus the MCP bundle: the desktop's tools (graded, asked about) and YantrikDB,
# and none of OpenClaw's own exec, file or browser tools beside them.
printf '%s\n' '{"tools":{"profile":"minimal","alsoAllow":["bundle-mcp"]}}' \
    | timeout 60 openclaw config patch --stdin >/dev/null \
    || fail "OpenClaw would not take the minimal tool profile"
if [ "$route" = gateway ]; then
    json_merge "$ours" '{"route":"gateway","token_env":"OPENCLAW_GATEWAY_TOKEN"}'
    timeout 120 openclaw gateway install </dev/null >/dev/null || fail "OpenClaw would not install its gateway service"
    timeout 120 openclaw gateway restart </dev/null >/dev/null \
        || timeout 120 openclaw gateway start </dev/null >/dev/null \
        || fail "OpenClaw's gateway would not start"
else
    json_merge "$ours" '{"route":"cli"}'
fi

# Asked again after the gateway has been installed and restarted, which rewrites OpenClaw's config.
for server in yantrikdb yantrik-os; do
    has_mcp_server "$server" || fail "OpenClaw's $server MCP server is gone after its gateway restarted; press Install again"
done

start_unit openclaw

# ── Proofs ──
prove openclaw 120
say "OpenClaw is installed: it answers, remembers in YantrikDB and is paired with this desktop"
