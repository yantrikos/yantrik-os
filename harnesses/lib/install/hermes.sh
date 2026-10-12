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
# Then the desktop plugin, copied into Hermes and enabled, and Hermes's gateway, which is what
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

HARNESS=hermes
# The Python Hermes runs on, which has its YAML library and is where its plugins are installed.
hermes_bin=$(readlink -f "$(command -v hermes)") || fail "cannot tell where hermes is installed"
hermes_python="$(dirname "$hermes_bin")/python"
[ -x "$hermes_python" ] || fail "no Python beside $hermes_bin"
# `timeout`: a `hermes config` call was seen to hang on a fresh HERMES_HOME (review of #696), and
# an install that runs from a button must never hang.
config=$(timeout 30 hermes config path </dev/null) || fail "Hermes would not say where its config.yaml is"
env_file=$(timeout 30 hermes config env-path </dev/null 2>/dev/null) || env_file="$HOME/.hermes/.env"
backup_once "$config"
backup_once "$env_file"

say "copying the desktop plugin into ~/.hermes"
mkdir -p "$HOME/.hermes/plugins/yantrik"
cp -r "$plugin/." "$HOME/.hermes/plugins/yantrik/"
hermes plugins enable yantrik-desktop || fail "Hermes would not enable the desktop plugin"

# ── The desktop's tools, not Hermes's own ──
# Hermes's `terminal`, `file`, `code_execution`, `browser` and `web` are a second, ungraded route
# around every app on this desktop (README.md). `hermes config set` cannot write a list, so
# hermes_config.py does it with Hermes's own Python, after `plugins enable`, which edits the same
# lists, and checks the result with Hermes's own resolver.
say "giving Hermes's desktop platform the desktop's tools in $config"
"$hermes_python" "$INSTALL_LIB/hermes_config.py" apply "$config" \
    || fail "could not give Hermes's desktop platform the desktop's tools; config.yaml is as it was"

# ── Model ──
wire_model hermes

# ── Memory: YantrikDB, from the image ──
# Installed from the wheels at /opt/yantrik/memory (bake-memory.sh; there by image or by update),
# with no index: nothing is fetched from a catalog, git or PyPI here, and no TTY is needed (the Mac failure of #677 was
# `hermes plugins install` refusing the provider's dependencies without one). The provider and
# the engine go in with --no-deps; of their own dependencies, only what Hermes's environment lacks
# is added, so none of Hermes's locked packages is moved. requests is Hermes's own.
[ -d "$MEMORY_WHEELS" ] || fail "this image has no YantrikDB wheels at $MEMORY_WHEELS"
say "installing the YantrikDB memory provider into Hermes, from the image"
pip_install() {
    if command -v uv >/dev/null 2>&1; then
        uv pip install --python "$hermes_python" --offline --no-index --find-links "$MEMORY_WHEELS" --no-deps "$@" </dev/null
    else
        "$hermes_python" -m pip install --no-index --find-links "$MEMORY_WHEELS" --no-deps "$@" </dev/null
    fi
}
pip_install yantrikdb yantrikdb-hermes-plugin \
    || fail "the YantrikDB memory provider did not install from $MEMORY_WHEELS"
for dep in uuid_utils:uuid-utils click:click; do
    "$hermes_python" -c "import ${dep%%:*}" </dev/null 2>/dev/null && continue
    pip_install "${dep#*:}" || fail "the memory provider's dependency ${dep#*:} did not install from $MEMORY_WHEELS"
done
"$hermes_python" -c "import requests, yantrikdb, yantrikdb_hermes_plugin" </dev/null \
    || fail "the YantrikDB memory provider is installed but does not import in Hermes's Python"
# `--force` replaces the shim an earlier install left, which is the package's own.
"$(dirname "$hermes_bin")/yantrikdb-hermes" install --force </dev/null >/dev/null \
    || fail "the YantrikDB memory provider would not register itself with Hermes"
timeout 30 hermes config set memory.provider yantrikdb </dev/null \
    || fail "Hermes would not take yantrikdb as its memory"
# A provider runs beside Hermes's own MEMORY.md and USER.md (Hermes's agent_init.py). Left on,
# they are a second memory of the person outside YantrikDB.
timeout 30 hermes config set memory.memory_enabled false </dev/null || fail "Hermes would not turn off its own MEMORY.md"
timeout 30 hermes config set memory.user_profile_enabled false </dev/null || fail "Hermes would not turn off its own USER.md"
# TODO(#586): `YANTRIKDB_MODE=yantrik`, the person's one shared memory through the memory server
# (yantrikdb-hermes-plugin#93, baked in this image), once the door to it is tamper-proof. Until
# then any process running as the person could attach as `hermes` and be handed Hermes's memory
# credential, which is why #574 was held; so Hermes keeps a YantrikDB of its own, here, and every
# write carries `hermes` as its agent. Switching later is these two lines and a gateway restart.
env_line "$env_file" YANTRIKDB_MODE embedded
env_line "$env_file" YANTRIKDB_DB_PATH "$(harness_memory_db hermes)"
env_line "$env_file" YANTRIKDB_AGENT_NAME hermes
memory_dir hermes

# ── The desktop channel, paired with the owner only ──
# Hermes reads a platform's gate from the profile's own .env, not from what a plugin put in its
# environment (#676), so it is written there: the desktop platform's allowlist is the desktop's
# owner, the one user id the adapter gives every desktop turn (adapter.py, OWNER_ID). Never
# YANTRIK_ALLOW_ALL_USERS (an earlier install's is taken out) and never a wildcard. Whoever reaches
# the harness socket is the owner because the host checks the kernel's peer credentials at accept.
# A TCP address in YANTRIK_HARNESS is the dev path, which any local user can reach: there nobody is
# allowlisted, and Hermes asks for a pairing code as it would on any other platform.
env_unset "$env_file" YANTRIK_ALLOW_ALL_USERS
case "${YANTRIK_HARNESS:-}" in
    ""|off|unix:*|/*) env_line "$env_file" YANTRIK_ALLOWED_USERS owner ;;
    *) say "YANTRIK_HARNESS is a network address (the dev path), so nobody is allowlisted; Hermes will ask for a pairing code" ;;
esac

# Once more now that the memory provider is in: whatever it brought as a plugin toolset is marked
# seen for the desktop platform too, so Hermes does not turn it on there by itself.
"$hermes_python" "$INSTALL_LIB/hermes_config.py" apply "$config" >/dev/null \
    || fail "could not hold Hermes's desktop platform to the desktop's tools; see $config"

# Checked before the gateway starts, and again once it has, against the same file. A gateway is not
# left running on an allowlist that does not hold.
hermes_check() {
    "$hermes_python" "$INSTALL_LIB/hermes_config.py" check "$config"
}
stop_gateway() {
    systemctl --user stop hermes-gateway >/dev/null 2>&1 || true
}
hermes_check || { stop_gateway; fail "Hermes's desktop platform has more than the desktop's tools, and Hermes's gateway is stopped; see $config"; }

# Hermes's gateway as the person's own user service, started now and at every login: nothing
# else loads the plugin. `--if-missing` leaves one that is already there alone.
hermes gateway install --if-missing --start-now --start-on-login </dev/null \
    || { stop_gateway; fail "Hermes would not install its gateway service"; }
systemctl --user restart hermes-gateway || { stop_gateway; fail "Hermes's gateway would not start"; }

# Read back last, after everything else that edits config.yaml has run.
hermes_check || { stop_gateway; fail "Hermes's desktop platform still has Hermes's own tools, so its gateway is stopped; see $config"; }

# ── Proofs ──
prove hermes 120
say "Hermes is installed: it answers, remembers in YantrikDB and is paired with this desktop"
[ -s "$(harness_state hermes)/model" ] \
    || say "Hermes keeps its own model: choose it with Choose model on this row, or run: hermes model"
