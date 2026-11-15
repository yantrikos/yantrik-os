#!/usr/bin/env bash
# Prove the host-side filter (#1252 reviews R3, H1). Run as root on the node, with the controller
# stopped and no clone in 9600-9699. Two phases, both with a clone started WITHOUT the CT's own
# rules, so everything below is what CT root can do:
#
# 1. The real bridge (vmbr0), L3 only: the node's API, CT 530, the gateway and our own public
#    addresses are refused; the public internet and DNS work; CT 530 is read once, read-only.
#    Nothing here sends a spoofed frame.
# 2. L2, on an isolated test bridge (vmbr-yci-test: no uplink, no other port, a dummy "gateway" in a
#    netns, a stand-in for the node's own address). The clone spoofs the gateway's MAC, claims the
#    gateway's IP with ARP, and broadcasts a rogue DHCP offer. Each is run once with the filter off
#    (control: the attack works) and once with it on (the attack is dropped). The spoofs never touch
#    the real LAN: they cannot, the test bridge has no way out.
#
# The clone and the test bridge are removed at the end, and so is the nft table.
#
# Usage: check-host-filter.sh [CT-id]   (default 9698)

# shellcheck disable=SC1091,SC2329,SC2015,SC2013
set -u
ID=${1:-9698}
LIB_DIR=${YANTRIK_CI_LIB:-/usr/local/lib/yantrik-ci}
OWN_PUBLIC_FILE=${YANTRIK_CI_OWN_PUBLIC:-/etc/yantrik-ci/own-public-ips}
RUN_DIR=$(mktemp -d)
ID_MIN=9600 ID_MAX=9699 TEMPLATE_ID=9530 BUILD_CT=530 POOL=pve/data
TB=vmbr-yci-test GWNS=yci-gw GWH=yci-gwh GWN=yci-gwn
T_GW=10.77.0.1 T_NODE=10.77.0.2 T_CLONE=10.77.0.50
log() { printf '   %s\n' "$*"; }
trusted_path() { return 0; } # an operator's proof tool: the controller checks these at run time
# shellcheck source=lib/host.sh
. "$LIB_DIR/host.sh"

[ "$(id -u)" -eq 0 ] || { echo "run as root" >&2; exit 1; }
in_range "$ID" || { echo "id must be in $ID_MIN-$ID_MAX" >&2; exit 1; }
if ls /etc/pve/nodes/*/lxc/96??.conf > /dev/null 2>&1; then echo "a CT exists in $ID_MIN-$ID_MAX" >&2; exit 1; fi
if ip route | grep -q '^10\.77\.0\.'; then echo "10.77.0.0/24 is in use on this node" >&2; exit 1; fi
fail=0

teardown() {
  destroy_ct "$ID" > /dev/null 2>&1
  nft delete table bridge yantrik_ci > /dev/null 2>&1
  ip link del "$GWH" > /dev/null 2>&1
  ip netns del "$GWNS" > /dev/null 2>&1
  ip link del "$TB" > /dev/null 2>&1
  rm -rf "$RUN_DIR"
}
trap teardown EXIT

pass() { echo "PASS  $1"; }
bad() { echo "FAIL  $1"; fail=1; }
wait_ct() { for _ in $(seq 1 60); do pct exec "$ID" -- "$@" > /dev/null 2>&1 && return 0; sleep 1; done; return 1; }

# ============================ phase 1: the real bridge, L3 ============================
echo "== phase 1: vmbr0, L3 (nothing is spoofed)"
host_filter_load || { echo "FAIL: the host filter did not load"; exit 1; }
node_ip=$(ip -4 -o addr show vmbr0 | awk '{sub(/\/.*/, "", $4); print $4; exit}')
net=$(pct config "$BUILD_CT" | grep '^net0:')
ct530=$(sed -n 's/.*ip=\([0-9.]*\).*/\1/p' <<< "$net")
gw=$(ip -4 route show default | awk '{print $3; exit}')
pct clone "$TEMPLATE_ID" "$ID" --hostname "yantrik-ci-$ID" > /dev/null || { echo "FAIL: clone"; exit 1; }
host_filter_add_clone "$ID" || { echo "FAIL: pin"; exit 1; }
pct start "$ID" || { echo "FAIL: start"; exit 1; }
wait_ct getent hosts github.com || bad "the clone never got a network"
echo "   CT rules loaded in the clone: $(pct exec "$ID" -- sh -c 'nft list tables | grep -c yantrik_ci || true')"
check() { # check LABEL want(ok|refused) URL
  local code got
  code=$(pct exec "$ID" -- curl -m5 -sk -o /dev/null -w '%{http_code}' "$3" 2> /dev/null)
  if [ "$code" = 000 ] || [ -z "$code" ]; then got=refused; else got=ok; fi
  if [ "$got" = "$2" ]; then pass "$1 ($got)"; else bad "$1 (wanted $2, got $got, http $code)"; fi
}
check "node API :8006" refused "https://$node_ip:8006/"
check "node, any port" refused "http://$node_ip:22/"
check "CT 530 http" refused "http://$ct530/"
check "gateway http" refused "http://$gw/"
check "gateway https" refused "https://$gw/"
for ip in $(grep -vE '^[[:space:]]*(#|$)' "$OWN_PUBLIC_FILE"); do check "own public address $ip :443" refused "https://$ip/"; done
check "GitHub (public 443)" ok "https://api.github.com/zen"
check "IPv6 to a public address" refused "https://[2606:4700:4700::1111]/"
if pct exec "$ID" -- getent hosts index.crates.io > /dev/null 2>&1; then pass "DNS works (ok)"; else bad "DNS"; fi
echo "   counters on the node:"
nft list chain bridge yantrik_ci forward | grep -E 'counter packets' | sed 's/^/     /'
nft list chain bridge yantrik_ci input | grep -E 'counter packets' | sed 's/^/     /'
echo "== CT 530, read-only: its networking is unchanged"
if [ "$(pct exec "$BUILD_CT" -- curl -sI -m10 -o /dev/null -w '%{http_code}' https://api.github.com 2> /dev/null)" = 200 ]; then
  pass "CT 530 reaches api.github.com (HTTP 200)"
else bad "CT 530 cannot reach api.github.com"; fi
if nft get element bridge yantrik_ci clone_ports "{ veth${BUILD_CT}i0 }" > /dev/null 2>&1; then bad "CT 530's port is in the filter"; else pass "CT 530's port is not in the filter's port set"; fi
destroy_ct "$ID" > /dev/null 2>&1
host_filter_remove_clone "$ID"

# ============================ phase 2: L2, isolated bridge ============================
echo "== phase 2: L2 on the isolated bridge $TB (no uplink)"
ip link add "$TB" type bridge && ip addr add "$T_NODE/24" dev "$TB" && ip link set "$TB" up || { echo "FAIL: test bridge"; exit 1; }
ip netns add "$GWNS" && ip link add "$GWH" type veth peer name "$GWN" && ip link set "$GWN" netns "$GWNS" || { echo "FAIL: test netns"; exit 1; }
ip -n "$GWNS" addr add "$T_GW/24" dev "$GWN" && ip -n "$GWNS" link set "$GWN" up && ip -n "$GWNS" link set lo up
ip link set "$GWH" master "$TB" && ip link set "$GWH" up
GWMAC=$(ip -n "$GWNS" -o link show "$GWN" | sed -n 's/.*link\/ether \([0-9a-f:]*\) .*/\1/p')
echo "   isolated bridge: gateway $T_GW ($GWMAC) in netns $GWNS, node address $T_NODE; ports: $(bridge link show master "$TB" | wc -l)"

pct clone "$TEMPLATE_ID" "$ID" --hostname "yantrik-ci-$ID" > /dev/null || { echo "FAIL: clone"; exit 1; }
CMAC=$(pct config "$ID" | sed -n '/^net0:/s/.*hwaddr=\([0-9A-Fa-f:]\{17\}\).*/\1/p')
pct set "$ID" --net0 "name=eth0,bridge=$TB,hwaddr=$CMAC,ip=$T_CLONE/24" > /dev/null
pct start "$ID" || { echo "FAIL: start"; exit 1; }
wait_ct ip -4 addr show eth0 || bad "no address in the clone"
for _ in $(seq 1 30); do pct exec "$ID" -- ip -4 addr show eth0 | grep -q "$T_CLONE" && break; sleep 1; done
PORT=$(clone_port "$ID")
bridge link show master "$TB" | awk '{print "   port:", $2}' | tr -d ':' | sed 's/^/ /'

cat > "$RUN_DIR/arp.py" <<'PY'
import socket, struct, sys
iface, mac, spa, tpa = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
m = bytes.fromhex(mac.replace(':', ''))
s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW)
s.bind((iface, 0))
def frame(op, dst, tha):
    arp = struct.pack('!HHBBH6s4s6s4s', 1, 0x0800, 6, 4, op, m, socket.inet_aton(spa), tha, socket.inet_aton(tpa))
    return dst + m + b'\x08\x06' + arp
for _ in range(3):
    s.send(frame(1, b'\xff' * 6, b'\x00' * 6))   # gratuitous request: "spa is at mac"
    s.send(frame(2, b'\xff' * 6, b'\xff' * 6))   # unsolicited reply
PY
cat > "$RUN_DIR/udp.py" <<'PY'
import socket, sys
sport, dport = int(sys.argv[1]), int(sys.argv[2])
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
s.setsockopt(socket.SOL_SOCKET, 25, b'eth0')  # SO_BINDTODEVICE: the clone has no route for a broadcast
s.bind(('0.0.0.0', sport))
for _ in range(3):
    s.sendto(b'yantrik-l2-proof', ('255.255.255.255', dport))
PY
cat > "$RUN_DIR/listen.py" <<'PY'
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
s.bind(('0.0.0.0', int(sys.argv[1])))
s.settimeout(5)
try:
    data, addr = s.recvfrom(100)
    print('RECEIVED', addr[0])
except socket.timeout:
    print('NOTHING')
PY
pct push "$ID" "$RUN_DIR/arp.py" /root/arp.py > /dev/null
pct push "$ID" "$RUN_DIR/udp.py" /root/udp.py > /dev/null

fdb_dev() { bridge fdb show br "$TB" | grep -i "$GWMAC" | grep -v permanent | awk '{for (i = 1; i <= NF; i++) if ($i == "dev") {print $(i + 1); exit}}'; }
gw_relearn() { ip netns exec "$GWNS" ping -c1 -W1 "$T_NODE" > /dev/null 2>&1; ping -c1 -W1 -I "$TB" "$T_GW" > /dev/null 2>&1; }
neigh_mac() { ip neigh show "$T_GW" dev "$TB" | awk '{for (i = 1; i <= NF; i++) if ($i == "lladdr") print $(i + 1)}'; }
listen() { # listen PORT -> RECEIVED/NOTHING, with the clone sending after the listener is up
  ip netns exec "$GWNS" python3 "$RUN_DIR/listen.py" "$1" > "$RUN_DIR/listen.out" 2>&1 &
  local lp=$!
  sleep 1
  "${@:2}"
  wait "$lp"
  cat "$RUN_DIR/listen.out"
}
attacks() { # attacks LABEL: the three attacks once; sets MAC_RES BC_RES ARP_RES
  local label=$1 out
  gw_relearn; sleep 1
  # 1. MAC spoof: the clone takes the gateway's MAC and talks.
  pct exec "$ID" -- ip link set dev eth0 address "$GWMAC" > /dev/null 2>&1
  pct exec "$ID" -- ping -c3 -W1 "$T_NODE" > /dev/null 2>&1
  MAC_RES=$(fdb_dev)
  echo "   [$label] bridge fdb: $GWMAC is on port '${MAC_RES:-none}' (the gateway's own port is $GWH, the clone's is $PORT)"
  pct exec "$ID" -- ip link set dev eth0 address "$CMAC" > /dev/null 2>&1
  gw_relearn; sleep 1
  # 2. A rogue DHCP offer to everyone (UDP 67 -> 68 to 255.255.255.255), seen from the gateway's netns.
  out=$(listen 68 pct exec "$ID" -- python3 /root/udp.py 67 68)
  BC_RES=${out%% *}
  echo "   [$label] rogue DHCP offer to 255.255.255.255:68, the gateway side: $out"
  # 2b. A DHCP client's own request (68 -> 67) must still get through, rate-limited.
  out=$(listen 67 pct exec "$ID" -- python3 /root/udp.py 68 67)
  BC_OK=${out%% *}
  echo "   [$label] DHCP client request 255.255.255.255:68->67, the gateway side: $out"
  # 3. ARP: claim the gateway's address.
  ping -c1 -W1 -I "$TB" "$T_GW" > /dev/null 2>&1
  local before after
  before=$(neigh_mac)
  pct exec "$ID" -- python3 /root/arp.py eth0 "$CMAC" "$T_GW" "$T_NODE" > /dev/null 2>&1
  sleep 1
  after=$(neigh_mac)
  ARP_RES=$after
  echo "   [$label] node's neighbour entry for $T_GW: before $before, after claiming it from the clone: $after"
}

echo "-- control: the filter is OFF (the attacks should work)"
nft delete table bridge yantrik_ci > /dev/null 2>&1
attacks control
c_mac=$MAC_RES c_bc=$BC_RES c_arp=$ARP_RES
[ "$c_mac" = "$PORT" ] && echo "   control: the MAC spoof moved the gateway's entry to the clone's port" || echo "   control: the MAC spoof did not take (fdb on '${c_mac:-none}')"
[ "$c_bc" = RECEIVED ] && echo "   control: the rogue broadcast got through" || echo "   control: the rogue broadcast did not arrive"
[ "${c_arp,,}" = "${CMAC,,}" ] && echo "   control: the ARP claim poisoned the node's neighbour entry" || echo "   control: the ARP claim did not change the entry (${c_arp:-none})"

echo "-- with the filter ON"
host_filter_render "$T_GW" "$T_GW" "$T_NODE" 203.0.113.9 || { echo "FAIL: render"; exit 1; }
host_filter_add_clone "$ID" || { echo "FAIL: pin"; exit 1; }
attacks filtered
[ "$MAC_RES" = "$GWH" ] && pass "MAC spoof of the gateway: bridge fdb still shows the uplink port $GWH" || bad "MAC spoof: fdb shows '${MAC_RES:-none}'"
[ "$BC_RES" = NOTHING ] && pass "broadcast to 255.255.255.255:68 (rogue DHCP offer) is dropped" || bad "the rogue broadcast got through"
[ "$BC_OK" = RECEIVED ] && pass "a DHCP client request (68 -> 67) still reaches the gateway" || bad "DHCP client request blocked"
[ "$ARP_RES" = "$GWMAC" ] && pass "ARP claiming the gateway: node's neighbour entry still has the gateway's MAC" || bad "ARP claim changed the node's entry to '${ARP_RES:-none}'"
echo "   drop counters (prerouting, forward):"
nft list chain bridge yantrik_ci prerouting | grep -E 'counter packets' | sed 's/^/     /'
nft list chain bridge yantrik_ci forward | grep -E 'counter packets' | sed 's/^/     /'

if [ "$fail" -eq 0 ]; then echo "== host filter: ALL PASS"; else echo "== host filter: FAILURES"; fi
exit "$fail"
