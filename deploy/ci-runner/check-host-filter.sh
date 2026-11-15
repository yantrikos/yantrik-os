#!/usr/bin/env bash
# Prove the host-side filter (#1252 reviews R3, H1, N1, N3). Run as root on the node, with the
# controller stopped and no clone in 9600-9699. Every clone below is started WITHOUT the CT's own
# rules, so everything shown is what CT root can do.
#
# Phase 1, the real bridge (vmbr0), L3 only, nothing spoofed. It needs the clones' fixed address
# (/etc/yantrik-ci/clone-net); without it the phase prints "SKIPPED" and the L2 phase below, which uses
# its own test address, is the whole proof.
#
# Phase 2, L2, on an isolated test bridge (vmbr-yci-test: no uplink, no other port, a dummy "gateway"
# in a netns, a stand-in for the node's address). The clone, as CT root, spoofs the gateway's MAC,
# claims the gateway's and a third host's address with ARP, broadcasts a rogue DHCP offer, and sends
# DHCP with a foreign client id. Each is run once with the filter off (control: the attack works) and
# once with it on. A clone that is not pinned must reach nothing at all. The spoofs cannot reach the
# real LAN: the test bridge has no way out.
#
# The clone, the test bridge, its netns and the nft table are removed at the end.
#
# Usage: check-host-filter.sh [CT-id]   (default 9698)

# shellcheck disable=SC1091,SC2329,SC2015,SC2013,SC2034
set -u
ID=${1:-9698}
LIB_DIR=${YANTRIK_CI_LIB:-/usr/local/lib/yantrik-ci}
OWN_PUBLIC_FILE=${YANTRIK_CI_OWN_PUBLIC:-/etc/yantrik-ci/own-public-ips}
CLONE_NET_FILE=${YANTRIK_CI_CLONE_NET:-/etc/yantrik-ci/clone-net}
RUN_DIR=$(mktemp -d)
ID_MIN=9600 ID_MAX=9699 TEMPLATE_ID=9530 BUILD_CT=530
TB=vmbr-yci-test GWNS=yci-gw GWH=yci-gwh GWN=yci-gwn THNS=yci-3rd THH=yci-3h THN=yci-3n
T_GW=10.77.0.1 T_NODE=10.77.0.2 T_CLONE=10.77.0.50 T_THIRD=10.77.0.3
log() { printf '   %s\n' "$*"; }
. "$LIB_DIR/trust.sh"
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
  ip link del "$THH" > /dev/null 2>&1
  ip netns del "$GWNS" > /dev/null 2>&1
  ip netns del "$THNS" > /dev/null 2>&1
  ip link del "$TB" > /dev/null 2>&1
  rm -rf "$RUN_DIR"
}
trap teardown EXIT

pass() { echo "PASS  $1"; }
bad() { echo "FAIL  $1"; fail=1; }
wait_ct() { for _ in $(seq 1 60); do pct exec "$ID" -- "$@" > /dev/null 2>&1 && return 0; sleep 1; done; return 1; }

# ============================ phase 1: the real bridge, L3 ============================
echo "== phase 1: vmbr0, L3 (nothing is spoofed)"
if [ ! -f "$CLONE_NET_FILE" ]; then
  echo "SKIPPED: no $CLONE_NET_FILE (the clones' fixed address); phase 2 below uses its own test address"
else
  clone_net_load || { echo "FAIL: $CLONE_NET_FILE is not valid"; exit 1; }
  pass "the clone address $CLONE_IP is valid and is not CT $BUILD_CT's"
  if clone_addr_free; then pass "nothing answers for $CLONE_IP on vmbr0 (ARP probe)"; else bad "$CLONE_IP is already in use"; exit 1; fi
  host_filter_load || { echo "FAIL: the host filter did not load"; exit 1; }
  node_ip=${NODE_CIDR%/*}
  net=$(pct config "$BUILD_CT" | grep '^net0:')
  ct530=$(sed -n 's/.*ip=\([0-9.]*\).*/\1/p' <<< "$net")
  gw=$NODE_GW
  pct clone "$TEMPLATE_ID" "$ID" --hostname "yantrik-ci-$ID" > /dev/null || { echo "FAIL: clone"; exit 1; }
  clone_set_net "$ID" && host_filter_add_clone "$ID" || { echo "FAIL: address or pin"; exit 1; }
  pct start "$ID" || { echo "FAIL: start"; exit 1; }
  wait_ct getent hosts github.com || bad "the clone never got a network"
  echo "   CT rules loaded in the clone: $(pct exec "$ID" -- sh -c 'nft list tables | grep -c yantrik_ci || true'); address $CLONE_IP"
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
  for ip in $(grep -vE '^[[:space:]]*(#|$)' "$OWN_PUBLIC_FILE"); do check "own public address $ip :443" refused "https://$ip/"; done
  check "GitHub (public 443)" ok "https://api.github.com/zen"
  check "IPv6 to a public address" refused "https://[2606:4700:4700::1111]/"
  if pct exec "$ID" -- getent hosts index.crates.io > /dev/null 2>&1; then pass "DNS works (ok)"; else bad "DNS"; fi
  echo "== CT 530, read-only: its networking is unchanged"
  if [ "$(pct exec "$BUILD_CT" -- curl -sI -m10 -o /dev/null -w '%{http_code}' https://api.github.com 2> /dev/null)" = 200 ]; then
    pass "CT 530 reaches api.github.com (HTTP 200)"
  else bad "CT 530 cannot reach api.github.com"; fi
  if nft get element bridge yantrik_ci clone_ports "{ veth${BUILD_CT}i0 }" > /dev/null 2>&1; then bad "CT 530's port is pinned"; else pass "CT 530's port is not pinned"; fi
  if nft get element bridge yantrik_ci ci_ports "{ veth${BUILD_CT}i0 }" > /dev/null 2>&1; then bad "CT 530's port is in ci_ports"; else pass "CT 530's port is not in ci_ports (untouched)"; fi
  destroy_ct "$ID" > /dev/null 2>&1
  lxc_exists "$ID" || host_filter_remove_clone "$ID"
fi

# ============================ phase 2: L2, isolated bridge ============================
echo "== phase 2: L2 on the isolated bridge $TB (no uplink)"
ip link add "$TB" type bridge && ip addr add "$T_NODE/24" dev "$TB" && ip link set "$TB" up || { echo "FAIL: test bridge"; exit 1; }
ip netns add "$GWNS" && ip link add "$GWH" type veth peer name "$GWN" && ip link set "$GWN" netns "$GWNS" || { echo "FAIL: test netns"; exit 1; }
ip -n "$GWNS" addr add "$T_GW/24" dev "$GWN" && ip -n "$GWNS" link set "$GWN" up && ip -n "$GWNS" link set lo up
ip link set "$GWH" master "$TB" && ip link set "$GWH" up
ip netns add "$THNS" && ip link add "$THH" type veth peer name "$THN" && ip link set "$THN" netns "$THNS" || { echo "FAIL: third host"; exit 1; }
ip -n "$THNS" addr add "$T_THIRD/24" dev "$THN" && ip -n "$THNS" link set "$THN" up && ip -n "$THNS" link set lo up
ip link set "$THH" master "$TB" && ip link set "$THH" up
THMAC=$(ip -n "$THNS" -o link show "$THN" | sed -n 's/.*link\/ether \([0-9a-f:]*\) .*/\1/p')
GWMAC=$(ip -n "$GWNS" -o link show "$GWN" | sed -n 's/.*link\/ether \([0-9a-f:]*\) .*/\1/p')
echo "   isolated bridge: gateway $T_GW ($GWMAC) and a third host $T_THIRD ($THMAC) in their own netns, node address $T_NODE"

# The test fixed address goes through the same check the controller uses.
printf 'ip=%s/24,gw=%s\n' "$T_CLONE" "$T_GW" > "$RUN_DIR/clone-net"
chmod 644 "$RUN_DIR/clone-net"
clone_net_check "$RUN_DIR/clone-net" "$T_NODE/24" "$T_GW" "$T_GW" && pass "the test clone-net file passes the controller's check ($CLONE_IP/$CLONE_LEN gw $CLONE_GW)" || { bad "clone-net check"; exit 1; }
for bad_line in "ip=10.77.0.60/24,gw=$T_GW" "ip=$T_NODE/24,gw=$T_GW" "ip=$T_GW/24,gw=$T_GW" "ip=10.77.1.5/24,gw=$T_GW" "ip=10.77.0.255/24,gw=$T_GW" "ip=$T_CLONE/24,gw=10.77.0.9" "ip=$T_CLONE/16,gw=$T_GW" "ip=$T_CLONE/24 ,gw=$T_GW" "ip=300.1.1.1/24,gw=$T_GW"; do
  printf '%s\n' "$bad_line" > "$RUN_DIR/bad"; chmod 644 "$RUN_DIR/bad"
  if (clone_net_check "$RUN_DIR/bad" "$T_NODE/24" "$T_GW" "$T_GW" 10.77.0.60 > /dev/null 2>&1); then bad "accepted '$bad_line'"; else :; fi
done
pass "bad clone-net lines are refused (CT 530's address, the node, the gateway, outside the subnet, broadcast, wrong gw, wrong prefix, bad format)"
CLONE_BRIDGE=$TB
if CLONE_IP=$T_CLONE clone_addr_free; then pass "a free address ($T_CLONE) passes the ARP probe"; else bad "the free address was reported in use"; fi
if (CLONE_IP=$T_THIRD clone_addr_free > /dev/null 2>&1); then bad "an address in use ($T_THIRD) passed the ARP probe"; else pass "an address that answers ($T_THIRD, the third host) is refused by the ARP probe"; fi
unset CLONE_BRIDGE
clone_net_check "$RUN_DIR/clone-net" "$T_NODE/24" "$T_GW" "$T_GW" > /dev/null

pct clone "$TEMPLATE_ID" "$ID" --hostname "yantrik-ci-$ID" > /dev/null || { echo "FAIL: clone"; exit 1; }
CMAC=$(pct config "$ID" | sed -n '/^net0:/s/.*hwaddr=\([0-9A-Fa-f:]\{17\}\).*/\1/p')
CLONE_BRIDGE=$TB clone_set_net "$ID" || { echo "FAIL: set net"; exit 1; }
pct start "$ID" || { echo "FAIL: start"; exit 1; }
for _ in $(seq 1 30); do pct exec "$ID" -- ip -4 addr show eth0 2> /dev/null | grep -q "$T_CLONE" && break; sleep 1; done
PORT=$(clone_port "$ID")
echo "   ports on $TB:"
ip -br link show master "$TB" | sed 's/^/     /'
dhcp_cfg=$(pct exec "$ID" -- sh -c 'grep -ciE "^DHCP=(yes|ipv4|true|ipv6)" /etc/systemd/network/eth0.network 2>/dev/null')
dhcp_sock=$(pct exec "$ID" -- sh -c 'ss -uln | grep -c ":68 "')
[ "$dhcp_cfg" = 0 ] && [ "$dhcp_sock" = 0 ] && pass "no DHCP client in the clone (static config: $(pct exec "$ID" -- sh -c 'grep -E "^(Address|Gateway)" /etc/systemd/network/eth0.network | tr "\n" " "'))" || bad "the clone runs or is configured for DHCP (config lines $dhcp_cfg, sockets on :68 $dhcp_sock)"

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
cat > "$RUN_DIR/dhcp.py" <<'PY'
# A DHCP request to everyone. mode "sock": an ordinary socket (source: the clone's address).
# mode "raw": a hand-made frame with source 0.0.0.0 and a FOREIGN client hardware address, as a
# starvation or release attack would send it; the frame's own MAC is the clone's.
import socket, struct, sys
mode, sport, dport = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
if mode == "sock":
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    s.setsockopt(socket.SOL_SOCKET, 25, b'eth0')  # SO_BINDTODEVICE: no route is needed for a broadcast
    s.bind(('0.0.0.0', sport))
    for _ in range(3):
        s.sendto(b'yantrik-l2-proof', ('255.255.255.255', dport))
else:
    own = open('/sys/class/net/eth0/address').read().strip()
    src_mac = bytes.fromhex(own.replace(':', ''))
    chaddr = bytes.fromhex('02deadbeef01') + b'\x00' * 10
    bootp = struct.pack('!BBBBIHH4s4s4s4s', 1, 1, 6, 0, 0x1234, 0, 0x8000, b'\0' * 4, b'\0' * 4, b'\0' * 4, b'\0' * 4) + chaddr + b'\0' * 192 + b'\x63\x82\x53\x63' + b'\x35\x01\x03\xff'
    udp = struct.pack('!HHHH', sport, dport, 8 + len(bootp), 0) + bootp
    ip = struct.pack('!BBHHHBBH4s4s', 0x45, 0, 20 + len(udp), 0x4242, 0, 64, 17, 0, b'\0' * 4, b'\xff' * 4)
    csum = sum(struct.unpack('!10H', ip)); csum = (csum & 0xffff) + (csum >> 16); csum = (csum & 0xffff) + (csum >> 16)
    ip = ip[:10] + struct.pack('!H', ~csum & 0xffff) + ip[12:]
    r = socket.socket(socket.AF_PACKET, socket.SOCK_RAW)
    r.bind(('eth0', 0))
    for _ in range(3):
        r.send(b'\xff' * 6 + src_mac + b'\x08\x00' + ip + udp)
PY
cat > "$RUN_DIR/listen.py" <<'PY'
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
s.bind(('0.0.0.0', int(sys.argv[1])))
s.settimeout(5)
try:
    data, addr = s.recvfrom(1000)
    print('RECEIVED')
except socket.timeout:
    print('NOTHING')
PY
cat > "$RUN_DIR/ipsend.py" <<'PY'
# One IP/UDP frame from the clone to a public address, addressed at ethernet level to a chosen MAC.
import socket, struct, sys
dstmac, dstip, srcip = sys.argv[1], sys.argv[2], sys.argv[3]
own = bytes.fromhex(open('/sys/class/net/eth0/address').read().strip().replace(':', ''))
payload = b'yantrik-l2-dstmac'
udp = struct.pack('!HHHH', 40000, 40000, 8 + len(payload), 0) + payload
ip = struct.pack('!BBHHHBBH4s4s', 0x45, 0, 20 + len(udp), 0x4343, 0, 64, 17, 0, socket.inet_aton(srcip), socket.inet_aton(dstip))
c = sum(struct.unpack('!10H', ip)); c = (c & 0xffff) + (c >> 16); c = (c & 0xffff) + (c >> 16)
ip = ip[:10] + struct.pack('!H', ~c & 0xffff) + ip[12:]
r = socket.socket(socket.AF_PACKET, socket.SOCK_RAW)
r.bind(('eth0', 0))
for _ in range(3):
    r.send(bytes.fromhex(dstmac.replace(':', '')) + own + b'\x08\x00' + ip + udp)
PY
cat > "$RUN_DIR/sniff.py" <<'PY'
import socket, sys, time
r = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0800))
r.bind((sys.argv[1], 0))
r.settimeout(0.5)
end = time.time() + 5
while time.time() < end:
    try:
        if b'yantrik-l2-dstmac' in r.recv(2048):
            print('RECEIVED'); sys.exit()
    except socket.timeout:
        pass
print('NOTHING')
PY
pct push "$ID" "$RUN_DIR/ipsend.py" /root/ipsend.py > /dev/null
pct push "$ID" "$RUN_DIR/arp.py" /root/arp.py > /dev/null
pct push "$ID" "$RUN_DIR/dhcp.py" /root/dhcp.py > /dev/null

fdb_dev() { bridge fdb show br "$TB" | grep -i "$GWMAC" | grep -v permanent | awk '{for (i = 1; i <= NF; i++) if ($i == "dev") {print $(i + 1); exit}}'; }
learn() { ip netns exec "$GWNS" ping -c1 -W1 "$T_NODE" > /dev/null 2>&1; ping -c1 -W1 -I "$TB" "$T_GW" > /dev/null 2>&1; ping -c1 -W1 -I "$TB" "$T_THIRD" > /dev/null 2>&1; }
neigh_mac() { ip neigh show "$1" dev "$TB" | awk '{for (i = 1; i <= NF; i++) if ($i == "lladdr") print $(i + 1)}'; }
listen() { # listen PORT CMD...: the gateway side's view of what the clone sent
  ip netns exec "$GWNS" python3 "$RUN_DIR/listen.py" "$1" > "$RUN_DIR/listen.out" 2>&1 &
  local lp=$!
  sleep 1
  "${@:2}" > /dev/null 2>&1
  wait "$lp"
  cat "$RUN_DIR/listen.out"
}
dstmac_probe() { # dstmac_probe NETNS IFACE DSTMAC: did a frame addressed to DSTMAC (public IP destination) arrive there?
  ip netns exec "$1" python3 "$RUN_DIR/sniff.py" "$2" > "$RUN_DIR/sniff.out" 2>&1 &
  local sp=$!
  sleep 1
  pct exec "$ID" -- python3 /root/ipsend.py "$3" 93.184.216.34 "$T_CLONE" > /dev/null 2>&1
  wait "$sp"
  cat "$RUN_DIR/sniff.out"
}
attacks() { # attacks LABEL: sets MAC_RES RAW_RES SOCK_RES ARP_GW ARP_THIRD
  local label=$1 before
  learn; sleep 1
  # 1. MAC spoof: the clone takes the gateway's MAC and talks.
  pct exec "$ID" -- ip link set dev eth0 address "$GWMAC" > /dev/null 2>&1
  pct exec "$ID" -- ping -c3 -W1 "$T_NODE" > /dev/null 2>&1
  MAC_RES=$(fdb_dev)
  echo "   [$label] bridge fdb: $GWMAC is on port '${MAC_RES:-none}' (the gateway's own port is $GWH, the clone's is $PORT)"
  pct exec "$ID" -- ip link set dev eth0 address "$CMAC" > /dev/null 2>&1
  learn; sleep 1
  # 2. DHCP from the clone, seen from the gateway's netns: a hand-made request with a foreign client
  #    hardware address and source 0.0.0.0, and an ordinary broadcast from the clone's own address.
  RAW_RES=$(listen 67 pct exec "$ID" -- python3 /root/dhcp.py raw 68 67)
  SOCK_RES=$(listen 67 pct exec "$ID" -- python3 /root/dhcp.py sock 68 67)
  echo "   [$label] DHCP request, foreign client id, source 0.0.0.0: the gateway side got $RAW_RES; broadcast from the clone's address: $SOCK_RES"
  # 3. ARP: claim the gateway's address, then a third host's.
  before=$(neigh_mac "$T_GW")
  pct exec "$ID" -- python3 /root/arp.py eth0 "$CMAC" "$T_GW" "$T_NODE" > /dev/null 2>&1
  sleep 1; ARP_GW=$(neigh_mac "$T_GW")
  echo "   [$label] node's neighbour entry for the gateway $T_GW: before $before, after the clone claimed it: $ARP_GW"
  before=$(neigh_mac "$T_THIRD")
  pct exec "$ID" -- python3 /root/arp.py eth0 "$CMAC" "$T_THIRD" "$T_NODE" > /dev/null 2>&1
  sleep 1; ARP_THIRD=$(neigh_mac "$T_THIRD")
  echo "   [$label] node's neighbour entry for a third host $T_THIRD: before $before, after the clone claimed it: $ARP_THIRD"
}
lc() { printf '%s' "${1,,}"; }

echo "-- control: the filter is OFF (the attacks should work)"
nft delete table bridge yantrik_ci > /dev/null 2>&1
attacks control
[ "$MAC_RES" = "$PORT" ] && echo "   control: the MAC spoof moved the gateway's entry to the clone's port" || echo "   control: the MAC spoof did not take (fdb on '${MAC_RES:-none}')"
[ "$RAW_RES" = RECEIVED ] && echo "   control: the foreign-client DHCP request got through" || echo "   control: the foreign DHCP request did not arrive"
[ "$(lc "$ARP_GW")" = "$(lc "$CMAC")" ] && echo "   control: the ARP claim poisoned the gateway's entry" || echo "   control: the gateway ARP claim did not change the entry"
[ "$(lc "$ARP_THIRD")" = "$(lc "$CMAC")" ] && echo "   control: the ARP claim poisoned the third host's entry" || echo "   control: the third-host ARP claim did not change the entry"

echo "   [control] frame to the third host's MAC with a public destination: $(dstmac_probe "$THNS" "$THN" "$THMAC")"

echo "-- filter ON, the clone NOT pinned (a clone the controller has not pinned has no network)"
host_filter_render "$T_GW" "$T_GW" "$T_CLONE" 203.0.113.9 "$GWMAC" || { echo "FAIL: render"; exit 1; }
if (host_filter_render "$T_GW" "$T_GW" "$T_CLONE" 203.0.113.9 "" > /dev/null 2>&1); then bad "the filter loaded without the gateway MAC"; else pass "the filter refuses to load when the gateway MAC is unknown"; fi
host_filter_render "$T_GW" "$T_GW" "$T_CLONE" 203.0.113.9 "$GWMAC" > /dev/null
dns_probe() { ip netns exec "$GWNS" python3 "$RUN_DIR/listen.py" 53 > "$RUN_DIR/listen.out" 2>&1 & local lp=$!; sleep 1
  pct exec "$ID" -- python3 -c "import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); [s.sendto(b'q', ('$T_GW', 53)) for _ in range(3)]" > /dev/null 2>&1; wait "$lp"; cat "$RUN_DIR/listen.out"; }
if [ "$(dns_probe)" = RECEIVED ]; then bad "an unpinned clone's DNS-style packet reached the resolver"; else pass "an unpinned clone reaches nothing (a packet to the resolver, which a pinned clone may send, never arrives)"; fi
if pct exec "$ID" -- ping -c2 -W1 "$T_NODE" > /dev/null 2>&1; then bad "an unpinned clone reached the node"; else pass "an unpinned clone cannot reach the node either"; fi
echo "   drops of unpinned ports: $(nft list chain bridge yantrik_ci prerouting | grep -E 'iifname != @clone_ports' | sed 's/.*counter //')"
echo "   a port outside ci_ports is untouched: CT 530's port in ci_ports? $(nft get element bridge yantrik_ci ci_ports "{ veth530i0 }" > /dev/null 2>&1 && echo yes || echo no)"

echo "-- filter ON, the clone pinned"
host_filter_add_clone "$ID" || { echo "FAIL: pin"; exit 1; }
learn
pct exec "$ID" -- ping -c1 -W1 "$T_GW" > /dev/null 2>&1   # resolves the gateway's MAC (the ping itself is dropped: the gateway is private)
nb=$(pct exec "$ID" -- ip neigh show "$T_GW" dev eth0 | awk '{for (i = 1; i <= NF; i++) if ($i == "lladdr") print $(i + 1)}')
[ "$(lc "$nb")" = "$(lc "$GWMAC")" ] && pass "a pinned clone, with its own MAC and address, resolves the gateway by ARP ($nb)" || bad "a pinned clone cannot ARP for the gateway (neighbour '${nb:-none}')"
[ "$(dns_probe)" = RECEIVED ] && pass "a pinned clone's packet to its resolver (UDP 53) arrives" || bad "a pinned clone cannot send DNS to the resolver"
attacks filtered
[ "$MAC_RES" = "$GWH" ] && pass "MAC spoof of the gateway: bridge fdb still shows the uplink port $GWH" || bad "MAC spoof: fdb shows '${MAC_RES:-none}'"
[ "$RAW_RES" = NOTHING ] && pass "DHCP with a foreign client id from source 0.0.0.0 is dropped" || bad "the foreign DHCP request got through"
[ "$SOCK_RES" = NOTHING ] && pass "a broadcast from the clone's own address to port 67 is dropped" || bad "the broadcast got through"
[ "$(lc "$ARP_GW")" = "$(lc "$GWMAC")" ] && pass "ARP claiming the gateway: the node's neighbour entry keeps the gateway's MAC" || bad "ARP claim changed the gateway's entry to '${ARP_GW:-none}'"
[ "$(lc "$ARP_THIRD")" = "$(lc "$THMAC")" ] && pass "ARP claiming a third host ($T_THIRD): the node's neighbour entry is unchanged" || bad "ARP claim changed the third host's entry to '${ARP_THIRD:-none}'"
[ "$(dstmac_probe "$THNS" "$THN" "$THMAC")" = NOTHING ] && pass "a frame to the third host's MAC with a public destination is dropped" || bad "a frame to the third host's MAC got through"
[ "$(dstmac_probe "$GWNS" "$GWN" "$GWMAC")" = RECEIVED ] && pass "a frame to the gateway's MAC with a public destination passes" || bad "a frame to the gateway's MAC was dropped"
echo "   drop counters (prerouting, forward):"
nft list chain bridge yantrik_ci prerouting | grep -E 'counter packets' | sed 's/^/     /'
nft list chain bridge yantrik_ci forward | grep -E 'counter packets' | sed 's/^/     /'

if [ "$fail" -eq 0 ]; then echo "== host filter: ALL PASS"; else echo "== host filter: FAILURES"; fi
exit "$fail"
