#!/bin/sh
# Put Hermes in this account's home, ready to run, while the ISO is built: hermes-bake.sh
#
# Run by deploy/yantrik-os/bake-hermes.sh inside the image, as the mind account `yantrik-mind` with
# HOME=/var/lib/yantrik-mind/harness/hermes. That is where hermes.sh puts Hermes when it runs as
# that account (#411, #1076), so a first run finds `hermes` on its PATH and skips the download.
# It is built at the path it will run from, not copied there: Hermes's venv and launcher hold
# absolute paths that would not survive a move.
#
# The same pinned commit and installer checksum as the runtime install (hermes-pin.sh), so a
# mismatch stops the image build instead of shipping an unchecked Hermes. What is left for the
# runtime is the config, YantrikDB as its memory and the gateway: all local. The desktop plugin
# is enabled here as well, to prove the first run owes Hermes no dependency sync (below).
set -eu
. "$(dirname "$0")/common.sh"
. "$(dirname "$0")/hermes-launcher.sh"
. "$(dirname "$0")/hermes-pin.sh"
. "$(dirname "$0")/hermes-plugins.sh"

hermes_run_installer "$HERMES_INSTALLER_URL" "$HERMES_INSTALLER_SHA256" "$HERMES_COMMIT"
command -v hermes >/dev/null 2>&1 || fail "Hermes installed, but there is no hermes in ~/.local/bin"
heal_hermes_launcher "$HOME" || fail "Hermes's launcher still points into /tmp"

got=$(git -C "$HOME/.hermes/hermes-agent" rev-parse HEAD 2>/dev/null) || got=none
[ "$got" = "$HERMES_COMMIT" ] || fail "Hermes is at $got, not the pinned $HERMES_COMMIT"
hermes --version >/dev/null 2>&1 || fail "Hermes installed, but hermes will not run"

# Left out of the image, so it ships nothing under a restrictive licence:
#  - the ffmpeg Hermes's installer fetches is BtbN's `gpl` build (GPLv3, x264 and x265 in it). Hermes
#    runs without it (voice and audio conversion ask for it) and `hermes pm install` fetches it on
#    request;
#  - gsap, a build-time dependency of Hermes's web UI under GSAP's own "no charge" licence, which is
#    not an open-source one. The built UI does not contain it.
# And the two caches the installer leaves behind, 285 MB of uv's and 93 MB of npm's.
rm -rf "$HOME/.hermes/tools"/ffmpeg-* "$HOME/.hermes/hermes-agent/node_modules/gsap" "$HOME/.hermes/cache/uv" "$HOME/.npm"
python3 -I -c '
import json, sys
path = sys.argv[1]
facts = json.load(open(path))
facts["packages"].pop("ffmpeg", None)
json.dump(facts, open(path, "w"), indent=2, sort_keys=True)
' "$HOME/.hermes/tools/facts.json" || fail "could not take ffmpeg out of Hermes's package record"

# A new Hermes finishes its own setup on its first command, which took 102 s on VM 541 (hermes.sh).
# Done here so the person's machine does not wait for it.
timeout 600 hermes config path </dev/null >/dev/null || fail "Hermes did not finish its own setup"

# The first run puts the desktop plugin in and enables it (hermes.sh). A Hermes whose venv is out
# of date after that runs a dependency sync before its next command, and that sync first fetches
# every tool in its lockfile, ffmpeg included; with no network it stops (review of #1169, H1). So
# the same plugin is enabled here, with the network, and the image build stops unless the command
# after it fetched nothing and left nothing to sync: the proof, taken at build time, that the first
# run owes no sync and works with the network off. The first run enables it again; that is a
# no-op the same check covers.
plugin=$(dirname "$HARNESS_LIB")/hermes
[ -f "$plugin/plugin.yaml" ] || fail "no desktop plugin at $plugin"
new_desktop=$(hermes_staging yantrik)-new || fail "could not make ~/.hermes/.yantrik-staging"
hermes_desktop_plugin_build "$plugin" "$new_desktop" \
    && hermes_swap_dir "$new_desktop" "$HOME/.hermes/plugins/yantrik" \
    || fail "could not put the desktop plugin in ~/.hermes/plugins/yantrik"
timeout 60 hermes plugins enable yantrik-desktop </dev/null || fail "Hermes would not enable the desktop plugin"
timeout 600 hermes config path </dev/null >/dev/null || fail "Hermes would not run after the desktop plugin was enabled"
[ -z "$(ls -d "$HOME/.hermes/tools"/ffmpeg-* 2>/dev/null)" ] \
    || fail "enabling the desktop plugin fetched ffmpeg, so the first run would too; the image does not ship this Hermes"
hermes_python=$(python3 "$HARNESS_LIB/hermes_runtime.py" </dev/null) \
    || fail "cannot find the Python Hermes runs on: none beside hermes, and no venv on Hermes's own sys.path"
(cd "$HOME/.hermes/hermes-agent" && "$hermes_python" -c 'import pm, pathlib, sys; sys.exit(0 if pm.venv_is_current(project_root=pathlib.Path.cwd()) else 1)') </dev/null \
    || fail "Hermes still owes itself a dependency sync after the desktop plugin was enabled, so a first run with no network would stop; the image does not ship this Hermes"

# Read last, after everything Hermes runs here: every package and tool must be one the image may
# carry (hermes_licences.py fails closed on anything unknown or undeclared).
python3 -I "$INSTALL_LIB/hermes_licences.py" "$HOME/.hermes" || fail "Hermes brought in a licence the image does not ship (above)"
say "$(hermes --version 2>/dev/null | head -n 1) at $HERMES_COMMIT"
