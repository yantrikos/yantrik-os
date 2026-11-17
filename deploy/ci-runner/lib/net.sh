# The clones' network on the node (#1252, VM design): the bridge `yci0` (no physical port), the one
# nftables table `inet yantrik_ci`, the forwarding switches, and the per-clone pins. All of it is made
# at runtime by net_up and removed by net_down; no network file is edited and no existing bridge,
# table or firewall is touched. Sourced by the controller, make-template.sh and yantrik-ci-net, which
# define log(), RUN_DIR, LIB_DIR and OWN_PUBLIC_FILE. See host/yci.nft for the rules.
# shellcheck shell=bash disable=SC2034,SC2015

NET_BRIDGE=yci0
NET_TABLE=yantrik_ci
NET_PREFIX=10.95.30     # the clones' subnet is $NET_PREFIX.0/24, the node is .1
NET_LEN=24
NET_GW=$NET_PREFIX.1
NET_MAC=02:59:43:49:30:01
NET_RESOLVER=${YANTRIK_CI_RESOLVER:-9.9.9.9}
NET_ID_MIN=9540         # the template (9540) and its clones (9541-9549): every id with a pin
NET_ID_MAX=9549
NET_SAVED_NAME=net.saved
NET_NFT=${NET_NFT:-$LIB_DIR/yci.nft} # the rules file (make-template.sh points it at its own copy)

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

# net_own: our own public addresses, one per line (a root-only file; each must be an IPv4 address).
net_own() {
  local line out=""
  trusted_path "$(dirname "$OWN_PUBLIC_FILE")" && trusted_path "$OWN_PUBLIC_FILE" || return 1
  while IFS= read -r line; do
    line=${line//[[:space:]]/}
    case $line in '' | '#'*) continue ;; esac
    [[ $line =~ ^([0-9]{1,3}\.){3}[0-9]{1,3}$ ]] || { log "REFUSING $OWN_PUBLIC_FILE: '$line' is not an IPv4 address"; return 1; }
    out+="${out:+, }$line"
  done < "$OWN_PUBLIC_FILE"
  [ -n "$out" ] || { log "$OWN_PUBLIC_FILE lists no address"; return 1; }
  printf '%s\n' "$out"
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

# net_up: make the bridge and load the table. Idempotent: an existing bridge of ours is reused and
# the table is replaced in one `nft -f`. Refuses a bridge of that name that is not ours.
net_up() {
  local up own pre ports="" i
  up=$(net_uplink)
  [ -n "$up" ] && [ "$up" != "$NET_BRIDGE" ] || { log "net: no uplink (default route device)"; return 1; }
  own=$(net_own) || return 1
  [ -f "$NET_NFT" ] && trusted_path "$NET_NFT" || { log "net: $NET_NFT is missing or not root's"; return 1; }
  if net_present; then
    net_is_ours || { log "REFUSING: a bridge $NET_BRIDGE exists and is not ours"; return 1; }
  else
    ip link add "$NET_BRIDGE" type bridge stp_state 0 forward_delay 0 &&
      ip link set "$NET_BRIDGE" address "$NET_MAC" &&
      ip addr add "$NET_GW/$NET_LEN" dev "$NET_BRIDGE" &&
      ip link set "$NET_BRIDGE" up || { log "net: cannot make $NET_BRIDGE"; net_present && ip link del "$NET_BRIDGE"; return 1; }
  fi
  # Forwarding for the bridge and the uplink only (the node's global ip_forward is left alone), strict
  # reverse-path checking and no redirects or IPv6 on the bridge.
  net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/forwarding" 1 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$up/forwarding" 1 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/rp_filter" 1 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/send_redirects" 0 &&
    net_sysctl "/proc/sys/net/ipv4/conf/$NET_BRIDGE/accept_redirects" 0 &&
    net_sysctl "/proc/sys/net/ipv6/conf/$NET_BRIDGE/disable_ipv6" 1 || { log "net: could not set the forwarding switches"; return 1; }
  for ((i = NET_ID_MIN; i <= NET_ID_MAX; i++)); do ports+="${ports:+, }$(net_port "$i")"; done
  pre=$(mktemp "$RUN_DIR/net.XXXXXX")
  {
    printf 'define ciif = "%s"\n' "$NET_BRIDGE"
    printf 'define ciports = { %s }\n' "$ports"
    printf 'define bridgemac = %s\n' "$NET_MAC"
    printf 'define subnet = %s.0/%s\n' "$NET_PREFIX" "$NET_LEN"
    printf 'define resolver = %s\n' "$NET_RESOLVER"
    printf 'define ownpub = { %s }\n' "$own"
    printf 'include "%s"\n' "$NET_NFT"
  } > "$pre"
  if ! nft -f "$pre" 2> "$pre.err"; then
    log "net: nft refused the ruleset: $(head -c 300 "$pre.err" | tr '\n' ' ')"
    rm -f "$pre" "$pre.err"
    return 1
  fi
  rm -f "$pre" "$pre.err"
  net_ok
}

# net_down: remove the table and the bridge and put the switches back. Refuses while a clone's port is
# still on the bridge (a clone left running would be unfiltered).
net_down() {
  if net_present; then
    net_is_ours || { log "REFUSING: $NET_BRIDGE is not ours: leaving it"; return 1; }
    if [ -n "$(net_ports)" ]; then log "REFUSING to remove $NET_BRIDGE: $(net_ports | paste -sd' ' -) still on it"; return 1; fi
  fi
  nft delete table inet "$NET_TABLE" 2> /dev/null || true
  if net_present; then ip link del "$NET_BRIDGE" || { log "net: could not delete $NET_BRIDGE"; return 1; }; fi
  net_restore_sysctl
}

# net_pin ID: pin the clone's port with its own MAC and address (from `qm config`). Run after the clone
# is configured and before `qm start`: the tap appears at start, so there is no gap. Fails closed.
net_pin() {
  local id=$1 mac port ip
  net_in_range "$id" || { log "net: refusing to pin $id"; return 1; }
  mac=$(vm_mac "$id")
  [[ $mac =~ ^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$ ]] || { log "VM $id: no MAC in its net0"; return 1; }
  mac=${mac,,}
  port=$(net_port "$id") ip=$(net_ip "$id")
  if ! { nft add element inet "$NET_TABLE" clone_ifmac "{ $port . $mac }" 2> /dev/null &&
    nft add element inet "$NET_TABLE" clone_pin "{ $port . $mac . $ip }" 2> /dev/null; }; then
    log "net: cannot pin $port to $mac $ip"
    return 1
  fi
  net_ok "$id"
}

# net_unpin ID: drop a clone's pins (the VM is gone, or given up on: an unpinned port is dropped).
net_unpin() {
  local id=$1 port mac ip
  port=$(net_port "$id") ip=$(net_ip "$id")
  mac=$(nft list set inet "$NET_TABLE" clone_ifmac 2> /dev/null | sed -n "s/.*\"$port\" \\. \\([0-9a-f:]\\{17\\}\\).*/\\1/p" | head -1)
  [ -z "$mac" ] || nft delete element inet "$NET_TABLE" clone_pin "{ $port . $mac . $ip }" 2> /dev/null || true
  [ -z "$mac" ] || nft delete element inet "$NET_TABLE" clone_ifmac "{ $port . $mac }" 2> /dev/null || true
}

# net_ok [ID]: the bridge is there, the table is what was loaded, and (with an id) that clone is pinned.
# Run before a clone and on every poll while it runs: something on the node (an `nft flush ruleset`) can
# remove the table.
net_ok() {
  local ing fwd inp
  net_present && net_is_ours || { log "net: $NET_BRIDGE is not there"; return 1; }
  ing=$(nft list chain inet "$NET_TABLE" ingress 2> /dev/null) || { log "net: the table is not loaded"; return 1; }
  fwd=$(nft list chain inet "$NET_TABLE" forward 2> /dev/null) || { log "net: the table is not loaded"; return 1; }
  inp=$(nft list chain inet "$NET_TABLE" input 2> /dev/null) || { log "net: the table is not loaded"; return 1; }
  if ! { grep -q 'hook ingress devices = .* priority -500; policy drop' <<< "$ing" && grep -q '@clone_pin accept' <<< "$ing" &&
    grep -q 'ether daddr' <<< "$ing" && grep -q 'oifname "yci0" counter packets [0-9]* bytes [0-9]* drop' <<< "$fwd" &&
    grep -q 'tcp dport 443 accept' <<< "$fwd" && grep -q 'iifname != "yci0" accept' <<< "$inp"; }; then
    log "net: the table is not what was loaded"
    return 1
  fi
  if [ -n "${1:-}" ]; then
    nft get element inet "$NET_TABLE" clone_pin "{ $(net_port "$1") . $(vm_mac "$1" | tr 'A-F' 'a-f') . $(net_ip "$1") }" > /dev/null 2>&1 ||
      { log "net: VM $1 is not pinned"; return 1; }
  fi
  return 0
}
