#!/usr/bin/env bash
# Proves the clones' network (host/yci.nft, lib/net.sh) on the node with REAL clones and no runner, no
# token and no job: nothing here talks to GitHub's runner API. Run as root, with the controller stopped
# and no VM in 9541-9549. Usage: check-net.sh [A [B]]   (the two clone ids, default 9548 and 9549)
#
# It brings up yci0 and both tables (inet, netdev), starts clone A unpinned (it must have no network), pins it and
# checks what it can and cannot reach, as root with only the node's table in the way; sends spoofed
# frames and counts, with tcpdump on the node, what arrives; starts clone B for the clone-to-clone checks;
# loads the in-guest filter and runs the proofs as `runner`; then destroys both clones, brings the
# network down, and compares the node's links, tables and the bridge ports of every other guest with
# what they were before. The clones are reached through the QEMU guest agent: no ssh, no key.
# Ends with ALL PASS or FAIL (exit status 1). Never enables or starts the controller.
# shellcheck disable=SC2015,SC2001,SC2016
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
GUEST=$(cd "$HERE/../guest" && pwd)
LIB_DIR=${YANTRIK_CI_LIB:-/usr/local/lib/yantrik-ci}
RUN_DIR=${YANTRIK_CI_RUN_DIR:-/run/yantrik-ci}
OWN_PUBLIC_FILE=${YANTRIK_CI_OWN_PUBLIC:-/etc/yantrik-ci/own-public-ips}
A=${1:-9548}
B=${2:-9549}
ID_MIN=9541
ID_MAX=9549
TEMPLATE_ID=9540
POOL=pve/data
[ "$(id -u)" -eq 0 ] || { echo "check-net: run as root" >&2; exit 1; }
log() { printf '  %s\n' "$*"; }
mkdir -p "$RUN_DIR"
chmod 700 "$RUN_DIR"
for f in "$LIB_DIR" "$LIB_DIR/trust.sh"; do
  m=$(stat -c '%u %a' "$f" 2> /dev/null) || { echo "cannot stat $f" >&2; exit 1; }
  if [ "${m%% *}" != 0 ] || [ $((8#${m#* } & 8#022)) -ne 0 ]; then echo "REFUSING to use $f: owner/mode '$m'" >&2; exit 1; fi
done
# shellcheck source=lib/trust.sh
. "$LIB_DIR/trust.sh"
# shellcheck source=lib/host.sh
. "$LIB_DIR/host.sh"
# shellcheck source=lib/net.sh
. "$LIB_DIR/net.sh"
for id in "$A" "$B"; do
  in_range "$id" || { echo "check-net: $id is not in $ID_MIN-$ID_MAX" >&2; exit 1; }
  id_in_use "$id" && { echo "check-net: $id exists: not touching it" >&2; exit 1; }
done
systemctl is-active --quiet yantrik-ci-controller && { echo "check-net: the controller is running" >&2; exit 1; }
# The controller's own lock (N2): neither a controller nor the stop-time cleanup runs beside this check, whose clones (9548, 9549)
# the cleanup would destroy.
exec 8> "$RUN_DIR/controller.lock"
flock -n 8 || { echo "check-net: a controller or a cleanup holds the lock" >&2; exit 1; }

W=$(mktemp -d /var/tmp/yci-check.XXXXXX)
NET_WAS_UP=0
net_present && NET_WAS_UP=1
PIDS=()
fails=0
ok() { echo "  PASS  $1"; }
no() { echo "  FAIL  $1"; fails=$((fails + 1)); }
cleanup() {
  local p
  for p in "${PIDS[@]}"; do kill "$p" 2> /dev/null || true; done
  for id in "$A" "$B"; do vm_exists "$id" && destroy_vm "$id"; done
  [ "$NET_WAS_UP" -eq 1 ] || net_down > /dev/null 2>&1
  rm -rf "$W"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# gx ID [VAR=value ...]: run the script on stdin in the guest, as root, through the guest agent; print its
# output, and return its exit status.
gx() {
  local id=$1 out
  shift
  out=$(qm guest exec "$id" --pass-stdin 1 --timeout 240 -- env "$@" bash -s 2>&1)
  python3 -c '
import json, sys
raw = sys.stdin.read()
try:
    d = json.loads(raw)
except ValueError:
    print(raw)
    sys.exit(254)
sys.stdout.write(d.get("out-data", "") + d.get("err-data", ""))
sys.exit(d.get("exitcode", 255))' <<< "$out"
}
count() { tcpdump -nn -r "$1" "${@:2}" 2> /dev/null | wc -l; }
expect_count() { # label file wanted(0|pos) filter...
  local n
  n=$(count "$2" "${@:4}")
  if [ "$3" = 0 ]; then [ "$n" -eq 0 ] && ok "$1: dropped (0 frames reached the bridge)" || no "$1: $n frames got through"
  else [ "$n" -gt 0 ] && ok "$1: arrived ($n)" || no "$1: nothing arrived"; fi
}
start_clone() { # id pin(0|1)
  clone_vm "$1" && clone_configure "$1" || { no "clone $1 made"; return 1; }
  [ "$2" -eq 0 ] || net_pin "$1" || { no "clone $1 pinned"; return 1; }
  qm start "$1" > /dev/null 2>&1 8>&- && vm_oom_first "$1" && vm_agent_wait "$1" || { no "clone $1 started and answered the agent"; return 1; }
  [ "$2" -eq 0 ] || net_lock "$1" || { no "clone $1's port isolated and locked"; return 1; }
}
# l2c mac|arpok|arpdrop|other: the packet counter of one rule of the netdev table's ingress chain
l2c() {
  local pat
  case $1 in
    mac) pat='^[[:space:]]*iifname \. ether saddr' ;;
    arpok) pat='arp daddr ip' ;;
    arpdrop) pat='ether type arp counter' ;;
    vlan) pat='ether type .*8021' ;;
    other) pat='^[[:space:]]*counter packets' ;;
  esac
  nft list chain netdev "$NET_TABLE_L2" ingress 2> /dev/null | awk -v pat="$pat" '$0 ~ pat {for (i = 1; i < NF; i++) if ($i == "packets") {print $(i + 1); exit}}'
}
ingress_drops() { nft list chain inet "$NET_TABLE" ingress 2> /dev/null | awk '/counter packets/ && /drop/ {sum += $(NF-3)} END {print sum + 0}'; }

echo "== before: the node's own state"
ip -br link | sed 's/ .*//' | sort > "$W/links.before"
nft list tables > "$W/tables.before" 2>&1
bridge link show 2> /dev/null | grep -v 'tap954' | sed 's/ state.*//' | sort > "$W/ports.before"
sysctls() { local f; for f in /proc/sys/net/ipv4/ip_forward /proc/sys/net/ipv4/conf/{all,default,"$(net_uplink)"}/{forwarding,rp_filter,send_redirects,accept_redirects}; do echo "$f $(cat "$f")"; done; }
sysctls > "$W/sysctl.before"
NODE_KERNEL=$(uname -r)
NODE_LAN_IP=$(ip -4 -o addr show "$(net_uplink)" | awk '{sub(/\/.*/, "", $4); print $4; exit}')
GW_IP=$(ip -4 route show default | awk '{print $3; exit}')
LAN_IP=$(pct config 530 2> /dev/null | sed -n '/^net0:/s/.*ip=\([0-9.]*\).*/\1/p')
OWN_IPS=$(grep -vE '^[[:space:]]*(#|$)' "$OWN_PUBLIC_FILE" | paste -sd, -)
log "links $(wc -l < "$W/links.before"), tables: $(wc -l < "$W/tables.before"), other bridge ports: $(wc -l < "$W/ports.before"); node kernel $NODE_KERNEL"
[ -n "$NODE_LAN_IP" ] && [ -n "$GW_IP" ] && [ -n "$LAN_IP" ] && [ -n "$OWN_IPS" ] || { echo "cannot work out the addresses to test against"; exit 1; }

echo "== network up: the bridge $NET_BRIDGE (no physical port) and the table $NET_TABLE"
net_up && ok "net_up" || { no "net_up"; exit 1; }
ip -d link show "$NET_BRIDGE" | grep -q 'bridge ' && [ -z "$(net_ports)" ] && ok "the bridge has no port of its own" || no "the bridge has no port"

echo "== 0: the layer-2 table is on a port from the moment it exists (before a guest could send its first frame)"
TW=9547
{ [ "$A" = "$TW" ] || [ "$B" = "$TW" ]; } && TW=9546
id_in_use "$TW" && { echo "check-net: $TW exists: not touching it" >&2; exit 1; }
timeout 20 tcpdump -ni "$NET_BRIDGE" -w "$W/tw.pcap" > /dev/null 2>&1 &
PIDS+=($!)
sleep 2
m0=$(l2c mac)
python3 -I "$HERE/tap-window.py" "$(net_port "$TW")" "$NET_BRIDGE" "$NET_GW" | sed 's/^/  /'
m1=$(l2c mac)
sleep 1
for p in "${PIDS[@]}"; do kill "$p" 2> /dev/null || true; done
wait 2> /dev/null
PIDS=()
[ "$((m1 - m0))" -ge 2 ] && ok "two frames written into a brand-new unpinned port were dropped by the netdev table (counter $m0 to $m1)" || no "the first frames of a new port were not dropped ($m0 to $m1)"
expect_count "nothing from that port reached the bridge" "$W/tw.pcap" 0 ether src 02:11:22:33:44:55
[ -e "/sys/class/net/$(net_port "$TW")" ] && no "the test port is still there" || ok "the test port is gone"

echo "== 1: clone $A starts UNPINNED: its port is dropped, it has no network at all"
start_clone "$A" 0 && ok "clone $A booted (guest agent answers)"
d0=$(l2c mac)
out=$(gx "$A" <<< 'curl -m6 -s -o /dev/null https://github.com && echo REACHED || echo BLOCKED; ping -c2 -W2 10.95.30.1 > /dev/null && echo PINGED || echo NOPING')
echo "$out" | grep -q BLOCKED && echo "$out" | grep -q NOPING && ok "unpinned: no HTTPS, no ping" || no "unpinned clone had network: $out"
[ "$(l2c mac)" -gt "$d0" ] && ok "the netdev table's wrong-MAC counter moved: the unpinned port was dropped there ($d0 to $(l2c mac))" || no "the unpinned port's drops were not counted"

echo "== 2: pin it: it has its own kernel and the public internet over 443 and DNS"
net_pin "$A" && net_lock "$A" && ok "pinned, isolated and locked $(net_port "$A")" || no "pinned, isolated and locked"
bridge -d link show dev "$(net_port "$A")" | sed 's/^/  /'
sleep 6
echo "  kernel inside: $(gx "$A" <<< 'uname -r')   node: $NODE_KERNEL"
[ "$(gx "$A" <<< 'uname -r')" != "$NODE_KERNEL" ] && ok "the clone runs its own kernel, not the node's" || no "same kernel as the node"
start_clone "$B" 1 && ok "clone $B booted and pinned"
ud=$(qm cloudinit dump "$A" user 2> /dev/null)
if grep -q '^#cloud-config' <<< "$ud" && ! grep -qE 'package_(upgrade|update): true' <<< "$ud" && qm config "$A" | grep -q '^ciupgrade: 0$'; then
  ok "the clone's cloud-init user-data does not ask for a package upgrade (ciupgrade 0), so a clone is the template plus nothing"
else no "the clone's user-data asks for a package upgrade, or could not be read"; fi
(qm guest exec "$B" --synchronous 0 -- python3 -m http.server 8080 --bind 0.0.0.0 > /dev/null 2>&1) &
PIDS+=($!)
for _ in $(seq 1 15); do curl -m3 -s -o /dev/null "http://$(net_ip "$B"):8080/" && break; sleep 2; done
curl -m5 -s -o /dev/null "http://$(net_ip "$B"):8080/" && ok "control: the node reaches a listener in clone $B" || no "control: the node reaches a listener in clone $B"
echo "== 3: as root, with only the node's table in the way (the in-guest filter is not loaded)"
res=$(gx "$A" NODE_LAN_IP="$NODE_LAN_IP" NODE_BR_IP="$NET_GW" LAN_IP="$LAN_IP" GW_IP="$GW_IP" OWN_IPS="$OWN_IPS" PEER_IP="$(net_ip "$B")" < "$HERE/guest-net-probes.sh")
echo "$res" | sed 's/^/  /'
grep -q '^FAIL' <<< "$res" && no "a probe failed" || { grep -q '^PASS' <<< "$res" && ok "every probe held"; }

echo "== 4: spoofed frames from clone $A (counted on the node, with tcpdump)"
MACA=$(vm_mac "$A" | tr 'A-F' 'a-f')
timeout 30 tcpdump -ni "$NET_BRIDGE" -w "$W/yci0.pcap" > /dev/null 2>&1 &
PIDS+=($!)
timeout 30 tcpdump -ni "$(net_port "$B")" -w "$W/tapB.pcap" > /dev/null 2>&1 &
PIDS+=($!)
sleep 2
d0=$(ingress_drops)
gx "$A" < <(printf 'python3 - %s %s %s %s <<'"'"'PY'"'"'\n' "$(net_ip "$A")" "$NET_MAC" "$(vm_mac "$B" | tr 'A-F' 'a-f')" "$(net_ip "$B")"; cat "$HERE/guest-spoof.py"; printf 'PY\n') | sed 's/^/  /'
sleep 3
for p in "${PIDS[@]}"; do kill "$p" 2> /dev/null || true; done
wait 2> /dev/null
expect_count "control: own MAC and address to the node's MAC (udp 40001)" "$W/yci0.pcap" pos udp dst port 40001
expect_count "spoofed source address (udp 40002)" "$W/yci0.pcap" 0 udp dst port 40002
expect_count "spoofed source MAC (udp 40003)" "$W/yci0.pcap" 0 udp dst port 40003
expect_count "spoofed source address and MAC (udp 40004)" "$W/yci0.pcap" 0 udp dst port 40004
expect_count "VLAN-tagged frame (udp 40007)" "$W/yci0.pcap" 0 vlan
expect_count "DHCP discover from 0.0.0.0 (udp 67)" "$W/yci0.pcap" 0 udp dst port 67
expect_count "IPv6 frame" "$W/yci0.pcap" 0 ip6
expect_count "control: ARP who-has for the node's address, as the clone" "$W/yci0.pcap" pos arp and ether src "$MACA" and src host "$(net_ip "$A")"
# The inet table cannot see ARP or other non-IP frames; the netdev table (host/yci-l2.nft) does (#1303).
expect_count "forged ARP reply (the node's address at the clone's MAC): dropped by the netdev table" "$W/yci0.pcap" 0 arp and src host "$NET_GW" and ether src "$MACA"
nb=$(ip neigh show dev "$NET_BRIDGE" | grep -c "^$NET_GW ")
[ "$nb" -eq 0 ] && ok "the node's neighbour table did not take the forged entry for its own address" || no "the node's neighbour table holds an entry for its own address"
expect_count "forged ARP and every broadcast from clone $A: none reach clone $B (isolated port)" "$W/tapB.pcap" 0 ether src "$MACA"
expect_count "an experimental-ethertype broadcast with its own MAC: dropped by the netdev table" "$W/yci0.pcap" 0 ether proto 0x88b5 and ether src "$MACA"
expect_count "... and it does not reach clone $B either" "$W/tapB.pcap" 0 ether proto 0x88b5
expect_count "the same frame with a forged source MAC (not an IP or ARP frame): dropped by the locked port" "$W/yci0.pcap" 0 ether proto 0x88b5 and ether src 02:aa:bb:cc:dd:ee
expect_count "frame addressed to clone $B's MAC (no clone to clone, udp 40005)" "$W/tapB.pcap" 0 udp dst port 40005
[ "$(ingress_drops)" -gt "$d0" ] && ok "the ingress drop counters moved ($d0 to $(ingress_drops))" || no "the ingress drop counters moved"
res=$(gx "$A" <<< 'I=$(ip -o route show default | awk "{print \$5; exit}"); O=$(cat /sys/class/net/$I/address); ip link set dev $I address 02:aa:bb:cc:dd:ee; sleep 1; curl -m6 -s -o /dev/null https://github.com && echo REACHED || echo BLOCKED; ip link set dev $I address $O')
echo "$res" | grep -q BLOCKED && ok "the clone changed its own MAC: no network" || no "a changed MAC kept the network: $res"
sleep 3
gx "$A" <<< 'curl -m10 -sSfo /dev/null https://github.com' > /dev/null && ok "its own MAC back: the network is back" || no "its own MAC back"

echo "== 4b: layer 2 (#1303): nine forged frames from clone $A, one at a time, against the netdev table's counters"
send_frame() { gx "$A" < <(printf 'python3 - %s %s %s %s %s %s %s <<'"'"'PY'"'"'\n' "$(net_ip "$A")" "$NET_MAC" "$(vm_mac "$B" | tr 'A-F' 'a-f')" "$(net_ip "$B")" "$1" "$LAN_IP" "$NODE_LAN_IP"; cat "$HERE/guest-spoof.py"; printf 'PY\n') > /dev/null; sleep 1; }
# l2_case LABEL FRAME COUNTER FILTER...: the counter moves, nothing matching the filter reaches the bridge
l2_case() {
  local b a
  b=$(l2c "$3")
  send_frame "$2"
  a=$(l2c "$3")
  [ "$a" -gt "$b" ] && ok "$1: dropped, the $3 counter went $b to $a" || no "$1: the $3 counter did not move ($b to $a)"
  L2FILTERS+=("$1|${*:4}")
}
L2FILTERS=()
timeout 90 tcpdump -ni "$NET_BRIDGE" -w "$W/l2.pcap" > /dev/null 2>&1 &
PIDS+=($!)
sleep 2
b=$(l2c arpok)
send_frame arp-gw-request
a=$(l2c arpok)
[ "$a" -gt "$b" ] && ok "control: an honest ARP request for the gateway passed (arpok counter $b to $a)" || no "control: the honest ARP request was not accepted"
sleep 1
ip neigh show dev "$NET_BRIDGE" | awk '{print $1, $3}' | sort > "$W/neigh.before"
l2_case "(a) forged ARP reply claiming the gateway's address" arp-gw-reply arpdrop arp and ether src "$MACA" and src host "$NET_GW"
l2_case "(b) forged ARP reply claiming clone $B's address" arp-peer-reply arpdrop arp and ether src "$MACA" and src host "$(net_ip "$B")"
l2_case "(c) gratuitous ARP for the LAN address" arp-gratuitous arpdrop arp and ether src "$MACA" and src host "$LAN_IP"
l2_case "(d) ARP request for the LAN address" arp-lan-request arpdrop arp and ether src "$MACA" and dst host "$LAN_IP" and src host "$(net_ip "$A")"
l2_case "(e) frame with an unused ethertype" ethertype other ether proto 0x88b5 and ether src "$MACA"
l2_case "(f) forged ARP reply claiming the node's LAN address" arp-node-lan-reply arpdrop arp and ether src "$MACA" and src host "$NODE_LAN_IP"
l2_case "(g) ARP whose sender MAC differs from the frame's source" arp-mac-mismatch arpdrop arp and 'arp[8:2]' = 0x02aa
l2_case "(h) 802.1Q-tagged ARP" arp-vlan vlan vlan
l2_case "(i) ARP request for the node's LAN address (no reply either)" arp-node-lan-request arpdrop arp and ether src "$MACA" and dst host "$NODE_LAN_IP"
sleep 2
for p in "${PIDS[@]}"; do kill "$p" 2> /dev/null || true; done
wait 2> /dev/null
expect_count "(i) the node sent no ARP reply for its LAN address to the clone" "$W/l2.pcap" 0 arp and 'arp[6:2]' = 2 and ether dst "$MACA" and src host "$NODE_LAN_IP"
nbm=$(ip neigh show dev "$NET_BRIDGE" | awk -v ip="$(net_ip "$B")" '$1 == ip {print $3}')
[ "$nbm" = "$(vm_mac "$B" | tr 'A-F' 'a-f')" ] && ok "the node's neighbour entry for clone $B ($(net_ip "$B")) is still clone $B's own MAC" || no "the node's neighbour entry for clone $B is '$nbm'"
expect_count "control: the honest ARP request arrived on the bridge" "$W/l2.pcap" pos arp and ether src "$MACA" and src host "$(net_ip "$A")" and dst host "$NET_GW"
set -f
for f in "${L2FILTERS[@]}"; do
  # shellcheck disable=SC2086
  expect_count "${f%%|*}" "$W/l2.pcap" 0 ${f#*|}
done
set +f
ip neigh show dev "$NET_BRIDGE" | awk '{print $1, $3}' | sort > "$W/neigh.after"
diff -q "$W/neigh.before" "$W/neigh.after" > /dev/null && ok "the node's neighbour table is unchanged by the forged frames ($(wc -l < "$W/neigh.after") entries)" || { no "the neighbour table changed"; diff "$W/neigh.before" "$W/neigh.after"; }
log "netdev table counters (packets): wrong-MAC $(l2c mac), ARP accepted $(l2c arpok), ARP dropped $(l2c arpdrop), other frames dropped $(l2c other)"
res=$(gx "$A" <<< 'ip neigh flush all; curl -m10 -sSfo /dev/null https://github.com && echo REACHED || echo BLOCKED; ip neigh show | head -3')
echo "$res" | sed 's/^/  guest: /'
echo "$res" | grep -q REACHED && ok "afterwards the clone still reaches GitHub over 443 (its own honest ARP works)" || no "the clone lost the network after the forged frames"

echo "== 5: in-guest filter loaded, then the proofs as runner (also: runner cannot change the rules or become root)"
res=$(gx "$A" SEALED=1 NODE_LAN_IP="$NODE_LAN_IP" NODE_BR_IP="$NET_GW" LAN_IP="$LAN_IP" < "$GUEST/template-proofs.sh")
echo "$res" | sed 's/^/  /'
grep -q '^FAIL' <<< "$res" && no "a runner proof failed" || { grep -q '^PASS' <<< "$res" && ok "every runner proof held"; }

echo "== 5b: the guest agent is closed after the hand-over (L3, N1): decided by QEMU's own view, not by the guest"
qm agent "$B" ping > /dev/null 2>&1 && ok "control: the agent of clone $B answers before" || no "control: the agent of clone $B answers before"
[ "$(vm_agent_state "$B")" = attached ] && ok "control: QEMU lists the agent's device (qga0) on clone $B" || no "control: QEMU lists qga0 on clone $B"
# The template on the node has the previous launcher until it is rebuilt: put this branch's launcher into clone A.
{ echo "base64 -d > /usr/local/sbin/yantrik-ci-launch << 'B64'"; base64 -w0 "$GUEST/yantrik-ci-launch"; echo; echo B64; echo "chmod 755 /usr/local/sbin/yantrik-ci-launch"; } | gx "$A" > /dev/null
res=$(gx "$A" <<< 'YANTRIK_CI_AGENT_WAIT=3 /usr/local/sbin/yantrik-ci-launch --wait-agent-gone; echo "rc=$?"')
grep -q 'rc=1' <<< "$res" && ok "guest, agent port present: the launcher's wait fails after its bound (so it would start no runner)" || no "the launcher's wait did not fail with the port present: $res"
res=$(gx "$A" <<< 'p=/dev/virtio-ports/org.qemu.guest_agent.0; ( sleep 2; rm -f "$p" ) & YANTRIK_CI_AGENT_WAIT=15 /usr/local/sbin/yantrik-ci-launch --wait-agent-gone; echo "rc=$?"')
grep -q 'rc=0' <<< "$res" && ok "guest, agent port gone: the launcher's wait returns at once" || no "the launcher's wait did not see the port go: $res"
# End to end on clone A: the launcher stops the agent and waits for the port, the host takes the device away.
gx "$A" <<< 'systemd-run --no-block --quiet /usr/local/sbin/yantrik-ci-launch --close-agent; echo started' | sed 's/^/  /'
t0=$SECONDS
if vm_agent_close "$A"; then ok "clone $A: QEMU's view (info pci and qom-list) shows qga0 gone $((SECONDS - t0)) s after device_del"; else no "clone $A: qga0 still there in QEMU's view"; fi
qm agent "$A" ping > /dev/null 2>&1 && no "the agent of clone $A still answers" || ok "clone $A: the agent no longer answers"
echo "  QEMU's own view of clone $A:"
printf '%s\n' 'info pci' | qm monitor "$A" 2> /dev/null | grep -ciE 'id "qga0"' | sed 's/^/    info pci lines naming qga0: /'
printf '%s\n' 'qom-list /machine/peripheral' | qm monitor "$A" 2> /dev/null | grep -c '^qga0 ' | sed 's/^/    qom-list peripherals named qga0: /'
# The guest keeps its agent running and does not acknowledge the unplug (its CPUs are stopped, as a guest that ignores
# the eject request): QEMU still lists the device, so the close is refused, whatever the agent's ping says.
printf '%s\n' 'stop' | qm monitor "$B" > /dev/null 2>&1
AGENT_CLOSE_S=6
t0=$SECONDS
if vm_agent_close "$B"; then no "clone $B: a close was reported for a guest that never acknowledged the unplug"; else ok "clone $B: the close is refused after $((SECONDS - t0)) s: the device is still in QEMU's view"; fi
[ "$(vm_agent_state "$B")" = attached ] && ok "clone $B: vm_agent_state says attached" || no "clone $B: vm_agent_state should say attached"
qm agent "$B" ping > /dev/null 2>&1 && log "(the agent still answers)" || log "the previous signal, 'qm agent ping fails', would have called this closed: it fails here too, with the device attached"
printf '%s\n' 'cont' | qm monitor "$B" > /dev/null 2>&1
for _ in $(seq 1 20); do [ "$(vm_agent_state "$B")" = gone ] && break; sleep 1; done
[ "$(vm_agent_state "$B")" = gone ] && ok "clone $B: once the guest runs again it acknowledges and QEMU reports the device gone (the agent was never stopped inside)" || no "clone $B: the device did not go after cont"

echo "== 6: destroy: nothing of the clones is left"
for id in "$A" "$B"; do
  destroy_vm "$id" && ok "clone $id destroyed" || no "clone $id destroyed"
  vm_exists "$id" && no "$id: config left" || ok "$id: no config"
  [ -z "$(vm_disks "$id")" ] && ok "$id: no disk left on the thin pool" || no "$id: disk left: $(vm_disks "$id")"
  [ ! -e "/sys/class/net/$(net_port "$id")" ] && ok "$id: no tap" || no "$id: tap left"
  [ ! -e "/run/qemu-server/$id.pid" ] && ok "$id: no qemu pid file" || no "$id: pid file left"
done
[ -z "$(net_ports)" ] && ok "no port left on $NET_BRIDGE" || no "ports left: $(net_ports)"
n=$({ nft list table inet "$NET_TABLE"; nft list table netdev "$NET_TABLE_L2"; } | grep -cE 'tap954[0-9]i0" . ')
[ "$n" -eq 0 ] && ok "no pin left in the table" || no "$n pins left in the table"
nft list table inet "$NET_TABLE" | grep -E 'counter packets [0-9]+ bytes [0-9]+ drop' | sed 's/^[[:space:]]*/  rule: /'

echo "== 7: the network comes down: $NET_BRIDGE and the table are gone, and the node is as it was"
if [ "$NET_WAS_UP" -eq 0 ]; then
  net_down && ok "net_down" || no "net_down"
  net_present && no "$NET_BRIDGE is still there" || ok "$NET_BRIDGE is gone"
  nft list table inet "$NET_TABLE" > /dev/null 2>&1 || nft list table netdev "$NET_TABLE_L2" > /dev/null 2>&1 && no "a table is still there" || ok "both tables are gone"
  ip -br link | sed 's/ .*//' | sort > "$W/links.after"
  nft list tables > "$W/tables.after" 2>&1
  bridge link show 2> /dev/null | grep -v 'tap954' | sed 's/ state.*//' | sort > "$W/ports.after"
  diff -q "$W/links.before" "$W/links.after" > /dev/null && ok "ip -br link: the same set of links as before" || { no "links differ"; diff "$W/links.before" "$W/links.after"; }
  diff -q "$W/tables.before" "$W/tables.after" > /dev/null && ok "nft list tables: the same as before ($(wc -l < "$W/tables.after") tables)" || { no "tables differ"; diff "$W/tables.before" "$W/tables.after"; }
  diff -q "$W/ports.before" "$W/ports.after" > /dev/null && ok "every other bridge's ports (vmbr0, vmbr9) are untouched" || { no "bridge ports differ"; diff "$W/ports.before" "$W/ports.after"; }
  sysctls > "$W/sysctl.after"
  diff -q "$W/sysctl.before" "$W/sysctl.after" > /dev/null && ok "the node's forwarding, rp_filter and redirect switches (all, default, uplink) are as before" || { no "the node's sysctls differ"; diff "$W/sysctl.before" "$W/sysctl.after"; }
  [ ! -e "$RUN_DIR/$NET_SAVED_NAME" ] && ok "the forwarding switches were put back" || no "the saved switches were not restored"
else
  log "the network was up before this check: it is left up"
fi
if [ "$fails" -eq 0 ]; then echo "== network proofs: ALL PASS"; else echo "== network proofs: FAIL ($fails)"; fi
[ "$fails" -eq 0 ]
