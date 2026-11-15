#!/usr/bin/env bash
# Controller tests on the node: a fake token, a stub of GitHub's org API (stub-api.py) and REAL clones
# of the template, on an isolated test bridge (vmbr-yci-test: no uplink) with a test fixed address, so
# nothing here touches the LAN. Never enables or starts the unit. Run as root, with the controller
# stopped and no CT in 9600-9699. Prints what happened; read it, there is no pass/fail summary.
#
# The stub sees only the token file's fake token. The secret scan at the end looks for it, the fake JIT
# configs and a marker in the controller's logs.

# shellcheck disable=SC2015,SC2012
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=/root/.yantrik-ci-test
TB=vmbr-yci-test
export STUB_DIR=$T
C=/usr/local/sbin/yantrik-ci-controller
rm -rf "$T"; mkdir -p "$T"; chmod 700 "$T"
printf 'ghp_FAKETOKEN_stub_only_0123456789abcdef' > "$T/token"; chmod 600 "$T/token"
printf 'ip=10.77.0.50/24,gw=10.77.0.1\n' > "$T/clone-net"; chmod 644 "$T/clone-net"
rm -rf /var/log/yantrik-ci

ip link add "$TB" type bridge && ip addr add 10.77.0.2/24 dev "$TB" && ip link set "$TB" up || { echo "no test bridge"; exit 1; }
python3 "$HERE/stub-api.py" > "$T/stub.log" 2>&1 &
STUB=$!
cleanup() {
  kill "$STUB" 2> /dev/null
  ip link del "$TB" > /dev/null 2>&1
  nft delete table bridge yantrik_ci > /dev/null 2>&1
}
trap cleanup EXIT
sleep 1
export YANTRIK_CI_API=http://127.0.0.1:8099 YANTRIK_CI_TOKEN_FILE=$T/token YANTRIK_CI_BACKOFF_MIN=5 YANTRIK_CI_POLL=5
export YANTRIK_CI_CLONE_NET=$T/clone-net YANTRIK_CI_BRIDGE=$TB NODE_BRIDGE=$TB YANTRIK_CI_GATEWAY=10.77.0.1
left() { ls /etc/pve/nodes/*/lxc/96??.conf 2> /dev/null || echo "  (no CT in 9600-9699)"; }
filt() { grep -E "$1" | sed 's/^/  /'; }
pins() { echo "  pins left in the table: $(nft list set bridge yantrik_ci clone_ports 2> /dev/null | grep -c 'veth96[0-9][0-9]i0')"; }

echo "--- 1: token mode 644 is refused"
chmod 644 "$T/token"
YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "refus"
chmod 600 "$T/token"

echo "--- 2: group created, fixed address set, host filter loaded, two cycles (fake config fails fast)"
echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=2 timeout 300 $C 2>&1 | filt "group|removed|done:"
nft list table bridge yantrik_ci > /dev/null 2>&1 && echo "  host table bridge yantrik_ci is loaded"
pins; left

echo "--- 2b: no clone-net file, or an invalid one (the node's own address): no clone"
YANTRIK_CI_CLONE_NET=/nonexistent YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "REFUS|fixed address"
printf 'ip=10.77.0.2/24,gw=10.77.0.1\n' > "$T/bad-net"; chmod 644 "$T/bad-net"
YANTRIK_CI_CLONE_NET=$T/bad-net YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "REFUS|fixed address"
left

echo "--- 3: a CT in the range that is not ours is left alone (N7)"
pct clone 9530 9601 --hostname foreign-9601 > /dev/null
YANTRIK_CI_MAX_CYCLES=1 timeout 120 $C 2>&1 | filt "refusing|ERROR"
pct destroy 9601 --purge > /dev/null 2>&1; left

echo "--- 4: job ADMITTED (ci.yml test, this repo), runs to its limit (20 s)"
echo busy > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_JOB_LIMIT=20 timeout 200 $C 2>&1 | filt "took a job|admitted|REFUS|done:"
left

echo "--- 5: job REFUSED: it is iso.yml's"
echo busyiso > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 timeout 200 $C 2>&1 | filt "took a job|admitted|REFUS|done:"
left

echo "--- 6: job REFUSED: a fork's head repository"
echo busyfork > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 timeout 200 $C 2>&1 | filt "took a job|admitted|REFUS|done:"
left

echo "--- 7: job REFUSED: not found on GitHub within 4 looks"
echo busynone > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 timeout 200 $C 2>&1 | filt "took a job|admitted|REFUS|done:"
left

echo "--- 7b: M1: 100 large runs in progress (1.4 MB of API body), our job is the last: still admitted"
echo busymany > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_JOB_LIMIT=20 timeout 300 $C 2>&1 | filt "took a job|admitted|REFUS|done:"
left

echo "--- 7c: M2: a job shorter than a poll still leaves an audit line (and a loud one if it was not allowed)"
echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_POLL=30 timeout 200 $C 2>&1 | filt "audit|VIOLATION|AUDIT-MISSING|done:"
echo fastiso > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_POLL=30 timeout 200 $C 2>&1 | filt "audit|VIOLATION|AUDIT-MISSING|done:"
left

echo "--- 7d: N2: GitHub cannot be asked about the runner: a job found anyway is judged (iso.yml: refused)"
echo apidownjob > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 timeout 200 $C 2>&1 | filt "took a job|admitted|REFUS|apilost|did not answer|done:"
echo "--- 7e: N2: GitHub does not answer at all: the CT is ended after the limit (15 s here, 5 min in service)"
echo apidown > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_APILOST=15 timeout 200 $C 2>&1 | filt "not answered|did not answer|done:"
left

echo "--- 8: no job: recycled quietly (idle limit 15 s)"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_IDLE_LIMIT=15 timeout 200 $C 2>&1 | filt "done:|backing"
left

echo "--- 9: memory short while idle: the runner is recycled, no job lost (idle floor 200 GB)"
YANTRIK_CI_IDLE_MIN_MEM_KB=209715200 YANTRIK_CI_MAX_CYCLES=1 timeout 120 $C 2>&1 | filt "done:|backing"
left

echo "--- 10: group widened to visibility all is refused (R8)"
echo wide > "$T/group"; echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "REFUS"
echo many > "$T/group"
YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "REFUS"
rm -f "$T/group"

echo "--- 11: thin pool too full: refuse to clone (pool gate pointed at a fake size)"
YANTRIK_CI_POOL=pve/root YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "thin pool|cannot read"
left

echo "--- 11b: L3: a file that is not root's alone is refused (own-public-ips mode 666)"
printf '203.0.113.9\n' > "$T/ips"; chmod 666 "$T/ips"
YANTRIK_CI_OWN_PUBLIC=$T/ips YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "REFUS"
left

echo "--- 12: no host filter inputs: refuse to clone"
YANTRIK_CI_OWN_PUBLIC=/nonexistent YANTRIK_CI_MAX_CYCLES=1 timeout 60 $C 2>&1 | filt "host filter|REFUS"
left

echo "--- 13: runaway output cut at the log cap (cap 1 KB)"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_RUNLOG_KB=1 YANTRIK_CI_IDLE_LIMIT=60 timeout 200 $C 2>&1 | filt "done:"
ls -l /var/log/yantrik-ci/runs | tail -1 | awk '{print "  newest run log bytes:", $5}'
left

echo "--- 13b: M4: the host filter is removed while a job runs: the CT is ended"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_IDLE_LIMIT=300 $C > "$T/c13b.out" 2>&1 &
P=$!
for _ in $(seq 1 60); do grep -q "runner launching" "$T/c13b.out" && break; sleep 1; done
sleep 3; nft delete table bridge yantrik_ci
wait $P
filt "filter|done:" < "$T/c13b.out"
left

echo "--- 14: SIGTERM mid-cycle destroys the CT"
YANTRIK_CI_MAX_CYCLES=0 $C > "$T/c14.out" 2>&1 &
P=$!
for _ in $(seq 1 60); do grep -q "runner launching" "$T/c14.out" && break; sleep 1; done
sleep 5
kill -TERM $P; wait $P
filt "done:|stopped" < "$T/c14.out"
pins; left

echo "--- 15: the kill path: a refused job whose runner is still 'running' is ended within 30 s, CT gone, pin gone"
echo busyiso > "$T/mode"
mkdir -p "$T/bin"; cp "$HERE/fake-pct" "$T/bin/pct"
PATH=$T/bin:$PATH FAKE_PCT=exec-sleeps YANTRIK_CI_POLL=10 YANTRIK_CI_MAX_CYCLES=1 timeout 120 $C > "$T/c15.out" 2>&1
t_launch=$(grep "runner launching" "$T/c15.out" | head -1 | cut -d' ' -f1)
t_done=$(grep "done:" "$T/c15.out" | head -1 | cut -d' ' -f1)
filt "REFUS|done:" < "$T/c15.out"
echo "  from 'runner launching' $t_launch to 'done' $t_done: $(( $(date -ud "$t_done" +%s) - $(date -ud "$t_launch" +%s) )) s"
pins; left

echo "--- 16: N1: a CT that survives its job stops the controller (exit non-zero) and leaves the table loaded"
echo slow > "$T/mode"
PATH=$T/bin:$PATH FAKE_PCT=no-destroy YANTRIK_CI_IDLE_LIMIT=10 YANTRIK_CI_MAX_CYCLES=3 timeout 120 $C > "$T/c16.out" 2>&1
echo "  controller exit status: $?"
filt "ERROR|stopped|cycle:" < "$T/c16.out"
nft list table bridge yantrik_ci > /dev/null 2>&1 && echo "  host table still loaded"
pins
echo "  the survivor is destroyed by hand now:"
for conf in /etc/pve/nodes/*/lxc/96??.conf; do [ -e "$conf" ] || continue; id=$(basename "$conf" .conf); pct stop "$id" > /dev/null 2>&1; pct destroy "$id" --purge > /dev/null 2>&1; done
left

kill "$STUB"
echo "--- stub saw (counts):"; sed 's/^/  /' "$T/stub.log" | sort | uniq -c | sort -rn | grep -E "jit name|create group" | head -3
echo "--- secret scan of the controller log and run logs (matches):"
TOK=$(cat "$T/token"); J1=$(sed -n 1p "$T/jit-marker"); J2=$(sed -n 2p "$T/jit-marker")
for pat in "$TOK" "$J1" "$J2" "STUBSECRETMARK"; do
  n=$(grep -rF -c -- "$pat" /var/log/yantrik-ci "$T"/c1*.out 2> /dev/null | awk -F: '{s+=$NF} END{print s+0}')
  echo "  pattern (${#pat} chars): $n"
done
echo "unit: $(systemctl is-enabled yantrik-ci-controller 2>&1) / $(systemctl is-active yantrik-ci-controller 2>&1)"
