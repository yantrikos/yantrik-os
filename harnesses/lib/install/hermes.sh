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
. "$(dirname "$0")/hermes-launcher.sh"
. "$(dirname "$0")/hermes-plugins.sh"

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
    heal_hermes_launcher "$HOME" || fail "Hermes's launcher still points into /tmp"
    rm -rf "$tmp"; trap - EXIT
    hermes --version >/dev/null 2>&1 || fail "Hermes installed, but hermes will not run"
fi
heal_hermes_launcher "$HOME" || fail "Hermes's launcher still points into /tmp"
say "$(hermes --version 2>/dev/null | head -n 1 || echo Hermes) is here"

HARNESS=hermes
# The Python Hermes runs on, which has its YAML library and Hermes's own modules: asked of
# hermes_runtime.py (beside proofs.py, which asks the same), never written down, because a newer
# Hermes rebuilds its venv as a new generation on an update and the path moves.
hermes_python=$(python3 "$HARNESS_LIB/hermes_runtime.py" </dev/null) \
    || fail "cannot find the Python Hermes runs on: none beside hermes, and no venv on Hermes's own sys.path"
# `timeout`: a `hermes config` call was seen to hang on a fresh HERMES_HOME (review of #696), and
# an install that runs from a button must never hang. The first command of a new Hermes 0.21 first
# finishes Hermes's own update (venv_sync.prepare_launch, from hermes_bootstrap; `--version` skips
# it), which took 102 seconds on VM 541: this is that command, so it is said and given three
# minutes, and the calls after it are quick until a plugin is enabled (see below).
say "asking Hermes where its settings are (a new Hermes first finishes its own setup: this can take a couple of minutes)"
said=0
config=$(timeout 180 hermes config path </dev/null) || said=$?
case $said in
    0) ;;
    124) fail "Hermes did not finish its own setup within three minutes; press Install again to let it carry on" ;;
    *) fail "Hermes would not say where its config.yaml is" ;;
esac
env_file=$(timeout 30 hermes config env-path </dev/null 2>/dev/null) || env_file="$HOME/.hermes/.env"
backup_once "$config"
backup_once "$env_file"

# Written beside the installed one and swapped in whole, by the same function refresh-plugins.sh
# uses after an update, so a reinstall and a refresh both leave exactly what ships.
say "copying the desktop plugin into ~/.hermes"
new_desktop=$(hermes_staging yantrik)-new || fail "could not make ~/.hermes/.yantrik-staging"
hermes_desktop_plugin_build "$plugin" "$new_desktop" \
    && hermes_swap_dir "$new_desktop" "$HOME/.hermes/plugins/yantrik" \
    || fail "could not put the desktop plugin in ~/.hermes/plugins/yantrik"
timeout 60 hermes plugins enable yantrik-desktop </dev/null || fail "Hermes would not enable the desktop plugin"

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
# `hermes plugins install` refusing the provider's dependencies without one).
#
# Built beside the installed one by hermes-plugins.sh, which says why it goes into vendor/ and not
# Hermes's venv, then swapped in whole, so a failure leaves the earlier provider as it was.
[ -d "$MEMORY_WHEELS" ] || fail "this image has no YantrikDB wheels at $MEMORY_WHEELS"
say "installing the YantrikDB memory provider beside Hermes, from the image"
plugin_dir="$HOME/.hermes/plugins/yantrikdb"
new_plugin=$(hermes_staging yantrikdb)-new || fail "could not make ~/.hermes/.yantrik-staging"
hermes_yantrikdb_plugin_build "$new_plugin" "$hermes_python"
hermes_swap_dir "$new_plugin" "$plugin_dir" || fail "could not put the YantrikDB plugin in $plugin_dir"
# yantrikdb as Hermes's memory, with Hermes's own MEMORY.md and USER.md off: a provider runs beside
# them (Hermes's agent_init.py), and left on they are a second memory of the person outside
# YantrikDB. Written by hermes_config.py, not `hermes config set`, which on a fresh Hermes 0.21
# first finishes Hermes's own update and took 102 seconds for one key (VM 541, 13 Oct).
"$hermes_python" "$INSTALL_LIB/hermes_config.py" memory "$config" \
    || fail "could not make yantrikdb Hermes's memory with its own MEMORY.md and USER.md off; see $config"
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

# Enabling a plugin leaves a newer Hermes owing itself a dependency sync, which its next command
# runs first (hermes_bootstrap; 90 to 120 seconds on VM 541, 13 Oct). It is run here, said and given
# five minutes, so that the gateway's install, its start and the proofs after it are not the ones
# caught waiting on it. An older Hermes answers at once.
say "letting Hermes bring itself up to date for the new plugins (this can take a couple of minutes)"
settled=0
timeout 300 hermes config path </dev/null >/dev/null || settled=$?
case $settled in
    0) ;;
    124) fail "Hermes did not finish updating itself within five minutes; press Install again to let it carry on" ;;
    *) fail "Hermes would not start after its plugins were added; its own message is above" ;;
esac
# Checked again in the Python Hermes runs on after that update, so the check is of what will run:
# the provider and the engine both from vendor/, not a copy something else put in Hermes's venv.
hermes_python=$(python3 "$HARNESS_LIB/hermes_runtime.py" </dev/null) \
    || fail "cannot find the Python Hermes runs on after its update"
"$hermes_python" -c '
import sys
vendor = sys.argv[1]
sys.path.append(vendor)
import yantrikdb, yantrikdb_hermes_plugin
for module in (yantrikdb, yantrikdb_hermes_plugin):
    if not module.__file__.startswith(vendor + "/"):
        sys.exit("%s loads from %s, not from %s" % (module.__name__, module.__file__, vendor))
' "$plugin_dir/vendor" </dev/null \
    || fail "the YantrikDB memory provider that would run is not the one this install put in $plugin_dir/vendor"

# Checked before the gateway starts, and again once it has, against the same file. A gateway is not
# left running on an allowlist that does not hold.
hermes_check() {
    # Asked again: the gateway install may have run Hermes's own update, which moves its venv.
    hermes_python=$(python3 "$HARNESS_LIB/hermes_runtime.py" </dev/null) || return 1
    "$hermes_python" "$INSTALL_LIB/hermes_config.py" check "$config"
}
stop_gateway() {
    systemctl --user stop hermes-gateway >/dev/null 2>&1 || true
}
# Hermes's gateway as the person's own user service, started now and at every login: nothing
# else loads the plugin. A Hermes that knows `--if-missing` leaves a unit that is already there
# alone. An older one (0.14.0 on VM 520 knows only --force, --system and --run-as-user, and
# refused the rest) is given a unit only when there is none, and is enabled for login here; the
# restart after this starts it either way.
gateway_install() {
    if timeout 30 hermes gateway install --help </dev/null 2>/dev/null | grep -q -e '--if-missing'; then
        timeout 180 hermes gateway install --if-missing --start-now --start-on-login </dev/null
        return
    fi
    if ! systemctl --user cat hermes-gateway >/dev/null 2>&1; then
        timeout 180 hermes gateway install </dev/null || return 1
    fi
    systemctl --user enable hermes-gateway >/dev/null 2>&1
}
hermes_check || { stop_gateway; fail "Hermes's desktop platform has more than the desktop's tools, and Hermes's gateway is stopped; see $config"; }

gateway_install || { stop_gateway; fail "Hermes would not install its gateway service"; }
systemctl --user restart hermes-gateway || { stop_gateway; fail "Hermes's gateway would not start"; }

# Read back last, after everything else that edits config.yaml has run.
hermes_check || { stop_gateway; fail "Hermes's desktop platform still has Hermes's own tools, so its gateway is stopped; see $config"; }

# ── Proofs ──
prove hermes 120
say "Hermes is installed: it answers, remembers in YantrikDB and is paired with this desktop"
[ -s "$(harness_state hermes)/model" ] \
    || say "Hermes keeps its own model: choose it with Choose model on this row, or run: hermes model"
