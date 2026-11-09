#!/bin/sh
# Hold Hermes's toolset allowlist after the file may have changed under it: hermes-guard.sh start|after
#
# Hermes runs as the mind account and can rewrite its own config.yaml (its `hermes model` and
# `hermes update` write the whole file, and its terminal toolset on another platform can edit it),
# so platform_toolsets / mcp_servers / skills.inline_shell are re-asserted (hermes_config.py reassert)
# rather than trusted. Run as the account, in the gateway's sandbox:
#   start  the unit's ExecStartPre=: the file is put right before Hermes reads it; exit 1 (no start)
#          only if it cannot be put right.
#   after  by `yantrik-update` after `harness run hermes model` and "Use Yantrik models": exit 0 the
#          file held, 3 it had been widened and is put right (the running gateway may have read it:
#          restart it), 1 it could not be put right.
set -eu
. "$(dirname "$0")/common.sh"
[ $# -eq 1 ] && { [ "$1" = start ] || [ "$1" = after ]; } || fail "usage: hermes-guard.sh start|after"
[ -n "${HERMES_HOME:-}" ] || fail "HERMES_HOME is not set"
hermes_python=$(python3 "$HARNESS_LIB/hermes_runtime.py" </dev/null) \
    || fail "cannot find the Python Hermes runs on"
rc=0
"$hermes_python" "$INSTALL_LIB/hermes_config.py" reassert "$HERMES_HOME/config.yaml" || rc=$?
if [ "$rc" = 3 ] && [ "$1" = start ]; then rc=0; fi
exit "$rc"
