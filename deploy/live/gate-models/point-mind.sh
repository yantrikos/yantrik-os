#!/bin/sh
# Point the live instance's Mind (VM 561) at the gate's cloud routes (setup-models.sh). Run on
# node2 as root, from this directory. VM defaults to 561. Restarts the Mind service, not the
# desktop. Undo: the Mind's settings are kept beside themselves as yantrik-mind.env.before-cloud.
#
# The brain it ends up with, in order:
#   Ollama Cloud, deepseek-v4.1-flash  first: 8 of 9 on the desktop task battery (2026-09-21),
#                                      at about 3 s a call;
#   NanoGPT, its default model         when Ollama Cloud refuses or fails;
#   AIG through the gate               the survival fallback, and still the private lane's only
#                                      model, because a private turn never goes to a cloud.
# AIG's own model (bonsai2-27b) leads no longer: on this machine it could not fill a tool's named
# parameters, and repeated the same bare string to the desktop seven times over two runs.
#
# NVIDIA NIM is given to the Mind too (mind-env.sh) but leads nothing until YM_PRIMARY_BRAIN says
# nim:nvidia/nemotron-3-super-120b-a12b.
#
# Each provider's key, in the Mind's settings, is the instance key the AIG lane already carries:
# the gate checks it and puts the real key on.
set -eu
cd "$(dirname "$0")"
VM=${VM:-561}
. ../guest.sh

guest 'runuser -u yantrik-mind -- sh -s' < mind-env.sh
guest 'set -e
systemctl restart yantrik-mind
sleep 8
systemctl is-active yantrik-mind
journalctl -u yantrik-mind --since "-20s" --no-pager -o cat | grep -iE "brain|chain" | head -8' < /dev/null
