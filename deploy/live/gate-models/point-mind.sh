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
# The gate is a private address, and the OS's egress proxy refuses those to a mind unless a rule
# that says `lan` allows it. Root adds that rule through the proxy's own control socket, as the
# machine's owner would from Settings: a person's rule, kept in the policy and never replaced by
# the updater's seeding. Without it every model call failed with "CONNECT proxy failed: 403" once
# 561 took the egress proxy (15 Oct 2026). Not by /etc/yantrik/mind-person.env, the updater's
# seed source: the Mind reads that file too, took YM_LOCAL_OLLAMA_URL in it for a keyless private
# lane, and failed every private turn closed against the gate's 401.
# The address comes from here, never from the Mind's settings: a mind must not choose its own rule.
GATE_HOST=${GATE_HOST:-10.99.0.1}
GATE_PORT=${GATE_PORT:-8443}
# They go into a root shell in the VM, so they are held to an IPv4 address and a port.
printf '%s' "$GATE_HOST" | grep -Eqx '([0-9]{1,3}\.){3}[0-9]{1,3}' || { echo "GATE_HOST must be an IPv4 address" >&2; exit 1; }
printf '%s' "$GATE_PORT" | grep -Eqx '[1-9][0-9]{0,4}' || { echo "GATE_PORT must be a port number" >&2; exit 1; }
guest "python3 -I -c '
import json, socket
rule = {\"host\": \"$GATE_HOST\", \"ports\": [$GATE_PORT], \"http\": True, \"lan\": True,
        \"why\": \"the live gate, the only way this instance reaches its models\"}
s = socket.socket(socket.AF_UNIX)
s.connect(\"/run/yantrik-egress/control\")
s.sendall((json.dumps({\"op\": \"allow\", \"rule\": rule}) + chr(10)).encode())
s.shutdown(socket.SHUT_WR)
answer = json.loads(s.recv(4096).decode())
print(\"egress: the gate allowed\" if answer.get(\"ok\") else \"egress refused the rule: %s\" % answer.get(\"error\"))
raise SystemExit(0 if answer.get(\"ok\") else 1)
'" < /dev/null
guest 'set -e
systemctl restart yantrik-mind
sleep 8
systemctl is-active yantrik-mind
journalctl -u yantrik-mind --since "-20s" --no-pager -o cat | grep -iE "brain|chain" | head -8' < /dev/null
