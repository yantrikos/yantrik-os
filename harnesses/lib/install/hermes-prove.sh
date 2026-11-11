#!/bin/sh
# Hermes's three proofs, run under the mind account once its gateway service is up: hermes-prove.sh
#
# The second half of hermes.sh when Hermes runs as the mind account (#411; hermes.sh says why it is
# in two parts). `yantrik-update harness install hermes` starts hermes-gateway.service between the
# two, because the third proof ("Can it reach the desktop?") is an attach seen on the harness socket.
# Reached through hermes.sh with YANTRIK_HARNESS_ISOLATED=prove; not a button of its own.
set -eu
. "$(dirname "$0")/common.sh"
HARNESS=hermes
prove hermes 120
say "Hermes is installed: it answers, remembers in YantrikDB and is paired with this desktop"
