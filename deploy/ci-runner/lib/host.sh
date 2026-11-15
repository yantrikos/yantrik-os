# Node side of the CI controller (#1234): CT ids, destroying clones, the gates that must hold
# before a clone (memory, thin pool, the host firewall), and the numbers logged at the end of a
# job. Sourced by yantrik-ci-controller.sh, which defines log(), nap() and the config used here.
# shellcheck shell=bash disable=SC2034

# --- ids ------------------------------------------------------------------------------

id_in_use() { compgen -G "/etc/pve/nodes/*/lxc/$1.conf" > /dev/null || compgen -G "/etc/pve/nodes/*/qemu-server/$1.conf" > /dev/null; }
lxc_exists() { compgen -G "/etc/pve/nodes/*/lxc/$1.conf" > /dev/null; }

free_id() {
  local id
  for ((id = ID_MIN; id <= ID_MAX; id++)); do
    if ! id_in_use "$id"; then printf '%s\n' "$id"; return 0; fi
  done
  return 1
}

in_range() { [[ $1 =~ ^[0-9]+$ ]] && [ "$1" -ge "$ID_MIN" ] && [ "$1" -le "$ID_MAX" ]; }

# Stop and destroy one clone. Refuses anything outside the range, the template itself, and
# any CT that is not named for this controller: an id someone else took is not ours (N7).
destroy_ct() {
  local id=$1
  in_range "$id" || { log "refusing to destroy $id: outside $ID_MIN-$ID_MAX"; return 1; }
  [ "$id" != "$TEMPLATE_ID" ] || return 1
  lxc_exists "$id" || return 0
  if ! pct config "$id" 2> /dev/null | grep -q "^hostname: yantrik-ci-$id\$"; then
    log "refusing to destroy CT $id: it is not named yantrik-ci-$id, so it is not ours"
    return 1
  fi
  if ! pct stop "$id" > /dev/null 2>&1; then
    # A hung or failing `pct stop`: end every task in the CT's cgroup, init included, from the kernel
    # (cgroup.kill, Linux 5.14 and later), then let `pct destroy` have another go.
    if [ -w "/sys/fs/cgroup/lxc/$id/cgroup.kill" ]; then
      echo 1 > "/sys/fs/cgroup/lxc/$id/cgroup.kill" 2> /dev/null || true
      sleep 2
    fi
  fi
  pct destroy "$id" --purge > /dev/null 2>&1 || true
  ! lxc_exists "$id"
}

destroy_all_clones() {
  local conf id rc=0
  for conf in /etc/pve/nodes/*/lxc/*.conf; do
    [ -e "$conf" ] || continue
    id=$(basename "$conf" .conf)
    in_range "$id" || continue
    log "start-up cleanup: destroying leftover CT $id"
    destroy_ct "$id" || { log "could not destroy leftover CT $id"; rc=1; }
  done
  return $rc
}

# --- gates ------------------------------------------------------------------------------

mem_available_kb() { awk '/^MemAvailable:/ {print $2}' /proc/meminfo; }
mem_ok() { [ "$(mem_available_kb)" -ge "$MIN_MEM_KB" ]; }
mem_idle_ok() { [ "$(mem_available_kb)" -ge "$IDLE_MIN_MEM_KB" ]; }

# The rootfs size of the template, in GB (what a clone may fill).
rootfs_gb() { pct config "$TEMPLATE_ID" | sed -n 's/^rootfs:.*size=\([0-9]*\)G.*/\1/p'; }

# The thin pool must have room for a whole rootfs plus 20%, and its metadata must be below 80%:
# a full pool errors the writes of every volume on it, other guests included (R4).
pool_ok() {
  local line size dp mp need
  need=$(rootfs_gb)
  [[ $need =~ ^[0-9]+$ ]] || { log "cannot read the template's rootfs size"; return 1; }
  line=$(LC_ALL=C lvs --noheadings --nosuffix --units g -o lv_size,data_percent,metadata_percent "$POOL" 2> /dev/null) ||
    { log "cannot read the thin pool $POOL"; return 1; }
  read -r size dp mp <<< "$line"
  if awk -v s="${size:-0}" -v d="${dp:-100}" -v m="${mp:-100}" -v n="$need" 'BEGIN { exit !(s * (100 - d) / 100 >= n * 1.2 && m < 80) }'; then
    return 0
  fi
  log "thin pool $POOL is too full for a clone: size ${size} GB, data ${dp}%, metadata ${mp}%, a clone may need ${need} GB x 1.2"
  return 1
}

# --- the clone's fixed address (N3) --------------------------------------------------------

CLONE_IP="" CLONE_LEN="" CLONE_GW=""

# arp_query BRIDGE SPA TPA: ask the bridge's segment "who has TPA?" (sender address SPA, 0.0.0.0 for
# a probe that updates no cache) and print the MAC of whoever answers, if anyone does within 3 s.
# Raw AF_PACKET, no extra package on the node.
arp_query() {
  python3 - "$1" "$2" "$3" << 'PY'
import socket, struct, sys, time
br, spa, tpa = sys.argv[1:4]
mac = bytes.fromhex(open('/sys/class/net/%s/address' % br).read().strip().replace(':', ''))
s = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(0x0806))
s.bind((br, 0))
s.settimeout(0.5)
req = b'\xff' * 6 + mac + b'\x08\x06' + struct.pack('!HHBBH6s4s6s4s', 1, 0x0800, 6, 4, 1, mac, socket.inet_aton(spa), b'\0' * 6, socket.inet_aton(tpa))
end = time.time() + 3
last = 0
while time.time() < end:
    if time.time() - last > 1:
        s.send(req)
        last = time.time()
    try:
        f = s.recv(2048)
    except socket.timeout:
        continue
    if len(f) >= 42 and f[12:14] == b'\x08\x06' and struct.unpack('!H', f[20:22])[0] == 2 and f[28:32] == socket.inet_aton(tpa):
        print(':'.join('%02x' % b for b in f[22:28]))
        break
PY
}

# clone_addr_free: nothing on the bridge's segment answers for the clone's address (L3). One probe
# per controller start; after the first clone the address is ours.
clone_addr_free() {
  local who
  who=$(arp_query "${CLONE_BRIDGE:-vmbr0}" 0.0.0.0 "$CLONE_IP")
  if [ -n "$who" ]; then
    log "REFUSING to clone: $CLONE_IP already answers on ${CLONE_BRIDGE:-vmbr0} (MAC $who)"
    return 1
  fi
}

# clone_net_check FILE NODE_CIDR GATEWAY RESOLVERS [DENY]: read the one line `ip=<addr>/<len>,gw=<gw>`.
# Strict format; the address must be inside the node's subnet and not the network or broadcast
# address, the gateway, the node, a resolver or one of DENY (comma list: CT 530's address); the
# gateway must be the node's own. Sets CLONE_IP,
# CLONE_LEN and CLONE_GW, or returns non-zero with the reason logged. Fail closed.
clone_net_check() {
  local file=$1 nodecidr=$2 gw=$3 res=$4 deny=${5:-} line r
  [ -f "$file" ] || { log "no $file: the clones' fixed address is not set"; return 1; }
  trusted_path "$(dirname "$file")" && trusted_path "$file" || return 1
  line=$(< "$file")
  [[ $line =~ ^ip=([0-9]{1,3}(\.[0-9]{1,3}){3})/([0-9]{1,2}),gw=([0-9]{1,3}(\.[0-9]{1,3}){3})$ ]] ||
    { log "$file is not one line 'ip=<addr>/<len>,gw=<gw>'"; return 1; }
  CLONE_IP=${BASH_REMATCH[1]} CLONE_LEN=${BASH_REMATCH[3]} CLONE_GW=${BASH_REMATCH[4]}
  if ! r=$(python3 - "$CLONE_IP" "$CLONE_LEN" "$CLONE_GW" "$nodecidr" "$gw" "$res" "$deny" 2>&1 << 'PY'
import ipaddress, sys
ip, ln, cgw, nodecidr, gw, res, deny = sys.argv[1:8]
try:
    net = ipaddress.ip_interface(nodecidr).network
    a = ipaddress.ip_address(ip)
    node = ipaddress.ip_interface(nodecidr).ip
    if int(ln) != net.prefixlen: sys.exit("prefix /%s is not the node's /%d" % (ln, net.prefixlen))
    if a not in net: sys.exit("%s is outside %s" % (ip, net))
    if a in (net.network_address, net.broadcast_address): sys.exit("%s is the network or broadcast address" % ip)
    if ip == cgw or ip == gw or a == node: sys.exit("%s is the gateway or the node" % ip)
    if cgw != gw: sys.exit("gw %s is not the node's gateway %s" % (cgw, gw))
    if ip in [x for x in res.split(",") if x]: sys.exit("%s is a resolver" % ip)
    if ip in [x for x in deny.split(",") if x]: sys.exit("%s is the build host's address" % ip)
except ValueError as e:
    sys.exit(str(e))
PY
  ); then
    log "REFUSING $file: $r"
    CLONE_IP="" CLONE_LEN="" CLONE_GW=""
    return 1
  fi
}

# The node's own values for the check above (what the controller uses).
node_values() {
  NODE_CIDR=$(ip -4 -o addr show "${NODE_BRIDGE:-vmbr0}" | awk '{print $4; exit}')
  NODE_GW=${YANTRIK_CI_GATEWAY:-$(ip -4 route show default | awk '{print $3; exit}')}
  NODE_RES=$(awk '/^nameserver[ \t]/ {print $2}' /etc/resolv.conf | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' | grep -v '^127\.' | paste -sd, -)
  NODE_OWN=$(grep -vE '^[[:space:]]*(#|$)' "$OWN_PUBLIC_FILE" 2> /dev/null | paste -sd, -)
  [ -n "$NODE_CIDR" ] && [ -n "$NODE_GW" ] && [ -n "$NODE_RES" ] && [ -n "$NODE_OWN" ] ||
    { log "need the node's address (${NODE_CIDR:-none}), gateway (${NODE_GW:-none}), resolvers (${NODE_RES:-none}) and $OWN_PUBLIC_FILE (${NODE_OWN:-empty})"; return 1; }
}

clone_net_load() {
  local ct530
  node_values || return 1
  ct530=$(pct config "${BUILD_CT:-530}" 2> /dev/null | sed -n '/^net0:/s/.*ip=\([0-9.]*\).*/\1/p')
  clone_net_check "$CLONE_NET_FILE" "$NODE_CIDR" "$NODE_GW" "$NODE_RES" "$ct530"
}

# clone_set_net ID: give the clone its fixed address before it starts: static `ip=`/`gw=` in its net0,
# its own MAC kept. PVE then writes a static config and no DHCP client runs in the CT.
clone_set_net() {
  local id=$1 mac
  mac=$(pct config "$id" 2> /dev/null | sed -n '/^net0:/s/.*hwaddr=\([0-9A-Fa-f:]\{17\}\).*/\1/p')
  [ -n "$mac" ] || { log "CT $id: no MAC in its net0"; return 1; }
  if ! pct set "$id" --net0 "name=eth0,bridge=${CLONE_BRIDGE:-vmbr0},hwaddr=$mac,ip=$CLONE_IP/$CLONE_LEN,gw=$CLONE_GW,type=veth" > /dev/null 2>&1 ||
    ! pct config "$id" | grep '^net0:' | grep -q "ip=$CLONE_IP/$CLONE_LEN"; then
    log "CT $id: could not set its fixed address"
    return 1
  fi
}

# --- the host-side filter (R3, H1, N1) -----------------------------------------------------

# host_filter_render RESOLVERS GATEWAY CLONEIP OWN GWMAC: load host-egress.nft with those values (comma
# lists, IPv4) and fill `ci_ports` with every port a clone can have. Atomic: the table is replaced
# in one `nft -f`. The pin sets start empty; host_filter_add_clone fills them. A clone not pinned
# has no network.
host_filter_render() {
  local res=$1 gw=$2 cloneip=$3 own=$4 gwmac=$5 pre ports="" i
  [[ $gwmac =~ ^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$ ]] || { log "host filter: the gateway's MAC is unknown"; return 1; }
  [ -f "$LIB_DIR/host-egress.nft" ] || { log "host filter: $LIB_DIR/host-egress.nft is missing"; return 1; }
  trusted_path "$LIB_DIR/host-egress.nft" || return 1
  for ((i = ID_MIN; i <= ID_MAX; i++)); do ports+="${ports:+, }$(clone_port "$i")"; done
  pre=$(mktemp "$RUN_DIR/hostnft.XXXXXX")
  {
    printf 'define resolvers = { %s }\n' "${res//,/, }"
    printf 'define ownpub = { %s }\n' "${own//,/, }"
    printf 'define gateway = %s\n' "$gw"
    printf 'define cloneip = %s\n' "$cloneip"
    printf 'define gwmac = %s\n' "$gwmac"
    printf 'include "%s/host-egress.nft"\n' "$LIB_DIR"
    printf 'add element bridge yantrik_ci ci_ports { %s }\n' "$ports"
  } > "$pre"
  if ! nft -f "$pre" 2> /dev/null; then rm -f "$pre"; log "host filter: nft refused the ruleset"; return 1; fi
  rm -f "$pre"
  host_filter_ok
}

# host_filter_load: the same with the node's own values. Needs clone_net_load first (CLONE_IP).
host_filter_load() {
  trusted_path "$(dirname "$OWN_PUBLIC_FILE")" && trusted_path "$OWN_PUBLIC_FILE" || return 1
  node_values || return 1
  [ -n "$CLONE_IP" ] || { log "host filter: the clone's address is not set"; return 1; }
  local gwmac
  gwmac=$(arp_query "${NODE_BRIDGE:-vmbr0}" "${NODE_CIDR%/*}" "$NODE_GW")
  host_filter_render "$NODE_RES" "$NODE_GW" "$CLONE_IP" "$NODE_OWN" "$gwmac"
}

# The host-side port name of a clone.
clone_port() { printf 'veth%si0' "$1"; }

# host_filter_add_clone ID: pin the clone's port and its own MAC (the hwaddr in its net0, from
# `pct config`). Run after the clone exists and before `pct start`: the veth appears at start, so
# there is no gap. Fails closed.
host_filter_add_clone() {
  local id=$1 mac port
  mac=$(pct config "$id" 2> /dev/null | sed -n '/^net0:/s/.*hwaddr=\([0-9A-Fa-f:]\{17\}\).*/\1/p')
  [[ $mac =~ ^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$ ]] || { log "CT $id: no MAC in its net0"; return 1; }
  port=$(clone_port "$id")
  if ! { nft add element bridge yantrik_ci clone_ports "{ $port }" 2> /dev/null &&
    nft add element bridge yantrik_ci clone_ok "{ $port . $mac }" 2> /dev/null; }; then
    log "host filter: cannot pin $port to $mac"
    return 1
  fi
  host_filter_ok "$id"
}

# Drop a clone's pin. Called when the CT is gone, and also for a CT that survives its job: the
# unpinned port is then dropped by the table (which stays loaded), so a survivor has no network.
host_filter_remove_clone() {
  local id=$1 port mac
  port=$(clone_port "$id")
  mac=$(nft list set bridge yantrik_ci clone_ok 2> /dev/null | sed -n "s/.*\"$port\" \\. \\([0-9a-f:]\\{17\\}\\).*/\\1/p" | head -1)
  [ -z "$mac" ] || nft delete element bridge yantrik_ci clone_ok "{ $port . $mac }" 2> /dev/null || true
  nft delete element bridge yantrik_ci clone_ports "{ $port }" 2> /dev/null || true
}

# host_filter_ok [ID]: is the filter in place and still what was loaded; with an id, is that
# clone pinned? Run before a clone and on every poll while it runs (M4): something on the node
# (an `nft flush ruleset`, a restart of nftables.service) can remove the table.
host_filter_ok() {
  local fwd pre
  fwd=$(nft list chain bridge yantrik_ci forward 2> /dev/null) || { log "host filter: the table is not loaded"; return 1; }
  pre=$(nft list chain bridge yantrik_ci prerouting 2> /dev/null) || { log "host filter: the table is not loaded"; return 1; }
  if ! { grep -q 'iifname != @ci_ports accept' <<< "$fwd" && grep -q 'iifname != @clone_ports drop' <<< "$fwd" &&
    grep -q 'ether type != ip drop' <<< "$fwd" && grep -qE '224\.0\.0\.0/3|240\.0\.0\.0/4' <<< "$fwd" &&
    grep -q 'iifname . ether saddr != @clone_ok' <<< "$pre" && grep -q 'ip saddr !=' <<< "$pre" &&
    grep -q 'arp saddr ip !=' <<< "$pre"; }; then
    log "host filter: the table is not what was loaded"
    return 1
  fi
  nft get element bridge yantrik_ci ci_ports "{ $(clone_port "$ID_MIN") }" > /dev/null 2>&1 ||
    { log "host filter: ci_ports is empty"; return 1; }
  if [ -n "${1:-}" ]; then
    nft get element bridge yantrik_ci clone_ports "{ $(clone_port "$1") }" > /dev/null 2>&1 ||
      { log "host filter: CT $1 is not pinned"; return 1; }
  fi
  return 0
}

# --- the node's OOM killer -----------------------------------------------------------------------

# Make the clone the host OOM killer's first victim: a global OOM then ends the CI job, not a VM
# or the nightly. PVE's config parser rejects `lxc.proc.oom_score_adj`, so the value is set on the
# CT's init (what the runner and the job later fork from inherit it). Run right after `pct start`.
ct_oom_first() {
  local pid
  pid=$(lxc-info -n "$1" -p -H 2> /dev/null)
  [[ $pid =~ ^[0-9]+$ ]] || { log "CT $1: cannot find its init pid for oom_score_adj"; return 1; }
  echo 1000 > "/proc/$pid/oom_score_adj" 2> /dev/null && [ "$(cat "/proc/$pid/oom_score_adj")" = 1000 ]
}

# --- numbers at the end of a job -----------------------------------------------------------

# ct_stats ID: the memory high-water mark and the oom counts from the host's cgroup files, and the
# disk the clone has written, from the thin pool (`lvs`). Nothing is run inside the CT: after the
# runner has started, the CT is hostile and `pct exec` would attach host root to it (M3). Call it
# before the stop, while the cgroup still exists.
ct_stats() {
  local id=$1 cg=/sys/fs/cgroup/lxc/$1 peak oom disk
  peak=$(awk '{printf "%d", $1 / 1048576}' "$cg/memory.peak" 2> /dev/null)
  oom=$(awk '$1 == "oom" || $1 == "oom_kill" {printf "%s=%s ", $1, $2}' "$cg/memory.events" 2> /dev/null)
  disk=$(LC_ALL=C lvs --noheadings --nosuffix --units g -o lv_size,data_percent "${POOL%/*}/vm-$id-disk-0" 2> /dev/null |
    awk '{printf "%.0fG of %.0fG", $1 * $2 / 100, $1}')
  printf 'disk written %s, memory.peak %s MiB, %s' "${disk:-?}" "${peak:-?}" "${oom:-oom counts ?}"
}
