#!/bin/sh
# Yantrik Mind, from Settings → Harnesses: mind.sh
#
# Nothing to fetch and nothing written: the Mind's first-boot setup (yantrik-update migrate-minds,
# armed by #749) installs and starts its service under its own account, which this, running as
# the person with no root, cannot and must not do. So this checks the service is up and the Mind
# has attached, and records it as installed only then. The model goes over its memory socket's
# POST /provider, which is the model half's (#675), not this file's.
set -eu
. "$(dirname "$0")/common.sh"
HARNESS=mind

[ -x /opt/yantrik-mind/bin/yantrik-mind ] || fail "this image carries no Yantrik Mind in /opt/yantrik-mind"
if ! systemctl is-active --quiet yantrik-mind.service; then
    fail "Yantrik Mind is not running. The first boot sets it up; if it did not, run once in a terminal: sudo yantrik-update migrate-minds"
fi
say "Yantrik Mind is running under its own account"
prove mind 60
say "Yantrik Mind is installed and attached to this desktop"
