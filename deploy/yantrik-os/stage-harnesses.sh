#!/bin/sh
# Stage the harnesses into a share/harnesses directory: stage-harnesses.sh PROJECT_ROOT DEST [UNIT_DIR]
#
# The one place that decides what of harnesses/ ships, used by the ISO (build-debian-iso.sh) and
# by the release bundle every update installs (build-release.sh). There were two answers before:
# the ISO staged them and the bundle did not, and the updater installs share/ with --delete — so
# the first update on any installed machine removed /opt/yantrik/share/harnesses, and with it the
# manifests Settings → Harnesses reads, the installers its buttons run, and the scripts the
# harness units start. A machine was left with units pointing at files that were gone.
#
# Source only, nothing enabled, nothing configured: each needs an endpoint, a model and a key
# only the person has. `lib` is what they share (attach, poll, heartbeat, the MCP client, and
# the installers in lib/install); Hermes ships only its plugin (*.py, plugin.yaml, harness.yaml)
# and the README its row links to; the rest are copied whole less bytecode.
#
# UNIT_DIR, when given, also gets each harness's unit — where systemd looks (/etc/systemd/user on
# the image). Staging a unit is not enabling it.
set -eu
[ $# -ge 2 ] || { echo "usage: stage-harnesses.sh PROJECT_ROOT DEST [UNIT_DIR]" >&2; exit 2; }
src="$1/harnesses" dest="$2" units="${3:-}"
[ -d "$src" ] || { echo "no harnesses/ under $1" >&2; exit 1; }

mkdir -p "$dest"
for harness in lib deepseek pi openclaw; do
    [ -d "$src/$harness" ] || continue
    mkdir -p "$dest/$harness"
    cp -r "$src/$harness/." "$dest/$harness/"
done
mkdir -p "$dest/hermes"
cp "$src/hermes/"*.py "$src/hermes/plugin.yaml" "$src/hermes/harness.yaml" "$src/hermes/README.md" \
    "$dest/hermes/"
# Bytecode from someone's checkout is not part of what ships.
find "$dest" -name __pycache__ -type d -prune -exec rm -rf {} +

# The manifests are what Settings reads, and the installers are what its buttons run: a staging
# that lost either is a page back to "not installed" with nothing to press. Asserted, not hoped.
for harness in hermes deepseek pi openclaw; do
    [ -f "$dest/$harness/harness.yaml" ] || { echo "$harness has no harness.yaml in $dest" >&2; exit 1; }
done
for script in common.sh npm.sh hermes.sh; do
    [ -f "$dest/lib/install/$script" ] || { echo "lib/install/$script did not stage" >&2; exit 1; }
done

if [ -n "$units" ]; then
    mkdir -p "$units"
    for harness in deepseek pi openclaw; do
        unit="$src/$harness/yantrik-$harness.service"
        [ -f "$unit" ] || continue
        install -m 644 "$unit" "$units/"
    done
fi
echo "harnesses staged in $dest${units:+, units in $units}"
