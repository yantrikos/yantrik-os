#!/usr/bin/env bash
# Controller tests on the node: a fake token, a stub of GitHub's org API (stub-api.py) and REAL clones of
# the template VM on the real bridge yci0 (no physical port: nothing here reaches the LAN, and the
# runner inside is given a fake config that cannot reach anything). No GitHub token, no registration, no
# real job. Never enables or starts the unit. Run as root, with the controller stopped and no VM in
# 9541-9549 (and the network of yantrik-ci-net down: the controller makes and removes it itself). Each
# test prints what happened and asserts the points that matter; the script ends with ALL PASS or FAIL
# (and exits non-zero on FAIL).
#
# The stub sees only the token file's fake token. The secret scan at the end looks for it, the fake JIT
# configs and a marker in the controller's logs, on the node's disks and on the template's disk.

# shellcheck disable=SC2015,SC2012,SC2016
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=/root/.yantrik-ci-test
export STUB_DIR=$T
C=/usr/local/sbin/yantrik-ci-controller
[ "$(id -u)" -eq 0 ] || { echo "run as root" >&2; exit 1; }
systemctl is-active --quiet yantrik-ci-controller && { echo "the controller is running" >&2; exit 1; }
ip link show yci0 > /dev/null 2>&1 && { echo "yci0 is up already: run yantrik-ci-net down first" >&2; exit 1; }
rm -rf "$T"
mkdir -p "$T/log"
chmod 700 "$T"
printf 'ghp_FAKETOKEN_stub_only_0123456789abcdef' > "$T/token"
chmod 600 "$T/token"
python3 "$HERE/stub-api.py" > "$T/stub.log" 2>&1 &
STUB=$!
cleanup() {
  kill "$STUB" 2> /dev/null
  for conf in /etc/pve/nodes/*/qemu-server/954[1-9].conf; do
    [ -e "$conf" ] || continue
    id=$(basename "$conf" .conf)
    qm config "$id" 2> /dev/null | grep -q "^name: \(yantrik-ci\|foreign\)-" && { qm stop "$id" > /dev/null 2>&1; qm destroy "$id" --purge 1 > /dev/null 2>&1; }
  done
  yantrik-ci-net down > /dev/null 2>&1
}
trap cleanup EXIT
trap 'exit 130' INT TERM
sleep 1
export YANTRIK_CI_API=http://127.0.0.1:8099 YANTRIK_CI_TOKEN_FILE=$T/token YANTRIK_CI_BACKOFF_MIN=5 YANTRIK_CI_POLL=5
export YANTRIK_CI_LOG_DIR=$T/log
export YANTRIK_CI_MIN_MEM_KB=8388608 # the 40 GB gate is the node's business, not under test here (test 9 has its own floor)

fails=0
ok() { echo "  PASS  $1"; }
no() { echo "  FAIL  $1"; fails=$((fails + 1)); }
has() { grep -qE -- "$2" "$1" && ok "$3" || no "$3 (not found: $2)"; }
hasnt() { grep -qE -- "$2" "$1" && no "$3 (found: $2)" || ok "$3"; }
no_vm() { if ls /etc/pve/nodes/*/qemu-server/954[1-9].conf > /dev/null 2>&1; then no "a VM is left in 9541-9549"; else ok "no VM left in 9541-9549"; fi; }
pins_left() { { nft list set inet yantrik_ci clone_pin; nft list set netdev yantrik_ci_l2 clone_pin; } 2> /dev/null | grep -c 'tap954[0-9]i0'; }
net_gone() { if ip link show yci0 > /dev/null 2>&1 || nft list table inet yantrik_ci > /dev/null 2>&1 || nft list table netdev yantrik_ci_l2 > /dev/null 2>&1; then no "yci0 or a table is left"; else ok "yci0 and the table are removed at stop"; fi; }
filt() { grep -E "$1" | sed 's/^/  /'; }
# run N PATTERN CMD...: the controller (or a command) into $T/tN.out, echoing the lines that match PATTERN
run() { local n=$1 pat=$2; shift 2; "$@" > "$T/t$n.out" 2>&1; echo $? > "$T/t$n.rc"; filt "$pat" < "$T/t$n.out"; }
out() { echo "$T/t$1.out"; }

echo "--- 1: token mode 644 is refused"
chmod 644 "$T/token"
YANTRIK_CI_MAX_CYCLES=1 run 1 "refus" timeout 60 $C
chmod 600 "$T/token"
has "$(out 1)" "refusing the token file" "the 644 token is refused"

echo "--- 2: group created, network up, two cycles (a fake config fails fast and the guest powers off)"
echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=2 run 2 "group|removed|done:|cloned" timeout 600 $C
has "$(out 2)" "runner group yantrik-ci created" "the group is created on first use"
[ "$(grep -c 'done:' "$(out 2)")" = 2 ] && ok "two cycles ran" || no "two cycles ran"
has "$(out 2)" "cloned \(linked clone\)" "the clones are linked clones"
has "$(out 2)" "agent's device is gone from QEMU's view" "every job logs that QEMU itself shows the agent's device gone (N1)"
[ "$(pins_left)" = 0 ] && ok "no pin left" || no "no pin left"
no_vm
net_gone

echo "--- 3: a VM in the range that is not ours is left alone (N7), and the controller exits 3"
qm clone 9540 9543 --name foreign-9543 > /dev/null 2>&1
YANTRIK_CI_MAX_CYCLES=1 run 3 "refusing|ERROR" timeout 120 $C
has "$(out 3)" "it is not named yantrik-ci-9543" "a VM that is not ours is not destroyed"
[ "$(cat "$T/t3.rc")" = 3 ] && ok "exit status 3" || no "exit status 3 (got $(cat "$T/t3.rc"))"
qm destroy 9543 --purge 1 > /dev/null 2>&1
no_vm

echo "--- 4: job ADMITTED (ci.yml test, this repo), runs to its limit (20 s)"
echo busy > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_JOB_LIMIT=20 run 4 "took a job|admitted|REFUS|done:" timeout 400 $C
has "$(out 4)" "job admitted" "the job is admitted"
has "$(out 4)" "job exit: 124" "it is ended at its limit"
no_vm

echo "--- 5: job REFUSED: it is iso.yml's"
echo busyiso > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 5 "took a job|admitted|REFUS|done:" timeout 400 $C
has "$(out 5)" "REFUSING the job.*iso.yml" "an iso.yml job is refused"
has "$(out 5)" "job exit: 1 \(job refused\)" "and the VM is ended"
no_vm

echo "--- 6: job REFUSED: a fork's head repository"
echo busyfork > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 6 "took a job|admitted|REFUS|done:" timeout 400 $C
has "$(out 6)" "REFUSING the job.*head repository evil/yantrik-os" "a fork's job is refused"
no_vm

echo "--- 7: job REFUSED: not found on GitHub within 4 looks"
echo busynone > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 7 "took a job|admitted|REFUS|done:" timeout 400 $C
has "$(out 7)" "could not be found on GitHub in 4 looks" "a job that cannot be found is refused after 4 looks"
no_vm

echo "--- 7b: M1: 100 large runs in progress (1.4 MB of API body), our job is the last: still admitted"
echo busymany > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_JOB_LIMIT=20 run 7b "took a job|admitted|REFUS|done:" timeout 400 $C
has "$(out 7b)" "job admitted" "the job is found among 100 large runs"
no_vm

echo "--- 7c: M2: a job shorter than a poll still leaves an audit line (and a loud one if it was not allowed)"
echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_POLL=60 run 7c "audit|VIOLATION|AUDIT-MISSING|done:" timeout 400 $C
has "$(out 7c)" "audit: runner .*\(admitted\)" "a short job leaves an audit line"
echo fastiso > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_POLL=60 run 7d "audit|VIOLATION|AUDIT-MISSING|done:" timeout 400 $C
has "$(out 7d)" "VIOLATION" "a short job that was not allowed is logged as a VIOLATION"
no_vm

echo "--- 7e: N2: GitHub cannot be asked about the runner: a job found anyway is judged (iso.yml: refused)"
echo apidownjob > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 7e "took a job|admitted|REFUS|apilost|did not answer|done:" timeout 400 $C
has "$(out 7e)" "REFUSING the job" "a job found while the runner endpoint is down is judged"
echo "--- 7f: N2: GitHub does not answer at all: the VM is ended after the limit (15 s here, 5 min in service)"
echo apidown > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_APILOST=15 run 7f "not answered|did not answer|done:" timeout 400 $C
has "$(out 7f)" "has not answered about runner" "an unanswered runner is ended"
no_vm

echo "--- 8: no job: recycled quietly (idle limit 15 s)"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_IDLE_LIMIT=15 run 8 "done:|backing" timeout 400 $C
has "$(out 8)" "no job in 15s: recycled" "an idle runner is recycled"
hasnt "$(out 8)" "backing off" "with no back-off"
no_vm

echo "--- 9: memory short while idle: the runner is recycled, no job lost (idle floor 200 GB)"
YANTRIK_CI_IDLE_MIN_MEM_KB=209715200 YANTRIK_CI_MAX_CYCLES=1 run 9 "done:|backing" timeout 400 $C
has "$(out 9)" "memory short while idle: recycled" "a runner is recycled when memory is short"
no_vm

echo "--- 10: group widened to visibility all, or listing another repository, is refused (R8)"
echo wide > "$T/group"
echo fast > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 run 10 "REFUS" timeout 60 $C
has "$(out 10)" "visibility 'all'" "a widened group is refused"
echo many > "$T/group"
YANTRIK_CI_MAX_CYCLES=1 run 10b "REFUS" timeout 60 $C
has "$(out 10b)" "not limited to yantrikos/yantrik-os alone" "a group with another repository is refused"
rm -f "$T/group"

echo "--- 11: thin pool too full: refuse to clone (pool gate pointed at a fake size)"
YANTRIK_CI_POOL=pve/root YANTRIK_CI_MAX_CYCLES=1 run 11 "thin pool|cannot read" timeout 60 $C
has "$(out 11)" "thin pool pve/root is too full" "the pool gate refuses"
no_vm

echo "--- 11b: L3: a file that is not root's alone is refused (own-public-ips mode 666)"
printf '203.0.113.9\n' > "$T/ips"
chmod 666 "$T/ips"
YANTRIK_CI_OWN_PUBLIC=$T/ips YANTRIK_CI_MAX_CYCLES=1 run 11b "REFUS" timeout 60 $C
has "$(out 11b)" "REFUSING to use .*owner/mode '0 666'" "a world-writable input is refused"
no_vm

echo "--- 12: no network inputs: refuse to clone"
YANTRIK_CI_OWN_PUBLIC=/nonexistent YANTRIK_CI_MAX_CYCLES=1 run 12 "net:|REFUS" timeout 60 $C
has "$(out 12)" "REFUSING to clone" "no clone without the filter's inputs"
no_vm

echo "--- 13: M4: the table is removed while a job runs: the VM is ended"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_IDLE_LIMIT=300 $C > "$T/t13.out" 2>&1 &
P=$!
for _ in $(seq 1 120); do grep -q "runner launching" "$T/t13.out" && break; sleep 1; done
sleep 3
nft delete table inet yantrik_ci
wait $P
filt "filter|done:" < "$T/t13.out"
has "$T/t13.out" "network filter was lost while VM .* ran" "a lost table ends the VM"
no_vm

echo "--- 14: SIGTERM mid-cycle destroys the VM, and the network goes"
YANTRIK_CI_MAX_CYCLES=0 $C > "$T/t14.out" 2>&1 &
P=$!
for _ in $(seq 1 120); do grep -q "runner launching" "$T/t14.out" && break; sleep 1; done
sleep 5
kill -TERM $P
wait $P
filt "done:|stopped" < "$T/t14.out"
has "$T/t14.out" "job exit: 143" "SIGTERM ends the job"
[ "$(pins_left)" = 0 ] && ok "no pin left" || no "no pin left"
no_vm
net_gone

echo "--- 15: the kill path: a refused job whose runner is still 'running' is ended within 30 s, VM gone, pin gone"
echo busyiso > "$T/mode"
YANTRIK_CI_POLL=10 YANTRIK_CI_MAX_CYCLES=1 run 15 "REFUS|done:" timeout 400 $C
t_launch=$(grep "runner launching" "$T/t15.out" | head -1 | cut -d' ' -f1)
t_done=$(grep "done:" "$T/t15.out" | head -1 | cut -d' ' -f1)
has "$(out 15)" "REFUSING the job" "the job is refused"
has "$(out 15)" "done: VM" "the VM is ended"
if [ -n "$t_launch" ] && [ -n "$t_done" ]; then
  el=$(($(date -ud "$t_done" +%s) - $(date -ud "$t_launch" +%s)))
  [ "$el" -le 30 ] && ok "refused and done within 30 s ($el s)" || no "refused and done within 30 s ($el s)"
else no "no launch or done line"; fi
[ "$(pins_left)" = 0 ] && ok "the pin is gone (pins left 0)" || no "the pin is gone"
no_vm

echo "--- 16: N1, L1, L2: a VM that survives its job: the controller exits 3, the table stays loaded, the survivor is unpinned"
mkdir -p "$T/bin"
cp "$HERE/fake-qm" "$T/bin/qm"
echo slow > "$T/mode"
PATH=$T/bin:$PATH FAKE_QM=no-destroy YANTRIK_CI_IDLE_LIMIT=10 YANTRIK_CI_MAX_CYCLES=3 run 16 "ERROR|stopped|cycle:" timeout 400 $C
rc=$(cat "$T/t16.rc")
[ "$rc" = 3 ] && ok "exit status 3 (the unit does not restart it)" || no "exit status 3 (got $rc)"
has "$(out 16)" "ERROR: VM 9541 could not be destroyed" "the survivor is reported"
[ "$(grep -c 'cycle:' "$(out 16)")" = 1 ] && ok "no second cycle started" || no "no second cycle started"
nft list table inet yantrik_ci > /dev/null 2>&1 && ok "the table is still loaded" || no "the table is still loaded"
ip link show yci0 > /dev/null 2>&1 && ok "the bridge is still there" || no "the bridge is still there"
[ "$(pins_left)" = 0 ] && ok "pins left in the table: 0 (the survivor's port is dropped, not pinned)" || no "pins left in the table: $(pins_left)"
echo "  the survivor is destroyed by hand now:"
for conf in /etc/pve/nodes/*/qemu-server/954[1-9].conf; do [ -e "$conf" ] || continue; id=$(basename "$conf" .conf); qm stop "$id" > /dev/null 2>&1; qm destroy "$id" --purge 1 > /dev/null 2>&1; done
no_vm
yantrik-ci-net down > /dev/null 2>&1
net_gone

echo "--- 16b: L2: a VM left from before cannot be destroyed at start-up: its pin is dropped, the controller exits 3"
yantrik-ci-net up > /dev/null
qm clone 9540 9542 --name yantrik-ci-9542 > /dev/null 2>&1
yantrik-ci-net pin 9542 > /dev/null
[ "$(pins_left)" -gt 0 ] && ok "the leftover VM 9542 is pinned (pins in both tables: $(pins_left))" || no "the leftover VM is pinned"
PATH=$T/bin:$PATH FAKE_QM=no-destroy run 16b "leftover|ERROR|survived" timeout 120 $C
[ "$(cat "$T/t16b.rc")" = 3 ] && ok "exit status 3" || no "exit status 3 (got $(cat "$T/t16b.rc"))"
has "$(out 16b)" "could not destroy leftover VM 9542: unpinning it" "the survivor is unpinned at start-up"
[ "$(pins_left)" = 0 ] && ok "pins left in either table: 0" || no "pins left: $(pins_left)"
qm destroy 9542 --purge 1 > /dev/null 2>&1
yantrik-ci-net down > /dev/null 2>&1
no_vm
net_gone

echo "--- 16c: net_lock fails: the VM is destroyed before the config is handed over"
cp "$HERE/fake-bridge" "$T/bin/bridge"
echo slow > "$T/mode"
PATH=$T/bin:$PATH YANTRIK_CI_MAX_CYCLES=1 run 16c "REFUS|done:|launching|could not hand" timeout 300 $C
has "$(out 16c)" "REFUSING to run in VM .*: its port could not be isolated and locked" "a failed net_lock refuses the VM"
has "$(out 16c)" "job exit: lock-failed" "and ends it"
hasnt "$(out 16c)" "runner launching" "the config was never handed over"
rm -f "$T/bin/bridge"
no_vm
net_gone

echo "--- 17: L1: the controller is SIGKILLed mid-job: the VM keeps running with its pin until the unit's ExecStopPost cleanup runs"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=0 YANTRIK_CI_IDLE_LIMIT=300 $C > "$T/t17.out" 2>&1 &
P=$!
for _ in $(seq 1 120); do grep -q "runner launching" "$T/t17.out" && break; sleep 1; done
sleep 3
kill -KILL $P
wait $P 2> /dev/null
sleep 3
ls /etc/pve/nodes/*/qemu-server/954[1-9].conf > /dev/null 2>&1 && ok "after SIGKILL the VM is still there (what the cleanup is for)" || no "the VM is not there after SIGKILL"
[ "$(pins_left)" -gt 0 ] && ok "and it is still pinned" || no "and it is still pinned"
yantrik-ci-net cleanup > "$T/t17c.out" 2>&1
rc=$?
filt "start-up cleanup|REFUS|could not" < "$T/t17c.out"
[ "$rc" = 0 ] && ok "the cleanup (ExecStopPost) exits 0" || no "the cleanup exits $rc"
no_vm
[ "$(pins_left)" = 0 ] && ok "no pin left in either table" || no "pins left: $(pins_left)"
{ ip -br link | grep -q 'tap954'; } && no "a tap is left" || ok "no tap left"
net_gone
# shellcheck disable=SC2009
ps -eo pid,args | grep -v grep | grep -q 'yantrik-ci-controller' && no "a controller process is left" || ok "no controller process is left"
sleep 3

echo "--- 17b: the cleanup also works when a VM cannot be destroyed: it is unpinned and the cleanup exits 1"
yantrik-ci-net up > /dev/null
qm clone 9540 9542 --name yantrik-ci-9542 > /dev/null 2>&1
yantrik-ci-net pin 9542 > /dev/null
PATH=$T/bin:$PATH FAKE_QM=no-destroy yantrik-ci-net cleanup > "$T/t17d.out" 2>&1
rc=$?
filt "unpinning|REFUS" < "$T/t17d.out"
[ "$rc" = 1 ] && ok "the cleanup exits 1 (the unit shows failed)" || no "the cleanup exits $rc"
[ "$(pins_left)" = 0 ] && ok "the survivor is unpinned" || no "pins left: $(pins_left)"
# (This survivor was never started, so no port is on the bridge and net_down may go ahead; test 16 shows the bridge staying when a
# running survivor has a port on it.)
qm destroy 9542 --purge 1 > /dev/null 2>&1
yantrik-ci-net down > /dev/null 2>&1
no_vm
net_gone

echo "--- 18: L1: the stop file alone (no signal) ends a running job: the unit's ExecStop= makes it"
echo slow > "$T/mode"
YANTRIK_CI_MAX_CYCLES=0 YANTRIK_CI_IDLE_LIMIT=300 $C > "$T/t18.out" 2>&1 &
P=$!
for _ in $(seq 1 120); do grep -q "runner launching" "$T/t18.out" && break; sleep 1; done
sleep 3
t_stop=$(date +%s)
touch /run/yantrik-ci/stop
wait $P
rc=$?
t_end=$(date +%s)
filt "done:|stopped" < "$T/t18.out"
has "$T/t18.out" "job exit: 143" "the stop file ends the job"
[ "$rc" = 0 ] && ok "the controller exits 0 after $((t_end - t_stop)) s, with no signal sent" || no "exit status $rc"
no_vm
net_gone

echo "--- 19: L1: SIGTERM at 25 random moments of start-up (the unit sends the stop file first, then SIGTERM): always gone within 45 s"
lost=0
slowest=0
for _ in $(seq 1 25); do
  YANTRIK_CI_MIN_MEM_KB=999999999999 $C > "$T/t19.out" 2>&1 &
  P=$!
  sleep "0.$((RANDOM % 9 + 1))"
  touch /run/yantrik-ci/stop
  kill -TERM $P 2> /dev/null
  t0=$SECONDS
  for _ in $(seq 1 450); do kill -0 $P 2> /dev/null || break; sleep 0.1; done
  if kill -0 $P 2> /dev/null; then lost=$((lost + 1)); kill -KILL $P; fi
  wait $P 2> /dev/null
  [ $((SECONDS - t0)) -le "$slowest" ] || slowest=$((SECONDS - t0))
done
[ "$lost" = 0 ] && ok "25 of 25 stops ended the controller (slowest $slowest s)" || no "$lost of 25 stops left the controller running"
rm -f /run/yantrik-ci/stop
no_vm
net_gone

echo "--- 20: the unit: ExecStop= makes the stop file, ExecStopPost= runs the cleanup, nothing starts it"
has "$HERE/../yantrik-ci-controller.service" "^ExecStop=.*touch /run/yantrik-ci/stop" "ExecStop makes the stop file"
has "$HERE/../yantrik-ci-controller.service" "^ExecStopPost=/usr/local/sbin/yantrik-ci-net cleanup" "ExecStopPost runs the cleanup"
hasnt "$HERE/../install-controller.sh" "^[[:space:]]*systemctl (enable|start|restart)" "the installer never enables, starts or restarts it (it only says how)"

echo "--- 21: N1: the agent's device cannot be removed from the host's side: the VM is destroyed, no runner was started"
echo slow > "$T/mode"
PATH=$T/bin:$PATH FAKE_QM=agent-stays YANTRIK_CI_AGENT_CLOSE=5 YANTRIK_CI_MAX_CYCLES=1 YANTRIK_CI_IDLE_LIMIT=60 run 21 "agent|REFUS|done:|launching" timeout 300 $C
has "$(out 21)" "VM 9541: the agent's device is still attached in QEMU's view" "QEMU's view says the device is still there (not the guest's ping)"
has "$(out 21)" "REFUSING to run in VM 9541: the agent's device could not be removed" "the VM is refused"
has "$(out 21)" "job exit: agent-open" "and ended"
hasnt "$(out 21)" "runner launching" "no runner was launched"
no_vm
net_gone

echo "--- 22: N2: the stop-time cleanup takes the controller's lock"
has "$HERE/../yantrik-ci-net" "flock -w .* 8 .*no cleanup" "the cleanup flocks the controller's lock file"
yantrik-ci-net up > /dev/null
for i in 9542 9543; do
  qm clone 9540 $i --name yantrik-ci-$i > /dev/null 2>&1
  yantrik-ci-net pin $i > /dev/null
done
: > "$T/destroys.log"
PATH=$T/bin:$PATH FAKE_QM=slow-destroy FAKE_QM_LOG=$T/destroys.log yantrik-ci-net cleanup > "$T/t22a.out" 2>&1 &
P1=$!
sleep 1
PATH=$T/bin:$PATH FAKE_QM=slow-destroy FAKE_QM_LOG=$T/destroys.log yantrik-ci-net cleanup > "$T/t22b.out" 2>&1 &
P2=$!
wait $P1
rc1=$?
wait $P2
rc2=$?
[ "$rc1" = 0 ] && [ "$rc2" = 0 ] && ok "two cleanups at once both exit 0" || no "two cleanups at once exit $rc1 and $rc2"
for i in 9542 9543; do
  n=$(grep -c "destroy $i\$" "$T/destroys.log")
  [ "$n" = 1 ] && ok "VM $i was destroyed once (the second cleanup waited for the first and found nothing)" || no "VM $i: $n destroy calls (two cleanups ran side by side)"
done
sed 's/^/  log: /' "$T/destroys.log" 
no_vm
[ "$(pins_left)" = 0 ] && ok "no pin left" || no "pins left: $(pins_left)"
net_gone

echo "--- 22b: N2: a cleanup while the controller script holds the lock touches nothing"
yantrik-ci-net up > /dev/null
YANTRIK_CI_MIN_MEM_KB=999999999999 $C > "$T/t22c.out" 2>&1 &
CP=$!
for _ in $(seq 1 60); do grep -q "waiting for memory" "$T/t22c.out" && break; sleep 1; done
qm clone 9540 9542 --name yantrik-ci-9542 > /dev/null 2>&1
yantrik-ci-net pin 9542 > /dev/null
YANTRIK_CI_CLEANUP_WAIT=3 yantrik-ci-net cleanup > "$T/t22d.out" 2>&1
rc=$?
filt "lock" < "$T/t22d.out"
[ "$rc" = 0 ] && ok "the cleanup exits 0 and says so" || no "the cleanup exits $rc"
has "$T/t22d.out" "holds the lock: no cleanup" "it names the lock"
vm_exists_9542() { ls /etc/pve/nodes/*/qemu-server/9542.conf > /dev/null 2>&1; }
vm_exists_9542 && ok "VM 9542 was not destroyed while the controller held the lock" || no "the cleanup destroyed a VM beside a running controller"
[ "$(pins_left)" -gt 0 ] && ok "its pin was not removed either" || no "its pin was removed"
ip link show yci0 > /dev/null 2>&1 && ok "yci0 was not removed" || no "yci0 was removed"
touch /run/yantrik-ci/stop
kill -TERM $CP
wait $CP 2> /dev/null
rm -f /run/yantrik-ci/stop
yantrik-ci-net cleanup > "$T/t22e.out" 2>&1
rc=$?
[ "$rc" = 0 ] && ok "after the controller has stopped, the cleanup takes the lock and runs (exit 0)" || no "the cleanup exits $rc once the lock is free"
has "$T/t22e.out" "destroying leftover VM 9542" "and destroys the leftover"
no_vm
net_gone

echo "--- 23: N3: the clone's configuration values are checked (a bad value refuses the clone)"
qm clone 9540 9541 --name yantrik-ci-9541 > /dev/null 2>&1
good=$(qm config 9541)
LIBHOST=/usr/local/lib/yantrik-ci/host.sh
log() { echo "    $*"; }
cfg_check() {
  (
    export TEMPLATE_ID=9540
    # shellcheck source=/dev/null
    . "$LIBHOST"
    clone_keys_ok 9541 "$1"
  ) 2>&1
}
if cfg_check "$good" > "$T/t23.out"; then ok "a plain clone's real configuration passes"; else no "a plain clone's real configuration is refused"; cat "$T/t23.out"; fi
refused() { # label sed-expression: the real configuration with one line changed is refused
  local bad out
  bad=$(sed -E "$2" <<< "$good")
  if out=$(cfg_check "$bad"); then no "$1: passed"; else ok "$1: refused"; echo "$out" | head -2; fi
}
refused "scsi1 is a host disk" 's|^scsi1: .*|scsi1: /dev/sdb,media=disk|'
refused "scsi1 is another VM's cloud-init drive" 's|^scsi1: (.*)vm-9541-cloudinit|scsi1: \1vm-9542-cloudinit|'
refused "serial0 is a host serial port" 's|^serial0: .*|serial0: /dev/ttyS0|'
refused "vga is not the serial port" 's|^vga: .*|vga: std|'
refused "scsi0 is a host disk" 's|^scsi0: .*|scsi0: /dev/sdb,size=80G|'
refused "scsi0 is on another storage" 's|^scsi0: [^:]*:|scsi0: other:|'
refused "scsi0 is another VM's disk" 's|^scsi0: (.*)vm-9541-disk-0|scsi0: \1vm-9542-disk-0|'
refused "allow-ksm is 1" 's|^allow-ksm: 0|allow-ksm: 1|'
refused "allow-ksm is missing" '/^allow-ksm:/d'
refused "ide2 is back (a key the rebuilt template has not)" '$a ide2: local:iso/x.iso,media=cdrom'
refused "a key the list does not know" '$a hostpci0: 0000:01:00.0'
qm destroy 9541 --purge 1 > /dev/null 2>&1
echo "--- 23b: N3: the controller refuses a real clone whose configuration was changed, before it starts"
echo slow > "$T/mode"
PATH=$T/bin:$PATH FAKE_QM=bad-config FAKE_QM_SED='s|^serial0: socket|serial0: /dev/ttyS0|' YANTRIK_CI_MAX_CYCLES=1 run 23b "configuration|REFUS|done:|launching" timeout 300 $C
has "$(out 23b)" "VM 9541: its configuration lacks 'serial0' with the template's value" "the changed value is named"
has "$(out 23b)" "job exit: config-failed" "and the clone is ended"
hasnt "$(out 23b)" "runner launching" "no runner was launched"
no_vm
net_gone

kill "$STUB"
echo "--- secret scan: the controller's log and run output, the node's disks, the template's disk (matches):"
TOK=$(cat "$T/token")
J1=$(sed -n 1p "$T/jit-marker")
J2=$(sed -n 2p "$T/jit-marker")
for pat in "$TOK" "$J1" "$J2" "STUBSECRETMARK"; do
  n=$(grep -rF -c -- "$pat" "$T/log" "$T"/t*.out /run/yantrik-ci /var/log /var/tmp /tmp 2> /dev/null | awk -F: '{s+=$NF} END{print s+0}')
  [ "$n" = 0 ] && ok "pattern (${#pat} chars): 0 matches in the logs, /run, /var/tmp, /tmp" || no "pattern (${#pat} chars): $n matches"
done
tv=/dev/pve/base-9540-disk-0
if [ -e "$tv" ]; then
  # In chunks: grep over a device of zeros would read one 80 GB "line" into memory.
  n=$(python3 -I -c '
import sys
m = sys.argv[2].encode()
n, tail = 0, b""
with open(sys.argv[1], "rb") as f:
    while True:
        c = f.read(16 << 20)
        if not c:
            break
        buf = tail + c
        n += buf.count(m)
        tail = buf[-(len(m) - 1):]
print(n)' "$tv" "STUBSECRETMARK" 2> /dev/null)
  [ "${n:-0}" = 0 ] && ok "the template's disk holds no trace of the fake config (0 matches)" || no "the template's disk holds $n matches"
else
  echo "  NOT RUN  the template's disk $tv is not a device here"
fi
st="$(systemctl is-enabled yantrik-ci-controller 2>&1) / $(systemctl is-active yantrik-ci-controller 2>&1)"
[ "$st" = "disabled / inactive" ] && ok "the unit is $st" || no "the unit is $st"
if [ "$fails" -eq 0 ]; then echo "== controller tests: ALL PASS"; else echo "== controller tests: FAIL ($fails)"; fi
[ "$fails" -eq 0 ]
