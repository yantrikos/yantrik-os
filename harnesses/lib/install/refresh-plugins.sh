#!/bin/sh
# Bring the harness plugins an install copied into this person's home up to this build's: refresh-plugins.sh [--restart]
#
# Run as the person, never as root: by `yantrik-update` after an update is applied and on every
# `yantrik-update reconcile` (reconcile_harness_plugins), and by hand from a terminal.
#
# Hermes is the only harness that loads a copy: its gateway reads ~/.hermes/plugins/, which
# hermes.sh writes once, at install, so without this every Hermes fix shipped later reached fresh
# installs only (VM 541, 14 Oct). Pi is handed its extension in /opt/yantrik/share by path
# (yantrik_pi.py, `_default_extension`); DeepSeek and OpenClaw run their scripts there; OpenClaw is
# given the image's own yantrik-os and yantrikdb-mcp by path; the Mind installs nothing in a home.
# So there is nothing of theirs to refresh, and a harness that starts copying one belongs here.
#
# Only a plugin the install wrote and that is still there is refreshed, only when this build's
# differs by content (hermes-plugins.sh, hermes_tree_hash), and by the same functions hermes.sh
# writes it with, built in ~/.hermes/.yantrik-staging and never beside them (hermes-plugins.sh,
# hermes_staging, says why). Each change is printed and kept in
# ~/.local/state/yantrik/harnesses/hermes/refreshed.
#
# Hermes's gateway is NOT restarted unless --restart is given, and yantrik-update never gives it.
# A restart kills a turn in flight, and Hermes 0.21 then hands that turn's final reply to the
# person's next turn as a "Recovered reply" (VM 541, 14 Oct; review of #808); this cannot tell
# whether a turn is in flight, and an update runs while the desktop is in use. So the running
# gateway keeps the plugin it loaded (it never re-reads the files, and the plugin imports nothing of
# its own lazily) and the new one is loaded when the gateway next starts: at the next login or
# reboot. A person running this by hand, who knows they are not mid-turn, passes --restart; then
# the gateway is restarted once, only if something changed and only if it was running.
# Nothing else under ~/.hermes is touched: config.yaml, .env, sessions, memories, other plugins.
# The staging directory is removed again once it is empty.
set -eu

restart=0
case "${1:-}" in
    "") ;;
    --restart) restart=1 ;;
    *) printf 'usage: refresh-plugins.sh [--restart]\n' >&2; exit 2 ;;
esac

# A machine with no Hermes plugin of ours: nothing to do, and nothing written, not even the
# ~/.local/bin common.sh makes.
[ -f "$HOME/.hermes/plugins/yantrik/plugin.yaml" ] || [ -d "$HOME/.hermes/plugins/yantrikdb" ] || exit 0

. "$(dirname "$0")/common.sh"
. "$(dirname "$0")/hermes-plugins.sh"

plugins="$HOME/.hermes/plugins"
shipped="$(dirname "$HARNESS_LIB")/hermes"
changed=""

# ── The desktop plugin ──
dest="$plugins/yantrik"
if [ -f "$dest/plugin.yaml" ] && [ -f "$shipped/plugin.yaml" ]; then
    was=$(hermes_tree_hash "$dest")
    now=$(hermes_tree_hash "$shipped")
    if [ "$was" != "$now" ]; then
        new=$(hermes_staging yantrik)-new || fail "could not make ~/.hermes/.yantrik-staging"
        hermes_desktop_plugin_build "$shipped" "$new" \
            && hermes_swap_dir "$new" "$dest" \
            || { rm -rf "$new"; fail "could not refresh Hermes's desktop plugin; the earlier one is in place"; }
        changed="$changed yantrik"
        say "refreshed Hermes's desktop plugin in ~/.hermes/plugins/yantrik (was $(echo "$was" | cut -c1-12), now $(echo "$now" | cut -c1-12))"
    fi
fi

# ── The YantrikDB memory plugin ──
# Built from the image's wheels beside the installed one and compared there, since what an install
# writes is what pip makes of the wheels, not a file that ships; kept only when it differs. A build
# that fails leaves the earlier plugin as it was, and the desktop plugin above still counts.
dest="$plugins/yantrikdb"
if [ -d "$dest" ]; then
    if [ ! -d "$MEMORY_WHEELS" ]; then
        say "left Hermes's YantrikDB plugin as it was: this build has no wheels at $MEMORY_WHEELS"
    elif ! hermes_python=$(python3 "$HARNESS_LIB/hermes_runtime.py" </dev/null); then
        say "left Hermes's YantrikDB plugin as it was: cannot find the Python Hermes runs on"
    elif ! new=$(hermes_staging yantrikdb)-new; then
        say "left Hermes's YantrikDB plugin as it was: could not make ~/.hermes/.yantrik-staging"
    elif ! ( hermes_yantrikdb_plugin_build "$new" "$hermes_python" ); then
        rm -rf "$new"
        say "left Hermes's YantrikDB plugin as it was: this build's would not build (the line above says why)"
    else
        was=$(hermes_tree_hash "$dest")
        now=$(hermes_tree_hash "$new")
        if [ "$was" = "$now" ]; then
            rm -rf "$new"
        else
            hermes_swap_dir "$new" "$dest" \
                || { rm -rf "$new"; fail "could not refresh Hermes's YantrikDB plugin; the earlier one is in place"; }
            changed="$changed yantrikdb"
            say "refreshed Hermes's YantrikDB plugin in ~/.hermes/plugins/yantrikdb (was $(echo "$was" | cut -c1-12), now $(echo "$now" | cut -c1-12))"
        fi
    fi
fi

rmdir "$HOME/.hermes/.yantrik-staging" 2>/dev/null || true
[ -n "$changed" ] || exit 0

state=$(harness_state hermes)
mkdir -p "$state" && printf '%s%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$changed" >> "$state/refreshed" \
    || say "could not note the refresh in $state/refreshed"

# The gateway is what loads the plugins, so a change reaches Hermes only once it starts again. Not
# restarted here unless asked (the header says why); one the person stopped is left stopped.
if [ "$restart" = 0 ]; then
    say "Hermes loads them when its gateway next starts (your next login or reboot); between turns, now: systemctl --user restart hermes-gateway"
elif systemctl --user is-active --quiet hermes-gateway 2>/dev/null; then
    systemctl --user restart hermes-gateway || fail "refreshed Hermes's plugins, but its gateway would not restart; run: systemctl --user restart hermes-gateway"
    say "restarted Hermes's gateway so it loads them"
else
    say "Hermes's gateway is not running; it loads them when it next starts"
fi
