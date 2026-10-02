#!/bin/sh
# Install Hermes and attach it to this desktop: hermes.sh PLUGIN_DIR
#
# Hermes itself first, when it is not here, by Hermes's own installer: it fetches a pinned,
# hash-checked uv and Python into ~/.hermes, clones Hermes into ~/.hermes/hermes-agent and puts
# `hermes` in ~/.local/bin, all without root. `--non-interactive` because this runs from a
# button with no terminal; the two things it then skips (choosing a model, and the gateway
# service) are done below or named at the end. The browser and computer-use tools are left
# out: this desktop is the tool surface Hermes drives, and Chromium alone is a large download.
# Either can be added later with `hermes pm install agent-browser` / `cua-driver`.
#
# Then the desktop plugin, copied into Hermes and enabled; the YantrikDB memory provider, so
# Hermes's memory is this machine's and not one of its own; and Hermes's gateway, which is what
# loads the plugin and so what attaches to the desktop.
set -eu
. "$(dirname "$0")/common.sh"

[ $# -eq 1 ] || fail "usage: hermes.sh PLUGIN_DIR"
plugin=$1
[ -f "$plugin/plugin.yaml" ] || fail "no desktop plugin at $plugin"

# The commit this was last tried against, on Hermes's main line, which is what Hermes's own
# instructions install. The installer is fetched from that commit and told to check out that
# commit, so the script and the tree it sets up always match; `hermes update` moves it on
# afterwards, as it would for anyone. (Release tags carry an older installer that stops on a
# machine with no C++ compiler, which this image is.)
HERMES_COMMIT=38d3dbce4adbb9074a34214eb330237a129e5904
INSTALLER="https://raw.githubusercontent.com/NousResearch/hermes-agent/$HERMES_COMMIT/scripts/install.sh"
# That commit's install.sh, hashed when the pin was moved. Both are bumped together, by hand.
INSTALLER_SHA256=9ed50b51fe072df4ece6dc9a72c1ec8d78bc7907eec16a61d8a2a0ef4ec45884

if ! command -v hermes >/dev/null 2>&1; then
    command -v git >/dev/null 2>&1 || fail "Hermes needs git"
    command -v bash >/dev/null 2>&1 || fail "Hermes's installer needs bash"
    say "fetching Hermes"
    tmp=$(mktemp -d) || fail "no temporary directory"
    trap 'rm -rf "$tmp"' EXIT
    curl -fsSL --retry 3 -o "$tmp/install.sh" "$INSTALLER" \
        || fail "could not download Hermes's installer"
    [ "$(sha256sum "$tmp/install.sh" | cut -d' ' -f1)" = "$INSTALLER_SHA256" ] \
        || fail "Hermes's installer did not match its checksum"
    bash "$tmp/install.sh" --commit "$HERMES_COMMIT" --non-interactive \
        --skip-browser --skip-computer-use </dev/null \
        || fail "Hermes's own installer stopped; its last lines are above"
    command -v hermes >/dev/null 2>&1 || fail "Hermes installed, but there is no hermes in ~/.local/bin"
fi
say "$(hermes --version 2>/dev/null | head -n 1 || echo Hermes) is here"

say "copying the desktop plugin into ~/.hermes"
mkdir -p "$HOME/.hermes/plugins/yantrik"
cp -r "$plugin/." "$HOME/.hermes/plugins/yantrik/"
hermes plugins enable yantrik-desktop || fail "Hermes would not enable the desktop plugin"

# The Python Hermes runs on, which has its YAML library and is where its plugins are installed.
hermes_bin=$(readlink -f "$(command -v hermes)") || fail "cannot tell where hermes is installed"
hermes_python="$(dirname "$hermes_bin")/python"
[ -x "$hermes_python" ] || fail "no Python beside $hermes_bin"

# The desktop's tools and not Hermes's own: Hermes's `terminal`, `file`, `code_execution`,
# `browser` and `web` are a second, ungraded route around every app on this desktop. A live
# machine whose install skipped this ran Hermes's own terminal as the person, past every shell
# approval. `hermes config set` cannot write a list, so hermes_config.py does it with Hermes's own
# Python, keeping every other key, after `plugins enable`, which edits the same lists.
config=$(hermes config path) || fail "Hermes would not say where its config.yaml is"
say "giving Hermes's desktop platform the desktop's tools in $config"
"$hermes_python" "$(dirname "$0")/hermes_config.py" apply "$config" \
    || fail "could not give Hermes's desktop platform the desktop's tools; config.yaml is as it was"

# Hermes's memory is this machine's YantrikDB, the one Yantrik Mind keeps (harness.yaml's
# `memory: yantrikdb`), never a second memory of the person in ~/.hermes. Hermes finds a memory
# provider as a directory in ~/.hermes/plugins and switches to it with `memory.provider`. The
# provider's own package puts that directory there (`yantrikdb-hermes install`, a shim importing
# the package), and it is installed from a pinned reference into the Python Hermes runs on.
# Not `hermes plugins install`: that clones whatever the repository's default branch holds today.
# In `yantrik` mode the provider presents, on every call, the credential the desktop hands Hermes
# with each turn (YANTRIK_MEMORY_CREDENTIAL and YANTRIK_MEMORY_URL, set by adapter.py); the
# desktop grants it once the install the person pressed has worked, never for one a mind asked for.
# v0.28.0, the release with `yantrik` mode (yantrikos/yantrikdb-hermes-plugin#93), by commit: a
# tag can be moved and a commit cannot. Moved on by hand, with the review of what changed.
YANTRIKDB_PLUGIN="yantrikdb-hermes-plugin @ git+https://github.com/yantrikos/yantrikdb-hermes-plugin@e3ad889d1890cfe04057641051da30e7d6a35d74"
# Hermes picks a memory provider by its directory in ~/.hermes/plugins, which this package names.
MEMORY_PROVIDER=yantrikdb

command -v git >/dev/null 2>&1 || fail "the YantrikDB memory provider is fetched with git, and there is no git"
say "installing the YantrikDB memory provider into Hermes"
# Hermes's own installer makes its environment with uv, which leaves pip out of it, and puts uv
# in ~/.local/bin, which common.sh has put on PATH.
uv=$(command -v uv || true)
if [ -n "$uv" ]; then
    "$uv" pip install --python "$hermes_python" "$YANTRIKDB_PLUGIN" </dev/null \
        || fail "the YantrikDB memory provider did not install"
else
    "$hermes_python" -m pip install "$YANTRIKDB_PLUGIN" </dev/null \
        || fail "the YantrikDB memory provider did not install (and there is no uv to try)"
fi
# `--force` replaces the shim an earlier install left, which is the package's own and nothing of
# the person's. Its "next steps" are not shown: they suggest YANTRIKDB_MODE=embedded, the
# separate memory this install exists to avoid.
"$(dirname "$hermes_bin")/yantrikdb-hermes" install --force </dev/null >/dev/null \
    || fail "the YantrikDB memory provider would not register itself with Hermes"
hermes config set memory.provider "$MEMORY_PROVIDER" || fail "Hermes would not take $MEMORY_PROVIDER as its memory"
# A provider runs beside Hermes's own MEMORY.md and USER.md and does not replace them (Hermes's
# agent_init.py and system_prompt.py). Left on, they are a second memory of the person that
# Yantrik Mind never sees. Off, Hermes's own memory tool says it is not available, and what it
# would have added still reaches the provider.
hermes config set memory.memory_enabled false || fail "Hermes would not turn off its own MEMORY.md"
hermes config set memory.user_profile_enabled false || fail "Hermes would not turn off its own USER.md"

# Hermes's own environment file, read into the gateway at start. Private, as Hermes keeps it, and
# only ever added to: a value the person set there is theirs.
env_file="$HOME/.hermes/.env"
( umask 077 && touch "$env_file" ) || fail "cannot write $env_file"
chmod 600 "$env_file" || fail "cannot make $env_file private"

# env_default KEY VALUE — KEY=VALUE at the end of ~/.hermes/.env, unless KEY is set there already.
# Prints what KEY is now, so a different value the person chose is said rather than overwritten.
env_default() {
    was=$(sed -n "s/^[[:space:]]*\(export[[:space:]]\{1,\}\)\{0,1\}$1=//p" "$env_file" | tail -n 1)
    if [ -n "$was" ] || grep -Eq "^[[:space:]]*(export[[:space:]]+)?$1=" "$env_file"; then
        printf '%s\n' "$was"
        return 0
    fi
    # A file whose last line has no newline would have this glued onto the end of it.
    if [ -s "$env_file" ] && [ -n "$(tail -c 1 "$env_file")" ]; then
        printf '\n' >>"$env_file" || fail "cannot write $env_file"
    fi
    printf '%s=%s\n' "$1" "$2" >>"$env_file" || fail "cannot write $env_file"
    printf '%s\n' "$2"
}

mode=$(env_default YANTRIKDB_MODE yantrik)
case "$mode" in
    yantrik|\"yantrik\"|\'yantrik\') ;;
    *) say "YANTRIKDB_MODE is ${mode:-empty} in ~/.hermes/.env, so Hermes keeps its own YantrikDB rather than this machine's; set it to yantrik to share Yantrik Mind's" ;;
esac
# The desktop's harness socket is in a directory only the person's own account can open (0700),
# so whoever is typing in the chat panel is the person and there is nobody to pair with. The
# plugin's register() sets this too, but Hermes reads a platform's gate from the profile's own
# secrets (gateway/platforms/_shared.py, platform_gate_env), not from what a plugin put in its
# environment, and the first message on a fresh machine was answered with a pairing code.
env_default YANTRIK_ALLOW_ALL_USERS true >/dev/null

# Hermes's gateway as the person's own user service, started now and at every login: nothing
# else loads the plugin. `--if-missing` leaves one that is already there alone.
hermes gateway install --if-missing --start-now --start-on-login </dev/null \
    || fail "Hermes would not install its gateway service"
systemctl --user restart hermes-gateway || fail "Hermes's gateway would not start"

# Read back last, after everything else that edits config.yaml has run: an install that leaves
# Hermes's own terminal on the desktop platform is not finished, whatever else worked.
"$hermes_python" "$(dirname "$0")/hermes_config.py" check "$config" \
    || fail "Hermes's desktop platform still has Hermes's own tools; see $config"

# The desktop opens `hermes model` next (the manifest's `configure`); by hand it is the same.
say "Hermes is installed. Next, choose its model: Choose model on this row, or run: hermes model"
