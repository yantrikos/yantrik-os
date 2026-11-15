#!/usr/bin/env bash
# Controller tests on the node: a fake token, a stub of GitHub's org API (stub-api.py) and REAL clones
# of the template, on an isolated test bridge (vmbr-yci-test: no uplink) with a test fixed address, so
# nothing here touches the LAN. Never enables or starts the unit. Run as root, with the controller
# stopped and no CT in 9600-9699. Each test prints what happened and asserts the points that matter;
# the script ends with ALL PASS or FAIL (and exits non-zero on FAIL).
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
# The gateway of the test segment: the controller learns its MAC by ARP before it loads the filter.
ip netns add yci-gw && ip link add yci-gwh type veth peer name yci-gwn && ip link set yci-gwn netns yci-gw || { echo "no test netns"; exit 1; }
ip -n yci-gw addr add 10.77.0.1/24 dev yci-gwn && ip -n yci-gw link set yci-gwn up && ip link set yci-gwh master "$TB" && ip link set yci-gwh up
python3 "$HERE/stub-api.py" > "$T/stub.log" 2>&1 &
STUB=$!
cleanup() {
  kill "$STUB" 2> /dev/null
  ip link del yci-gwh > /dev/null 2>&1; ip link del yci-bh > /dev/null 2>&1
  ip netns del yci-gw > /dev/null 2>&1; ip netns del yci-busy > /dev/null 2>&1
  ip link del "$TB" > /dev/null 2>&1
  nft delete table bridge yantrik_ci > /dev/null 2>&1
}
trap cleanup EXIT
sleep 1
export YANTRIK_CI_API=http://127.0.0.1:8099 YANTRIK_CI_TOKEN_FILE=$T/token YANTRIK_CI_BACKOFF_MIN=5 YANTRIK_CI_POLL=5
export YANTRIK_CI_CLONE_NET=$T/clone-net YANTRIK_CI_BRIDGE=$TB NODE_BRIDGE=$TB YANTRIK_CI_GATEWAY=10.77.0.1

fails=0
ok() { echo "  PASS  $1"; }
no() { echo "  FAIL  $1"; fails=$((fails + 1)); }
has() { grep -qE -- "$2" "$1" && ok "$3" || no "$3 (not found: $2)"; }
hasnt() { grep -qE -- "$2" "$1" && no "$3 (found: $2)" || ok "$3"; }
no_ct() { if ls /etc/pve/nodes/*/lxc/96??.conf > /dev/null 2>&1; then no "a CT is left in 9600-9699"; else ok "no CT left in 9600-9699"; fi; }
pins_left() { nft list set bridge yantrik_ci clone_ports 2> /dev/null | grep -c 'veth96[0-9][0-9]i0'; }
filt() { grep -E "$1" | sed 's/^/  /'; }
# run N PATTERN CMD...: the controller (or a command) into $T/tN.out, echoing the lines that match PATTERN
run() { local n=$1 pat=$2; shift 2; "$@" > "$T/t$n.out" 2>&1; echo $? > "$T/t$n.rc"; filt "$pat" < "$T/t$n.out"; }
out() { echo "$T/t$1.out"; }

echo "--- 1: token mode 644 is refused"
chmod 644 "$T/token"
YANTRIK_CI_MAX_CYCLES=1 run 1 "refus" timeout 60 $C
chmod 600 "$T/token"
has "$(out 1)" "refusing the token file" "the 644 token is refused"

echo "--- 2: group created, fixed address set, host filter loaded, two cycles (fake config fails fast)"
echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=2 run 2 "group|removed|done:" timeout 300 $C
has "$(out 2)" "runner group yantrik-ci created" "the group is created on first use"
[ "$(grep -c 'done:' "$(out 2)")" = 2 ] && ok "two cycles ran" || no "two cycles ran"
nft list table bridge yantrik_ci > /dev/null 2>&1 && ok "the host table is loaded" || no "the host table is loaded"
[ "$(pins_left)" = 0 ] && ok "no pin left" || no "no pin left"
no_ct

echo "--- 2b: no clone-net file, an invalid one (the node's own address), or an address something answers for: no clone"
YANTRIK_CI_CLONE_NET=/nonexistent YANTRIK_CI_MAX_CYCLES=1 run 2b "REFUS|fixed address" timeout 60 $C
has "$(out 2b)" "fixed address is not set" "a missing clone-net file refuses the clone"
printf 'ip=10.77.0.2/24,gw=10.77.0.1\n' > "$T/bad-net"; chmod 644 "$T/bad-net"
YANTRIK_CI_CLONE_NET=$T/bad-net YANTRIK_CI_MAX_CYCLES=1 run 2c "REFUS|fixed address" timeout 60 $C
has "$(out 2c)" "is the gateway or the node" "the node's own address is refused"
# L3: a host that answers for the address
ip netns add yci-busy && ip link add yci-bh type veth peer name yci-bn && ip link set yci-bn netns yci-busy
ip -n yci-busy addr add 10.77.0.77/24 dev yci-bn && ip -n yci-busy link set yci-bn up && ip link set yci-bh master "$TB" && ip link set yci-bh up
printf 'ip=10.77.0.77/24,gw=10.77.0.1\n' > "$T/busy-net"; chmod 644 "$T/busy-net"
YANTRIK_CI_CLONE_NET=$T/busy-net YANTRIK_CI_MAX_CYCLES=1 run 2d "REFUS|already answers" timeout 60 $C
has "$(out 2d)" "already answers on" "an address that something answers for is refused (ARP probe)"
no_ct

echo "--- 3: a CT in the range that is not ours is left alone (N7), and the controller exits 3"
pct clone 9530 9601 --hostname foreign-9601 > /dev/null
YANTRIK_CI_MAX_CYCLES=1 run 3 "refusing|ERROR" timeout 120 $C
has "$(out 3)" "it is not named yantrik-ci-9601" "a CT that is not ours is not destroyed"
[ "$(cat "$T/t3.rc")" = 3 ] && ok "exit status 3" || no "exit status 3 (got $(cat "$T/t3.rc"))"
pct destroy 9601 --purge > /dev/null 2>&1; no_ct

echo "--- 4: job ADMITTED (ci.yml test, this repo), runs to its limit (20 s)"
echo busy > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_JOB_LIMIT=20 run 4 "took a job|admitted|REFUS|done:" timeout 200 $C
has "$(out 4)" "job admitted" "the job is admitted"
has "$(out 4)" "job exit: 124" "it is ended at its limit"
no_ct

echo "--- 5: job REFUSED: it is iso.yml's"
echo busyiso > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 5 "took a job|admitted|REFUS|done:" timeout 200 $C
has "$(out 5)" "REFUSING the job.*iso.yml" "an iso.yml job is refused"
has "$(out 5)" "job exit: 1 \(job refused\)" "and the CT is ended"
no_ct

echo "--- 6: job REFUSED: a fork's head repository"
echo busyfork > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 6 "took a job|admitted|REFUS|done:" timeout 200 $C
has "$(out 6)" "REFUSING the job.*head repository evil/yantrik-os" "a fork's job is refused"
no_ct

echo "--- 7: job REFUSED: not found on GitHub within 4 looks"
echo busynone > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 7 "took a job|admitted|REFUS|done:" timeout 200 $C
has "$(out 7)" "could not be found on GitHub in 4 looks" "a job that cannot be found is refused after 4 looks"
no_ct

echo "--- 7b: M1: 100 large runs in progress (1.4 MB of API body), our job is the last: still admitted"
echo busymany > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_JOB_LIMIT=20 run 7b "took a job|admitted|REFUS|done:" timeout 300 $C
has "$(out 7b)" "job admitted" "the job is found among 100 large runs"
no_ct

echo "--- 7c: M2: a job shorter than a poll still leaves an audit line (and a loud one if it was not allowed)"
echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_POLL=30 run 7c "audit|VIOLATION|AUDIT-MISSING|done:" timeout 200 $C
has "$(out 7c)" "audit: runner .*\(admitted\)" "a short job leaves an audit line"
echo fastiso > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_POLL=30 run 7d "audit|VIOLATION|AUDIT-MISSING|done:" timeout 200 $C
has "$(out 7d)" "VIOLATION" "a short job that was not allowed is logged as a VIOLATION"
no_ct

echo "--- 7e: N2: GitHub cannot be asked about the runner: a job found anyway is judged (iso.yml: refused)"
echo apidownjob > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 7e "took a job|admitted|REFUS|apilost|did not answer|done:" timeout 200 $C
has "$(out 7e)" "REFUSING the job" "a job found while the runner endpoint is down is judged"
echo "--- 7f: N2: GitHub does not answer at all: the CT is ended after the limit (15 s here, 5 min in service)"
echo apidown > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_APILOST=15 run 7f "not answered|did not answer|done:" timeout 200 $C
has "$(out 7f)" "has not answered about runner" "an unanswered runner is ended"
no_ct

echo "--- 8: no job: recycled quietly (idle limit 15 s)"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_IDLE_LIMIT=15 run 8 "done:|backing" timeout 200 $C
has "$(out 8)" "no job in 15s: recycled" "an idle runner is recycled"
hasnt "$(out 8)" "backing off" "with no back-off"
no_ct

echo "--- 9: memory short while idle: the runner is recycled, no job lost (idle floor 200 GB)"
YANTRIK_CI_IDLE_MIN_MEM_KB=209715200 YANTRIK_CI_MAX_CYCLES=1 run 9 "done:|backing" timeout 120 $C
has "$(out 9)" "memory short while idle: recycled" "a runner is recycled when memory is short"
no_ct

echo "--- 10: group widened to visibility all, or listing another repository, is refused (R8)"
echo wide > "$T/group"; echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 10 "REFUS" timeout 60 $C
has "$(out 10)" "visibility 'all'" "a widened group is refused"
echo many > "$T/group"
YANTRIK_CI_MAX_CYCLES=1 run 10b "REFUS" timeout 60 $C
has "$(out 10b)" "not limited to yantrikos/yantrik-os alone" "a group with another repository is refused"
rm -f "$T/group"

echo "--- 11: thin pool too full: refuse to clone (pool gate pointed at a fake size)"
YANTRIK_CI_POOL=pve/root YANTRIK_CI_MAX_CYCLES=1 run 11 "thin pool|cannot read" timeout 60 $C
has "$(out 11)" "thin pool pve/root is too full" "the pool gate refuses"
no_ct

echo "--- 11b: L3: a file that is not root's alone is refused (own-public-ips mode 666)"
printf '203.0.113.9\n' > "$T/ips"; chmod 666 "$T/ips"
YANTRIK_CI_OWN_PUBLIC=$T/ips YANTRIK_CI_MAX_CYCLES=1 run 11b "REFUS" timeout 60 $C
has "$(out 11b)" "REFUSING to use .*owner/mode '0 666'" "a world-writable input is refused"
no_ct

echo "--- 12: no host filter inputs: refuse to clone"
YANTRIK_CI_OWN_PUBLIC=/nonexistent YANTRIK_CI_MAX_CYCLES=1 run 12 "host filter|REFUS" timeout 60 $C
has "$(out 12)" "REFUSING to clone" "no clone without the filter's inputs"
no_ct

echo "--- 13: runaway output cut at the log cap (cap 1 KB)"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_RUNLOG_KB=1 YANTRIK_CI_IDLE_LIMIT=60 run 13 "done:" timeout 300 $C
n=$(ls -l /var/log/yantrik-ci/runs | tail -1 | awk '{print $5}')
[ "$n" -le 1024 ] && ok "the run log stopped at the cap ($n bytes)" || no "the run log stopped at the cap ($n bytes)"
no_ct

echo "--- 13b: M4: the host filter is removed while a job runs: the CT is ended"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_IDLE_LIMIT=300 $C > "$T/t13b.out" 2>&1 &
P=$!
for _ in $(seq 1 60); do grep -q "runner launching" "$T/t13b.out" && break; sleep 1; done
sleep 3; nft delete table bridge yantrik_ci
wait $P
filt "filter|done:" < "$T/t13b.out"
has "$T/t13b.out" "host filter was lost while CT .* ran" "a lost host filter ends the CT"
no_ct

echo "--- 14: SIGTERM mid-cycle destroys the CT"
YANTRIK_CI_MAX_CYCLES=0 $C > "$T/t14.out" 2>&1 &
P=$!
for _ in $(seq 1 60); do grep -q "runner launching" "$T/t14.out" && break; sleep 1; done
sleep 5
kill -TERM $P; wait $P
filt "done:|stopped" < "$T/t14.out"
has "$T/t14.out" "job exit: 143" "SIGTERM ends the job"
[ "$(pins_left)" = 0 ] && ok "no pin left" || no "no pin left"
no_ct

echo "--- 15: the kill path: a refused job whose runner is still 'running' is ended within 30 s, CT gone, pin gone"
echo busyiso > "$T/mode"
mkdir -p "$T/bin"; cp "$HERE/fake-pct" "$T/bin/pct"
PATH=$T/bin:$PATH FAKE_PCT=exec-sleeps YANTRIK_CI_POLL=10 YANTRIK_CI_MAX_CYCLES=1 run 15 "REFUS|done:" timeout 120 $C
t_launch=$(grep "runner launching" "$T/t15.out" | head -1 | cut -d' ' -f1)
t_done=$(grep "done:" "$T/t15.out" | head -1 | cut -d' ' -f1)
has "$(out 15)" "REFUSING the job" "the job is refused"
has "$(out 15)" "done: CT" "the CT is ended"
if [ -n "$t_launch" ] && [ -n "$t_done" ]; then
  el=$(( $(date -ud "$t_done" +%s) - $(date -ud "$t_launch" +%s) ))
  [ "$el" -le 30 ] && ok "refused and done within 30 s ($el s)" || no "refused and done within 30 s ($el s)"
else no "no launch or done line"; fi
[ "$(pins_left)" = 0 ] && ok "the pin is gone (pins left 0)" || no "the pin is gone"
no_ct

echo "--- 16: N1, L1, L2: a CT that survives its job: the controller exits 3, the table stays loaded, the survivor is unpinned"
echo slow > "$T/mode"
PATH=$T/bin:$PATH FAKE_PCT=no-destroy YANTRIK_CI_IDLE_LIMIT=10 YANTRIK_CI_MAX_CYCLES=3 run 16 "ERROR|stopped|cycle:" timeout 150 $C
rc=$(cat "$T/t16.rc")
[ "$rc" = 3 ] && ok "exit status 3 (the unit does not restart it)" || no "exit status 3 (got $rc)"
has "$(out 16)" "ERROR: CT 9600 could not be destroyed" "the survivor is reported"
[ "$(grep -c 'cycle:' "$(out 16)")" = 1 ] && ok "no second cycle started" || no "no second cycle started"
nft list table bridge yantrik_ci > /dev/null 2>&1 && ok "the host table is still loaded" || no "the host table is still loaded"
[ "$(pins_left)" = 0 ] && ok "pins left in the table: 0 (the survivor's port is dropped, not pinned)" || no "pins left in the table: $(pins_left)"
echo "  the survivor is destroyed by hand now:"
for conf in /etc/pve/nodes/*/lxc/96??.conf; do [ -e "$conf" ] || continue; id=$(basename "$conf" .conf); pct stop "$id" > /dev/null 2>&1; pct destroy "$id" --purge > /dev/null 2>&1; done
no_ct

kill "$STUB"
echo "--- secret scan of the controller log and run logs (matches):"
TOK=$(cat "$T/token"); J1=$(sed -n 1p "$T/jit-marker"); J2=$(sed -n 2p "$T/jit-marker")
for pat in "$TOK" "$J1" "$J2" "STUBSECRETMARK"; do
  n=$(grep -rF -c -- "$pat" /var/log/yantrik-ci "$T"/t*.out 2> /dev/null | awk -F: '{s+=$NF} END{print s+0}')
  [ "$n" = 0 ] && ok "pattern (${#pat} chars): 0 matches" || no "pattern (${#pat} chars): $n matches"
done
st="$(systemctl is-enabled yantrik-ci-controller 2>&1) / $(systemctl is-active yantrik-ci-controller 2>&1)"
[ "$st" = "disabled / inactive" ] && ok "the unit is $st" || no "the unit is $st"
if [ "$fails" -eq 0 ]; then echo "== controller tests: ALL PASS"; else echo "== controller tests: FAIL ($fails)"; fi
[ "$fails" -eq 0 ]
