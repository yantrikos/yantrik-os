# Shared by every harness installer here. Sourced, not run.
#
# The Install button on Settings > Minds runs one of these as the person, with no terminal
# and no root, and streams what it prints into the row. So every line printed is written for
# that row, and everything lands in the person's home:
#
#   ~/.local/bin    the one place a harness's command is put. The desktop looks here as well as
#                   on PATH (crates/yantrik-ui/src/harness_catalogue.rs, `user_bin_dir`), and
#                   each harness's unit puts it on PATH, so nothing depends on a login shell
#                   having read ~/.profile first.
#   ~/.local/node   Node, when the machine has none new enough. The image ships none.

USER_BIN="$HOME/.local/bin"
mkdir -p "$USER_BIN"
case ":$PATH:" in *":$USER_BIN:"*) ;; *) PATH="$USER_BIN:$PATH" ;; esac
export PATH

say() { printf '%s\n' "$*"; }
fail() { printf 'install failed: %s\n' "$*" >&2; exit 1; }

# This directory: the installers, proofs.py beside lib/, and model.sh when the model half (#675)
# has put one here. Every installer is run as `sh <this dir>/<name>.sh`, so $0 is beside us.
INSTALL_LIB=$(cd "$(dirname "$0")" && pwd)
HARNESS_LIB=$(dirname "$INSTALL_LIB")

# YantrikDB as every mind's memory (#677), baked by deploy/yantrik-os/bake-memory.sh into the
# release bundle's memory/ and never fetched at install time: the engine and the Hermes provider as
# wheels, and yantrikdb-mcp on the image's Python. `yantrik-update apply` installs it and the ISO
# copies it from the bundle, so an updated machine has it as an imaged one does. Root's and outside
# /opt/yantrik/share, so neither the person nor a harness can swap what an install feeds to pip.
# YANTRIK_MEMORY_DIR moves it, for a checkout or a test.
MEMORY_DIR=${YANTRIK_MEMORY_DIR:-/opt/yantrik/memory}
# shellcheck disable=SC2034  # read by the installers that source this
MEMORY_WHEELS="$MEMORY_DIR/wheels"
# shellcheck disable=SC2034
YANTRIKDB_MCP="$MEMORY_DIR/bin/yantrikdb-mcp"

# What an install leaves for the proofs and for Revert to read: which steps ran, never a secret.
# `installed` in it is written only once all three proofs pass, and every manifest requires it,
# so Settings shows a harness as installed only when it was proved (harness.yaml, `requires`).
harness_state() { printf '%s\n' "$HOME/.local/state/yantrik/harnesses/$1"; }

# harness_memory_db HARNESS — the harness's own YantrikDB file. One per harness, not one shared
# file: the engine keeps its index in the process that opened it and does not see another
# process's later writes (measured on engine 0.23.2), so two running harnesses on one file would
# each recall a different memory. The person's one shared memory is the `yantrik` mode through
# the memory server, which waits on #586 (see hermes.sh).
harness_memory_db() { printf '%s\n' "${XDG_DATA_HOME:-$HOME/.local/share}/yantrik/memory/$1.db"; }

# memory_dir HARNESS — the directory of the harness's YantrikDB file, made and kept private: 700,
# and any file already in it 600. What a mind remembers about the person is never readable by
# another account, whatever the umask the install ran under or the engine creates its file with.
memory_dir() {
    dir=$(dirname "$(harness_memory_db "$1")")
    ( umask 077 && mkdir -p "$dir" ) && chmod 700 "$dir" || fail "could not make the memory directory $dir private"
    for f in "$dir"/*; do
        [ -f "$f" ] && [ ! -L "$f" ] && { chmod 600 "$f" || fail "could not make $f private"; }
    done
    return 0
}

# backup_once FILE — FILE copied to FILE.before-yantrik once, before the first change an install
# makes to it, so Revert (revert.sh) can put the person's own file back. A second install never
# replaces the backup with the first install's output. A file that did not exist is recorded as
# such, and Revert removes what the install created.
backup_once() {
    [ -e "$1.before-yantrik" ] || [ -e "$1.before-yantrik.absent" ] && return 0
    if [ -e "$1" ]; then
        ( umask 077 && cp -p "$1" "$1.before-yantrik" ) || fail "could not keep a copy of $1"
    else
        mkdir -p "$(dirname "$1")" && ( umask 077 && : > "$1.before-yantrik.absent" ) \
            || fail "could not note that $1 was not there"
    fi
    state=$(harness_state "${HARNESS:-unknown}")
    mkdir -p "$state" || fail "could not write $state"
    grep -qxF "$1" "$state/files" 2>/dev/null || printf '%s\n' "$1" >> "$state/files"
}

# wire_model HARNESS — the model step of an install (design/onboarding-one-model-one-memory-
# 2026-10-10.md, "The flow"): #675's `yantrik-ui --wire-model`, no question and no TTY. It
# records what happened in the harness's state/model for the "Can it answer?" proof (picked,
# failed, or absent for its own model) and always returns 0: installers run under set -e, and
# a model step that did not run must never abort the install.
wire_model() {
    state=$(harness_state "$1")
    mkdir -p "$state" || fail "could not write $state"
    # The image's own desktop first: PATH starts with ~/.local/bin, where harness npm packages put
    # their binaries (security review, #675). YANTRIK_UI names the binary for tests; a desktop
    # older than the model step (no --wire-model in it) is the same as none.
    ui=${YANTRIK_UI:-/opt/yantrik/bin/yantrik-ui}
    [ -x "$ui" ] || ui=$(command -v yantrik-ui 2>/dev/null || echo "$ui")
    if [ -x "$ui" ] && grep -q -e '--wire-model' "$ui" 2>/dev/null; then
        # #675's model step: the harness's own config pointed at the gateway, `model: picked`,
        # its file kept for Revert. It never stops the install: a step that did not run is said,
        # recorded as failed for the proofs, and Settings → Minds → Use Yantrik models retries.
        # Only its last line is kept: that is the one it writes for a person (logs go before it).
        if out=$(RUST_LOG=error "$ui" --wire-model "$1" </dev/null 2>&1); then
            out=$(printf '%s\n' "$out" | tail -n 1)
            case $out in
                *"uses your model"*) printf 'picked\n' > "$state/model" ;;
                *) rm -f "$state/model" ;;   # its own models, a saved provider, or the Mind's card
            esac
            say "model: $out"
            return 0
        fi
        out=$(printf '%s\n' "$out" | tail -n 1)
        say "model: $1 was not pointed at your model ($out); Settings → Minds → Use Yantrik models does it"
        printf 'failed\n' > "$state/model"
        return 0
    fi
    rm -f "$state/model"
    say "model: its own (no desktop here to point it at your model; Settings → Minds does it later)"
}

# Node, pinned. Updated by hand with the checksums nodejs.org publishes in SHASUMS256.txt for
# this release: a download that does not match is refused, not unpacked. The .tar.gz, not the
# smaller .tar.xz, because the image has no xz.
NODE_VERSION=24.21.0
NODE_SHA256_X64=6e1db87ef58b8819e5d5402eff1536491b18edd8eb7bee5ef7897876e88dc5ff
NODE_SHA256_ARM64=724282c3b43aec998aa9527380465b45d229e021b58035f5f4f63095eabfe5d5

# env_line FILE KEY VALUE — set KEY=VALUE in a dotenv file: the KEY= line is replaced in place,
# or the pair appended when absent, every other line kept as it was. Hermes 0.21.x reads its
# authorization flags from its own .env before a plugin loads, so hermes.sh writes one there.
# The umask 077 subshell means the file is never readable by others, at no moment, even when it
# is being created; the temp file next to it and the mv mean a reader sees the old file or the
# new one, never half of either. The value is only ever written, never printed, and the line is
# matched on `KEY=` so a key that only shares a prefix (FOOBAR for FOO) is left alone.
env_line() {
    file=$1 key=$2 value=$3
    (
        umask 077
        dir=$(dirname "$file") || exit 1
        mkdir -p "$dir" || exit 1
        tmp=$(mktemp "$dir/.env.XXXXXX") || exit 1
        trap 'rm -f "$tmp"' EXIT INT TERM
        found=0
        if [ -f "$file" ]; then
            while IFS= read -r line || [ -n "$line" ]; do
                case $line in
                    "$key="*) printf '%s=%s\n' "$key" "$value"; found=1 ;;
                    *) printf '%s\n' "$line" ;;
                esac
            done < "$file" > "$tmp"
        fi
        # Appending after a last line that had no newline: the read loop above already gave that
        # line its newline, so the pair always lands on a line of its own.
        [ "$found" -eq 1 ] || printf '%s=%s\n' "$key" "$value" >> "$tmp"
        mv "$tmp" "$file" || exit 1
    ) || fail "could not write $1"
}

# env_unset FILE KEY — every KEY= line removed from a dotenv file, the rest kept as it was, the
# same private, atomic way env_line writes. Nothing to do when the file or the key is not there.
env_unset() {
    file=$1 key=$2
    [ -f "$file" ] || return 0
    grep -q "^$key=" "$file" || return 0
    (
        umask 077
        tmp=$(mktemp "$(dirname "$file")/.env.XXXXXX") || exit 1
        trap 'rm -f "$tmp"' EXIT INT TERM
        while IFS= read -r line || [ -n "$line" ]; do
            case $line in
                "$key="*) ;;
                *) printf '%s\n' "$line" ;;
            esac
        done < "$file" > "$tmp"
        mv "$tmp" "$file" || exit 1
    ) || fail "could not write $1"
}

# json_merge FILE JSON — the keys in JSON set in the JSON object in FILE, every other key kept as
# it was; the file private (600) and replaced whole, so a reader sees the old file or the new one.
# A file that is not a JSON object is refused, never replaced.
json_merge() {
    python3 - "$1" "$2" <<'PY' || fail "could not write $1"
import json, os, sys, tempfile
path, patch = sys.argv[1], json.loads(sys.argv[2])
data = {}
if os.path.exists(path):
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    if not isinstance(data, dict):
        sys.exit("%s is not a JSON object; not touching it" % path)
data.update(patch)
os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path) or ".", prefix=".yantrik-")
with os.fdopen(fd, "w", encoding="utf-8") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
os.replace(tmp, path)
PY
}

# start_unit HARNESS — the harness's own user unit enabled and (re)started, so it attaches now and
# at every login.
start_unit() {
    systemctl --user daemon-reload >/dev/null 2>&1 || true
    systemctl --user enable "yantrik-$1.service" >/dev/null 2>&1 \
        || fail "could not enable yantrik-$1.service"
    systemctl --user restart "yantrik-$1.service" || fail "yantrik-$1.service would not start"
}

# prove HARNESS [WAIT] — the three proofs (lib/proofs.py): "Can it answer?", "Can it remember?",
# "Can it reach the desktop?". An install is not finished until all three pass; the rows it
# prints are the ones Settings shows, each failure with its one-press fix.
prove() {
    rm -f "$(harness_state "$1")/installed"
    python3 "$HARNESS_LIB/proofs.py" "$1" --wait "${2:-90}" \
        || fail "$1 is not installed yet: not every proof passed, and the rows above say what to press"
    mkdir -p "$(harness_state "$1")" && date -u +%Y-%m-%dT%H:%M:%SZ > "$(harness_state "$1")/installed" \
        || fail "could not record that $1 passed its proofs"
}

# node_at_least MAJOR.MINOR — the node on PATH is at least that version.
node_at_least() {
    command -v node >/dev/null 2>&1 || return 1
    node -e 'const h = process.versions.node.split(".").map(Number);
             const w = process.argv[1].split(".").map(Number);
             process.exit(h[0] > w[0] || (h[0] === w[0] && h[1] >= (w[1] || 0)) ? 0 : 1)' "$1" 2>/dev/null
}

# ensure_node MAJOR.MINOR — a node at least that new on PATH, installing the pinned one if not.
ensure_node() {
    want=$1
    if node_at_least "$want"; then
        say "Node $(node --version) is here"
        return 0
    fi
    case "$(uname -m)" in
        x86_64|amd64) arch=x64; sum=$NODE_SHA256_X64 ;;
        aarch64|arm64) arch=arm64; sum=$NODE_SHA256_ARM64 ;;
        *) fail "no Node build for $(uname -m)" ;;
    esac
    name="node-v$NODE_VERSION-linux-$arch"
    dest="$HOME/.local/$name"
    # ~/.local/node is a link this script owns. A real directory there is somebody's own Node,
    # which `ln -sfn` would not replace but quietly put a link inside of.
    if [ -e "$HOME/.local/node" ] && [ ! -L "$HOME/.local/node" ]; then
        fail "~/.local/node is a directory of your own; move it aside and press Install again"
    fi
    if [ ! -x "$dest/bin/node" ]; then
        say "fetching Node $NODE_VERSION from nodejs.org"
        # In ~/.local, so the move below is a rename on one filesystem rather than a copy out of
        # /tmp; removed however this ends, a killed job included.
        tmp=$(mktemp -d "$HOME/.local/.node-XXXXXX") || fail "no temporary directory in ~/.local"
        trap 'rm -rf "$tmp"' EXIT; trap 'exit 143' INT TERM
        curl -fsSL --retry 3 -o "$tmp/node.tar.gz" "https://nodejs.org/dist/v$NODE_VERSION/$name.tar.gz" \
            || fail "could not download Node $NODE_VERSION"
        got=$(sha256sum "$tmp/node.tar.gz" | cut -d' ' -f1)
        [ "$got" = "$sum" ] || fail "the Node download did not match its checksum"
        # Unpacked beside the download and moved into place whole, so a job stopped halfway
        # leaves no half a Node for the next run to take for a finished one.
        tar -xzf "$tmp/node.tar.gz" -C "$tmp" || fail "could not unpack Node"
        rm -rf "$dest"
        mv "$tmp/$name" "$dest" || fail "could not put Node in ~/.local"
    fi
    ln -sfn "$dest" "$HOME/.local/node"
    for tool in node npm npx; do
        ln -sf "$HOME/.local/node/bin/$tool" "$USER_BIN/$tool"
    done
    node_at_least "$want" || fail "Node $NODE_VERSION is unpacked but does not run, or is older than $want"
    say "Node $(node --version) installed in ~/.local/node"
}

