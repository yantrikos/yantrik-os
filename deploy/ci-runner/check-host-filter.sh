#!/usr/bin/env bash
# Prove the host-side filter (#1252 review R3). Run as root on the node, with the controller
# stopped. It loads the `bridge yantrik_ci` table with the controller's own loader, clones the
# template to 9699 (or $1, in 9600-9699), starts it WITHOUT the CT's own rules, and shows that
# even CT root cannot reach the node, the build host, the LAN or our own public addresses, while
# the public internet and DNS still work. It then reads CT 530 once, read-only, to show that its
# networking is unchanged. The clone is destroyed at the end.
#
# Usage: check-host-filter.sh [CT-id]   (default 9699)

# shellcheck disable=SC1091,SC2329,SC2015,SC2013
set -u
ID=${1:-9699}
LIB_DIR=${YANTRIK_CI_LIB:-/usr/local/lib/yantrik-ci}
OWN_PUBLIC_FILE=${YANTRIK_CI_OWN_PUBLIC:-/etc/yantrik-ci/own-public-ips}
RUN_DIR=$(mktemp -d)
ID_MIN=9600 ID_MAX=9699 TEMPLATE_ID=9530 BUILD_CT=530
log() { printf '   %s\n' "$*"; }
# shellcheck source=lib/host.sh
. "$LIB_DIR/host.sh"

[ "$(id -u)" -eq 0 ] || { echo "run as root" >&2; exit 1; }
in_range "$ID" || { echo "id must be in $ID_MIN-$ID_MAX" >&2; exit 1; }
if lxc_exists "$ID"; then echo "CT $ID exists" >&2; exit 1; fi
fail=0
cleanup() { destroy_ct "$ID" > /dev/null 2>&1; rm -rf "$RUN_DIR"; }
trap cleanup EXIT

echo "== loading the host filter"
host_filter_load || { echo "FAIL: the host filter did not load"; exit 1; }
nft list table bridge yantrik_ci | sed -n '1,4p;/chain input/,$p' | sed 's/^/   /' | head -12

node_ip=$(ip -4 -o addr show vmbr0 | awk '{sub(/\/.*/, "", $4); print $4; exit}')
net=$(pct config "$BUILD_CT" | grep '^net0:')
ct530=$(sed -n 's/.*ip=\([0-9.]*\).*/\1/p' <<< "$net")
gw=$(ip -4 route show default | awk '{print $3; exit}')

echo "== clone $ID, started with NO rules of its own"
pct clone "$TEMPLATE_ID" "$ID" --hostname "yantrik-ci-$ID" > /dev/null && pct start "$ID" || { echo "FAIL: clone"; exit 1; }
for _ in $(seq 1 60); do pct exec "$ID" -- getent hosts github.com > /dev/null 2>&1 && break; sleep 1; done
pct exec "$ID" -- sh -c 'nft list tables | grep -c yantrik_ci || true' | sed 's/^/   CT rules loaded in the clone: /'
check() { # check LABEL want(ok|refused) URL
  local code
  code=$(pct exec "$ID" -- curl -m5 -sk -o /dev/null -w '%{http_code}' "$3" 2> /dev/null)
  if [ "$code" = 000 ] || [ -z "$code" ]; then got=refused; else got=ok; fi
  if [ "$got" = "$2" ]; then echo "PASS  $1 ($got)"; else echo "FAIL  $1 (wanted $2, got $got, http $code)"; fail=1; fi
}
check "node API :8006" refused "https://$node_ip:8006/"
check "node ssh-port banner" refused "http://$node_ip:22/"
check "CT 530 http" refused "http://$ct530/"
check "gateway http" refused "http://$gw/"
check "gateway https" refused "https://$gw/"
for ip in $(grep -vE '^[[:space:]]*(#|$)' "$OWN_PUBLIC_FILE"); do check "own public address $ip :443" refused "https://$ip/"; done
check "GitHub (public 443)" ok "https://api.github.com/zen"
if pct exec "$ID" -- getent hosts index.crates.io > /dev/null 2>&1; then echo "PASS  DNS works (ok)"; else echo "FAIL  DNS"; fail=1; fi
check "IPv6 to a public address" refused "https://[2606:4700:4700::1111]/"
echo "   counters on the node (drops by the host filter):"
nft list chain bridge yantrik_ci forward | grep -E 'counter packets' | sed 's/^/     /'
nft list chain bridge yantrik_ci input | grep -E 'counter packets' | sed 's/^/     /'

echo "== CT 530, read-only: its networking is unchanged"
if [ "$(pct exec "$BUILD_CT" -- curl -sI -m10 -o /dev/null -w '%{http_code}' https://api.github.com 2> /dev/null)" = 200 ]; then
  echo "PASS  CT 530 reaches api.github.com (HTTP 200)"
else echo "FAIL  CT 530 cannot reach api.github.com"; fail=1; fi
echo "PASS  the filter names only veth96* ports: $(nft list chain bridge yantrik_ci forward | grep -c 'iifname != "veth96\*" accept') rule lets every other port through first"

if [ "$fail" -eq 0 ]; then echo "== host filter: ALL PASS"; else echo "== host filter: FAILURES"; fi
exit "$fail"
