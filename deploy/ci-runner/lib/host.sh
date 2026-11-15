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

# --- the host-side filter (R3) ---------------------------------------------------------------

# Load /usr/local/lib/yantrik-ci/host-egress.nft: a `table bridge yantrik_ci` that matches only
# frames entering the bridge from a clone's port (veth96*). The node's DNS servers and its
# default gateway are read here; our own public addresses come from /etc/yantrik-ci/own-public-ips.
# Returns non-zero, and the caller refuses the clone, if any of it is missing.
host_filter_load() {
  local res gw own pre
  res=$(awk '/^nameserver[ \t]/ {print $2}' /etc/resolv.conf | grep -E '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$' | grep -v '^127\.' | paste -sd, - | sed 's/,/, /g')
  gw=$(ip -4 route show default | awk '{print $3; exit}')
  own=$(grep -vE '^[[:space:]]*(#|$)' "$OWN_PUBLIC_FILE" 2> /dev/null | paste -sd, - | sed 's/,/, /g')
  if [ -z "$res" ] || [ -z "$gw" ] || [ -z "$own" ]; then
    log "host filter: need the node's resolvers (${res:-none}), gateway (${gw:-none}) and $OWN_PUBLIC_FILE (${own:-empty})"
    return 1
  fi
  [ -f "$LIB_DIR/host-egress.nft" ] || { log "host filter: $LIB_DIR/host-egress.nft is missing"; return 1; }
  pre=$(mktemp "$RUN_DIR/hostnft.XXXXXX")
  {
    printf 'define resolvers = { %s }\n' "$res"
    printf 'define ownpub = { %s }\n' "$own"
    printf 'define gateway = %s\n' "$gw"
    printf 'include "%s/host-egress.nft"\n' "$LIB_DIR"
  } > "$pre"
  if ! nft -f "$pre" 2> /dev/null; then rm -f "$pre"; log "host filter: nft refused the ruleset"; return 1; fi
  rm -f "$pre"
  host_filter_ok
}

# Is the filter in place, and does it still drop what it should?
host_filter_ok() {
  local fwd inp
  fwd=$(nft list chain bridge yantrik_ci forward 2> /dev/null) || { log "host filter: the table is not loaded"; return 1; }
  inp=$(nft list chain bridge yantrik_ci input 2> /dev/null) || { log "host filter: the table is not loaded"; return 1; }
  if grep -q 'iifname != "veth96\*" accept' <<< "$fwd" && grep -q 'ether type != ip drop' <<< "$fwd" &&
    grep -q '192.168.0.0/16' <<< "$fwd" && grep -q 'counter packets [0-9]* bytes [0-9]* drop' <<< "$inp"; then
    return 0
  fi
  log "host filter: the table is not what was loaded"
  return 1
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

# ct_stats ID: disk used, the memory high-water mark and the oom counts, for the `done` line.
# Needs the CT still running.
ct_stats() {
  local id=$1 used peak oom cg=/sys/fs/cgroup/lxc/$1
  used=$(timeout 20 pct exec "$id" -- df -BG --output=used / 2> /dev/null | tail -1 | tr -d ' ')
  peak=$(awk '{printf "%d", $1 / 1048576}' "$cg/memory.peak" 2> /dev/null)
  oom=$(awk '$1 == "oom" || $1 == "oom_kill" {printf "%s=%s ", $1, $2}' "$cg/memory.events" 2> /dev/null)
  printf 'disk used %s, memory.peak %s MiB, %s' "${used:-?}" "${peak:-?}" "${oom:-oom counts ?}"
}
