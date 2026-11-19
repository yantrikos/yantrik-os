#!/bin/bash
# Run inside a CI clone, as root, by check-net.sh (through the guest agent, on stdin of `bash -s`), with
# the in-guest filter NOT loaded: what is refused here is refused by the node's table alone (host/yci.nft).
# Environment: NODE_LAN_IP, NODE_BR_IP, LAN_IP, GW_IP, OWN_IPS (comma list), PEER_IP. No address is
# printed: the labels say what was tried.
# shellcheck disable=SC2016,SC2329
set -u
fail=0
check() { # check <label> <expected: ok|refused> <cmd...>
  local label=$1 want=$2 got
  shift 2
  if "$@" < /dev/null > /dev/null 2>&1; then got=ok; else got=refused; fi
  if [ "$got" = "$want" ]; then echo "PASS  $label ($got)"; else echo "FAIL  $label (wanted $want, got $got)"; fail=1; fi
}
tcp() { timeout 5 bash -c "exec 3<>/dev/tcp/$1/$2"; }
# udp_reply HOST PORT HEX: send a datagram and succeed only if something answers within 4 s.
udp_reply() {
  python3 -c '
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(4)
s.sendto(bytes.fromhex(sys.argv[3]), (sys.argv[1], int(sys.argv[2])))
s.recv(512)' "$@"
}
DNSQ=123401000001000000000000076578616d706c6503636f6d0000010001
NTPQ=1b$(printf '0%.0s' $(seq 1 94))

echo "kernel: $(uname -r)"
check "DNS resolves github.com" ok getent hosts github.com
check "https://github.com over 443" ok curl -m10 -sSfo /dev/null https://github.com
check "index.crates.io over 443" ok curl -m10 -sSfo /dev/null https://index.crates.io/config.json
check "static.crates.io over 443" ok curl -m10 -sSfo /dev/null -r 0-0 https://static.crates.io/crates/libc/libc-0.2.150.crate

check "node's address on the LAN: ssh (22)" refused tcp "$NODE_LAN_IP" 22
check "node's address on the LAN: API (8006)" refused tcp "$NODE_LAN_IP" 8006
check "node's address on yci0: ssh (22)" refused tcp "$NODE_BR_IP" 22
check "node's address on yci0: API (8006)" refused tcp "$NODE_BR_IP" 8006
check "node's address on yci0: ping" refused ping -c1 -W2 "$NODE_BR_IP"
check "node's address on yci0: DNS (53)" refused udp_reply "$NODE_BR_IP" 53 "$DNSQ"
check "a LAN host (the build host): ssh" refused tcp "$LAN_IP" 22
check "a LAN host (the build host): http" refused tcp "$LAN_IP" 80
check "the router: https" refused tcp "$GW_IP" 443
n=0
for ip in ${OWN_IPS//,/ }; do
  n=$((n + 1))
  check "our own public address #$n: 443" refused tcp "$ip" 443
done
[ -z "${PEER_IP:-}" ] || check "another clone's address: 8080" refused tcp "$PEER_IP" 8080
check "IPv6 (github over -6)" refused curl -m5 -6 -s -o /dev/null https://github.com
# (The guest has no IPv6 route, so these fail for want of one; check-net.sh also sends raw IPv6 frames and
# counts them on the node: the table drops them at the port.)
check "IPv6 ping of a public address" refused ping -6 -c1 -W3 2606:4700:4700::1111
gh=$(getent ahostsv4 github.com | awk '{print $1; exit}')
check "github.com port 22 (not 443)" refused tcp "$gh" 22
check "github.com port 8443 (not 443)" refused tcp "$gh" 8443
check "port 80 to the internet" refused curl -m5 -s -o /dev/null http://example.com/
check "ICMP echo to a public address" refused ping -c1 -W3 9.9.9.9
check "DNS to another resolver (1.1.1.1 udp 53)" refused udp_reply 1.1.1.1 53 "$DNSQ"
check "DNS to the one resolver (9.9.9.9 udp 53) works" ok udp_reply 9.9.9.9 53 "$DNSQ"
check "NTP (udp 123) to a public address" refused udp_reply "$(getent ahostsv4 time.cloudflare.com | awk '{print $1; exit}')" 123 "$NTPQ"
exit $fail
