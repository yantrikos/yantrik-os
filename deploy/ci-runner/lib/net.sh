# The clones' network on the node (#1252, VM design): the bridge `yci0` (no physical port), the nftables
# tables `inet yantrik_ci` (IP) and `netdev yantrik_ci_l2` (ARP and every other frame type, #1303), the
# forwarding switches, and the per-clone pins. All of it is made
# at runtime by net_up and removed by net_down; no network file is edited and no existing bridge,
# table or firewall is touched. Sourced by the controller, make-template.sh and yantrik-ci-net, which
# define log(), RUN_DIR, LIB_DIR and OWN_PUBLIC_FILE. See host/yci.nft for the rules.
# shellcheck shell=bash disable=SC2034,SC2015

NET_BRIDGE=yci0
NET_TABLE=yantrik_ci
NET_TABLE_L2=yantrik_ci_l2
NET_PREFIX=10.95.30     # the clones' subnet is $NET_PREFIX.0/24, the node is .1
NET_LEN=24
NET_GW=$NET_PREFIX.1
NET_MAC=02:59:43:49:30:01
NET_RESOLVER=${YANTRIK_CI_RESOLVER:-9.9.9.9}
NET_ID_MIN=9540         # the template (9540) and its clones (9541-9549): every id with a pin
NET_ID_MAX=9549
NET_SAVED_NAME=net.saved
NET_NFT=${NET_NFT:-$LIB_DIR/yci.nft} # the rules files (make-template.sh points them at its own copies)
NET_NFT_L2=${NET_NFT_L2:-$LIB_DIR/yci-l2.nft}

net_ip() { printf '%s.%s\n' "$NET_PREFIX" "$(($1 - 9500))"; }
net_port() { printf 'tap%si0\n' "$1"; }
net_in_range() { [[ $1 =~ ^[0-9]+$ ]] && [ "$1" -ge "$NET_ID_MIN" ] && [ "$1" -le "$NET_ID_MAX" ]; }
net_uplink() { ip -4 route show default | awk '{for (i = 1; i < NF; i++) if ($i == "dev") {print $(i + 1); exit}}'; }
net_present() { [ -d "/sys/class/net/$NET_BRIDGE" ]; }

# net_ports: the ports on the bridge, one per line.
net_ports() { ls "/sys/class/net/$NET_BRIDGE/brif" 2> /dev/null; }

# net_is_ours: the bridge is the one this code made (our MAC, our address, only our ports).
net_is_ours() {
  local p
  [ "$(cat "/sys/class/net/$NET_BRIDGE/address" 2> /dev/null)" = "$NET_MAC" ] || return 1
  ip -4 -o addr show dev "$NET_BRIDGE" | grep -q " $NET_GW/$NET_LEN " || return 1
  for p in $(net_ports); do [[ $p =~ ^tap954[0-9]i0$ ]] || return 1; done
}

# net_own: our own public addresses (a root-only file, one per line; each must be an IPv4 address) into
# NET_OWN, comma separated. Not printed: it is called without a subshell so that its refusals reach the log.
NET_OWN=
net_own() {
  local line out=""
  NET_OWN=
  trusted_path "$(dirname "$OWN_PUBLIC_FILE")" && trusted_path "$OWN_PUBLIC_FILE" || return 1
  while IFS= read -r line; do
    line=${line//[[:space:]]/}
    case $line in '' | '#'*) continue ;; esac
    [[ $line =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]] || { log "REFUSING $OWN_PUBLIC_FILE: '$line' is not an IPv4 address"; return 1; }
    out+="${out:+, }$line"
  done < "$OWN_PUBLIC_FILE"
  [ -n "$out" ] || { log "$OWN_PUBLIC_FILE lists no address"; return 1; }
  NET_OWN=$out
}

# sysctl switches (by /proc/sys path): the old values are kept in $RUN_DIR/net.saved (the first time
# only, so a crash and a second net_up keep the real originals) and put back by net_down.
net_sysctl() {
  local f=$1 val=$2 saved=$RUN_DIR/$NET_SAVED_NAME
  [ -w "$f" ] || { log "cannot set $f"; return 1; }
  grep -q "^$f " "$saved" 2> /dev/null || printf '%s %s
' "$f" "$(cat "$f")" >> "$saved"
  printf '%s' "$val" > "$f"
}

net_restore_sysctl() {
  local saved=$RUN_DIR/$NET_SAVED_NAME f val
  [ -f "$saved" ] || return 0
  while read -r f val; do
    [ -w "$f" ] && printf '%s' "$val" > "$f"
  done < "$saved"
  rm -f "$saved"
}

# net_up: make the bridge and load the tables. Idempotent: an existing bridge of ours is reused and
# both tables are replaced in one `nft -f`. Refuses a bridge of that name that is not ours.
net_up() {
  local up own pre ports="" i
  up=$(net_uplink)
  [ -n "$up" ] && [ "$up" != "$NET_BRIDGE" ] || { log "net: no uplink (default route device)"; return 1; }
  net_own || return 1
  own=$NET_OWN
  for pre in "$NET_NFT" "$NET_NFT_L2"; do
    [ -f "$pre" ] && trusted_path "$pre" || { log "net: $pre is missing or not root's"; return 1; }
  done
  if net_present; then
    net_is_ours || { log "REFUSING: a bridge $NET_BRIDGE exists and is not ours"; return 1; }
  else
    ip link add "$NET_BRIDGE" type bridge stp_state 0 forward_delay 0 &&
      ip link set "$NET_BRIDGE" address "$NET_MAC" &&
      ip addr add "$NET_GW/$NET_LEN" dev "$NET_BRIDGE" &&
      ip link set "$NET_BRIDGE" up || { log "net: cannot make $NET_BRIDGE"; net_present && ip link del "$NET_BRIDGE"; return 1; }
  fi
  # L6: IPv4 forwarding is decided by the INPUT device's switch, so turning it on for the uplink would make the
  # node route whatever arrives there to any interface (`iifname != yci0 accept` lets it through). On node2 the
  # switch is already 1 and is left as it is; where it is 0 this refuses rather than change what the node routes.
  [ "$(cat "/proc/sys/net/ipv4/conf/$up/forwarding" 2> /dev/null)" = 1 ] ||
    { log "net: forwarding is off on $up: refusing to turn it on (it would route everything arriving there)"; return 1; }
  # Forwarding for the bridge and the uplink only (the node's global ip_forward is left alone), strict
  # reverse-path checking, no redirects or IPv6 on the bridge, and the node answers ARP on it only for the
  # address it was asked on (arp_ignore), announces only that one, and takes no gratuitous ARP (#1303, M1).
  net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/forwarding" 1 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$up/forwarding" 1 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/rp_filter" 1 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/send_redirects" 0 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/accept_redirects" 0 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/arp_ignore" 1 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/arp_announce" 2 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/drop_gratuitous_arp" 1 &&
    net_sysctl "/proc/sys/net/ipv6/conf/$NET_BRIDGE/disable_ipv6" 1 || { log "net: could not set the forwarding switches"; return 1; }
  for ((i = NET_ID_MIN; i <= NET_ID_MAX; i++)); do ports+="${ports:+, }$(net_port "$i")"; done
  pre=$(mktemp "$RUN_DIR/net.XXXXXX")
  {
    printf 'define ciif = "%s"\n' "$NET_BRIDGE"
    printf 'define ciports = { %s }\n' "$ports"
    printf 'define bridgemac = %s\n' "$NET_MAC"
    printf 'define gateway = %s\n' "$NET_GW"
    printf 'define subnet = %s.0/%s\n' "$NET_PREFIX" "$NET_LEN"
    printf 'define resolver = %s\n' "$NET_RESOLVER"
    printf 'define ownpub = { %s }\n' "$own"
    printf 'include "%s"\n' "$NET_NFT" "$NET_NFT_L2"
  } > "$pre"
  if ! nft -f "$pre" 2> "$pre.err"; then
    log "net: nft refused the ruleset: $(head -c 300 "$pre.err" | tr '\n' ' ')"
    rm -f "$pre" "$pre.err"
    return 1
  fi
  rm -f "$pre" "$pre.err"
  net_ok
}

# net_down: remove the tables and the bridge and put the switches back. Refuses while a clone's port is
# still on the bridge (a clone left running would be unfiltered).
net_down() {
  if net_present; then
    net_is_ours || { log "REFUSING: $NET_BRIDGE is not ours: leaving it"; return 1; }
    if [ -n "$(net_ports)" ]; then log "REFUSING to remove $NET_BRIDGE: $(net_ports | paste -sd' ' -) still on it"; return 1; fi
  fi
  nft delete table inet "$NET_TABLE" 2> /dev/null || true
  nft delete table netdev "$NET_TABLE_L2" 2> /dev/null || true
  if net_present; then ip link del "$NET_BRIDGE" || { log "net: could not delete $NET_BRIDGE"; return 1; }; fi
  net_restore_sysctl
}

# net_pin ID: pin the clone's port with its own MAC and address (from `qm config`) in both tables, in ONE
# nft transaction (the two can never diverge). Run after the clone is configured and before `qm start`: the
# tap appears at start, so there is no gap. Fails closed.
net_pin() {
  local id=$1 mac port ip
  net_in_range "$id" || { log "net: refusing to pin $id"; return 1; }
  mac=$(vm_mac "$id")
  [[ $mac =~ ^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$ ]] || { log "VM $id: no MAC in its net0"; return 1; }
  mac=${mac,,}
  port=$(net_port "$id") ip=$(net_ip "$id")
  if ! net_pin_txn add "$port" "$mac" "$ip" 2> /dev/null; then
    log "net: cannot pin $port to $mac $ip"
    return 1
  fi
  net_ok "$id"
}

# net_pin_txn add|destroy PORT MAC IP: the four elements of one clone, in both tables, as one transaction.
net_pin_txn() {
  local verb=$1 port=$2 mac=$3 ip=$4
  nft -f - << EOF
$verb element inet $NET_TABLE clone_ifmac { $port . $mac }
$verb element inet $NET_TABLE clone_pin { $port . $mac . $ip }
$verb element netdev $NET_TABLE_L2 clone_ifmac { $port . $mac }
$verb element netdev $NET_TABLE_L2 clone_arp { $port . $mac . $mac . $ip }
EOF
}

# net_lock ID: after `qm start` (the port exists only from then on), make the clone's port on the bridge
# isolated (it can talk to the node's side of the bridge and to no other port: no clone to clone, for any
# frame type; the netdev table filters what the node itself may be sent) and locked with one static entry, its own MAC (a frame from any other
# source MAC is dropped by the bridge itself and nothing is learned, so the forwarding table cannot be
# flooded or poisoned). Run before the job's config is handed over; the guest is the pristine template until
# then. Fails closed: the caller destroys the VM if this fails.
net_lock() {
  local id=$1 port mac info
  port=$(net_port "$id")
  mac=$(vm_mac "$id" | tr 'A-F' 'a-f')
  [ -d "/sys/class/net/$port" ] || { log "net: $port does not exist"; return 1; }
  if ! { bridge fdb replace "$mac" dev "$port" master static > /dev/null 2>&1 &&
    ip link set dev "$port" type bridge_slave isolated on > /dev/null 2>&1 &&
    ip link set dev "$port" type bridge_slave locked on > /dev/null 2>&1 &&
    ip link set dev "$port" type bridge_slave learning off > /dev/null 2>&1; }; then
    log "net: cannot isolate and lock $port"
    return 1
  fi
  info=$(bridge -d link show dev "$port" 2> /dev/null)
  grep -q 'isolated on' <<< "$info" && grep -q 'locked on' <<< "$info" && grep -q 'learning off' <<< "$info" ||
    { log "net: $port is not isolated and locked"; return 1; }
}

# net_unpin ID: drop a clone's pins from both tables in one transaction (the VM is gone, or given up on: an
# unpinned port is dropped). `destroy` does not fail on an element that is not there. If the transaction fails
# (one table was flushed away) each table is cleaned on its own.
net_unpin() {
  local id=$1 port mac ip t
  port=$(net_port "$id") ip=$(net_ip "$id")
  # The MAC is read from whichever table still has it.
  mac=$({ nft list set inet "$NET_TABLE" clone_ifmac; nft list set netdev "$NET_TABLE_L2" clone_ifmac; } 2> /dev/null |
    sed -n "s/.*\"$port\" \\. \\([0-9a-f:]\\{17\\}\\).*/\\1/p" | head -1)
  [ -n "$mac" ] || return 0
  net_pin_txn destroy "$port" "$mac" "$ip" 2> /dev/null && return 0
  for t in "inet $NET_TABLE clone_ifmac { $port . $mac }" "inet $NET_TABLE clone_pin { $port . $mac . $ip }" \
    "netdev $NET_TABLE_L2 clone_ifmac { $port . $mac }" "netdev $NET_TABLE_L2 clone_arp { $port . $mac . $mac . $ip }"; do
    # shellcheck disable=SC2086
    nft destroy element $t 2> /dev/null || true
  done
}

# net_ok [ID]: the bridge is there, both tables are what was loaded, and (with an id) that clone is pinned.
# Run before a clone and on every poll while it runs: something on the node (an `nft flush ruleset`) can
# remove the table.
net_ok() {
  local ing fwd inp mac l2
  net_present && net_is_ours || { log "net: $NET_BRIDGE is not there"; return 1; }
  ing=$(nft list chain inet "$NET_TABLE" ingress 2> /dev/null) || { log "net: the table is not loaded"; return 1; }
  fwd=$(nft list chain inet "$NET_TABLE" forward 2> /dev/null) || { log "net: the table is not loaded"; return 1; }
  inp=$(nft list chain inet "$NET_TABLE" input 2> /dev/null) || { log "net: the table is not loaded"; return 1; }
  if ! { grep -q 'hook ingress devices = .* priority -500; policy drop' <<< "$ing" && grep -q '@clone_pin ether daddr .* accept' <<< "$ing" &&
    grep -q 'ether daddr' <<< "$ing" && grep -q 'oifname "yci0" counter packets [0-9]* bytes [0-9]* drop' <<< "$fwd" &&
    grep -q 'tcp dport 443 accept' <<< "$fwd" && grep -q 'iifname != "yci0" accept' <<< "$inp"; }; then
    log "net: the table is not what was loaded"
    return 1
  fi
  l2=$(nft list chain netdev "$NET_TABLE_L2" ingress 2> /dev/null) || { log "net: the layer-2 table is not loaded"; return 1; }
  if ! { grep -q 'hook ingress devices = .* priority -600; policy drop' <<< "$l2" &&
    grep -q '@clone_arp arp daddr ip '"$NET_GW"' counter packets [0-9]* bytes [0-9]* accept' <<< "$l2" &&
    grep -q 'ether type ip accept' <<< "$l2" && grep -q 'ether type { 8021q, 8021ad } counter packets [0-9]* bytes [0-9]* drop' <<< "$l2" &&
    grep -q 'ether type arp counter packets [0-9]* bytes [0-9]* drop' <<< "$l2"; }; then
    log "net: the layer-2 table is not what was loaded"
    return 1
  fi
  if [ -n "${1:-}" ]; then
    mac=$(vm_mac "$1")
    mac=${mac,,}
    nft get element inet "$NET_TABLE" clone_pin "{ tap${1}i0 . $mac . $NET_PREFIX.$(($1 - 9500)) }" > /dev/null 2>&1 ||
      { log "net: VM $1 is not pinned"; return 1; }
    nft get element netdev "$NET_TABLE_L2" clone_arp "{ tap${1}i0 . $mac . $mac . $NET_PREFIX.$(($1 - 9500)) }" > /dev/null 2>&1 ||
      { log "net: VM $1 is not pinned in the layer-2 table"; return 1; }
  fi
  return 0
}
