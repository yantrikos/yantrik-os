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
  pct stop "$id" > /dev/null 2>&1 || true
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

# --- the host-side filter (R3, H1) -----------------------------------------------------------

# host_filter_render RESOLVERS GATEWAY NODEIP OWN: load host-egress.nft with those values (comma
# lists, IPv4). Atomic: the table is replaced in one `nft -f`. The sets start empty; clones are
# added by host_filter_add_clone.
host_filter_render() {
  local res=$1 gw=$2 nodeip=$3 own=$4 pre
  [ -f "$LIB_DIR/host-egress.nft" ] || { log "host filter: $LIB_DIR/host-egress.nft is missing"; return 1; }
  trusted_path "$LIB_DIR/host-egress.nft" || return 1
  pre=$(mktemp "$RUN_DIR/hostnft.XXXXXX")
  {
    printf 'define resolvers = { %s }\n' "${res//,/, }"
    printf 'define ownpub = { %s }\n' "${own//,/, }"
    printf 'define gateway = %s\n' "$gw"
    printf 'define nodeip = %s\n' "$nodeip"
    printf 'define arpguard = { %s, %s, %s }\n' "$gw" "${res//,/, }" "$nodeip"
    printf 'include "%s/host-egress.nft"\n' "$LIB_DIR"
  } > "$pre"
  if ! nft -f "$pre" 2> /dev/null; then rm -f "$pre"; log "host filter: nft refused the ruleset"; return 1; fi
  rm -f "$pre"
  host_filter_ok
}

# host_filter_load: the same with the node's own values: its DNS servers, default gateway, vmbr0
# address, and our own public addresses from $OWN_PUBLIC_FILE. Returns non-zero, and the caller
# refuses the clone, if any of it is missing or not root's alone.
host_filter_load() {
  local res gw nodeip own
  trusted_path "$(dirname "$OWN_PUBLIC_FILE")" && trusted_path "$OWN_PUBLIC_FILE" || return 1
  res=$(awk '/^nameserver[ \t]/ {print $2}' /etc/resolv.conf | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' | grep -v '^127\.' | paste -sd, -)
  gw=$(ip -4 route show default | awk '{print $3; exit}')
  nodeip=$(ip -4 -o addr show "${NODE_BRIDGE:-vmbr0}" | awk '{sub(/\/.*/, "", $4); print $4; exit}')
  own=$(grep -vE '^[[:space:]]*(#|$)' "$OWN_PUBLIC_FILE" 2> /dev/null | paste -sd, -)
  if [ -z "$res" ] || [ -z "$gw" ] || [ -z "$nodeip" ] || [ -z "$own" ]; then
    log "host filter: need the node's resolvers (${res:-none}), gateway (${gw:-none}), address (${nodeip:-none}) and $OWN_PUBLIC_FILE (${own:-empty})"
    return 1
  fi
  host_filter_render "$res" "$gw" "$nodeip" "$own"
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
  if ! { grep -q 'iifname != @clone_ports accept' <<< "$fwd" && grep -q 'ether type != ip drop' <<< "$fwd" &&
    grep -qE '224.0.0.0/3|240.0.0.0/4' <<< "$fwd" && grep -q 'iifname . ether saddr != @clone_ok' <<< "$pre" &&
    grep -q 'arp saddr ip' <<< "$pre"; }; then
    log "host filter: the table is not what was loaded"
    return 1
  fi
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
