#!/bin/sh
# Hermes's model settings, as the mind account: hermes-model.sh apply|revert
#
# Run by `yantrik-update harness model hermes` in the gateway's sandbox (HERMES_HOME is the unit's
# own). apply reads the settings on stdin. Hermes's own Python, because it has the YAML library.
set -eu
. "$(dirname "$0")/common.sh"
HARNESS=hermes
[ $# -eq 1 ] || fail "usage: hermes-model.sh apply|revert"
[ -n "${HERMES_HOME:-}" ] || fail "HERMES_HOME is not set"
hermes_python=$(python3 "$HARNESS_LIB/hermes_runtime.py" </dev/null) \
    || fail "cannot find the Python Hermes runs on"
exec "$hermes_python" "$INSTALL_LIB/hermes_model.py" "$1" "$HERMES_HOME/config.yaml"
