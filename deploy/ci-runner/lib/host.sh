# Node side of the CI controller (#1252, VM design): VM ids, cloning and destroying the clones, the gates
# that must hold before a clone (memory, thin pool), and the numbers logged at the end of a job. The
# network is lib/net.sh. Sourced by yantrik-ci-controller.sh and make-template.sh, which define log(),
# nap() and the config used here.
# shellcheck shell=bash disable=SC2034

# nap is the controller's (a sleep a signal can interrupt); the other callers get a plain sleep.
declare -F nap > /dev/null || nap() { sleep "$1"; }

# --- ids ------------------------------------------------------------------------------

id_in_use() { compgen -G "/etc/pve/nodes/*/lxc/$1.conf" > /dev/null || compgen -G "/etc/pve/nodes/*/qemu-server/$1.conf" > /dev/null; }
vm_exists() { compgen -G "/etc/pve/nodes/*/qemu-server/$1.conf" > /dev/null; }

free_id() {
  local id
  for ((id = ID_MIN; id <= ID_MAX; id++)); do
    if ! id_in_use "$id"; then printf '%s\n' "$id"; return 0; fi
  done
  return 1
}

in_range() { [[ $1 =~ ^[0-9]+$ ]] && [ "$1" -ge "$ID_MIN" ] && [ "$1" -le "$ID_MAX" ]; }

vm_running() { [ "$(qm status "$1" 2> /dev/null)" = "status: running" ]; }
vm_mac() { qm config "$1" 2> /dev/null | sed -n '/^net0:/s/^net0: virtio=\([0-9A-Fa-f:]\{17\}\).*/\1/p'; }
vm_pid() { local p; p=$(cat "/run/qemu-server/$1.pid" 2> /dev/null); [[ $p =~ ^[0-9]+$ ]] && printf '%s\n' "$p"; }

# vm_disks ID: the logical volumes of a VM on the thin pool (what must be gone after a destroy).
vm_disks() { LC_ALL=C lvs --noheadings -o lv_name "${POOL%/*}" 2> /dev/null | awk -v id="$1" '$1 ~ "^vm-" id "-" {print $1}'; }

# Stop and destroy one clone, and drop its pin. Refuses anything outside the range, the template itself,
# and any VM that is not named for this controller: an id someone else took is not ours (N7).
destroy_vm() {
  local id=$1 pid
  in_range "$id" || { log "refusing to destroy $id: outside $ID_MIN-$ID_MAX"; return 1; }
  [ "$id" != "$TEMPLATE_ID" ] || return 1
  vm_exists "$id" || { net_unpin "$id"; return 0; }
  if ! qm config "$id" 2> /dev/null | grep -q "^name: yantrik-ci-$id\$"; then
    log "refusing to destroy VM $id: it is not named yantrik-ci-$id, so it is not ours"
    return 1
  fi
  if vm_running "$id"; then
    if ! qm stop "$id" --timeout 30 > /dev/null 2>&1 && vm_running "$id"; then
      # A hung `qm stop`: kill the QEMU process of this VM, by its pid file.
      pid=$(vm_pid "$id") && kill -KILL "$pid" 2> /dev/null || true
      sleep 2
    fi
  fi
  qm unlock "$id" > /dev/null 2>&1 || true
  qm destroy "$id" --purge 1 --destroy-unreferenced-disks 1 > /dev/null 2>&1 || true
  if vm_exists "$id"; then return 1; fi
  net_unpin "$id"
  return 0
}

destroy_all_clones() {
  local conf id rc=0
  for conf in /etc/pve/nodes/*/qemu-server/*.conf; do
    [ -e "$conf" ] || continue
    id=$(basename "$conf" .conf)
    in_range "$id" || continue
    log "start-up cleanup: destroying leftover VM $id"
    destroy_vm "$id" || { log "could not destroy leftover VM $id: unpinning it"; net_unpin "$id"; rc=1; }
  done
  return $rc
}

# --- gates ------------------------------------------------------------------------------

mem_available_kb() { awk '/^MemAvailable:/ {print $2}' /proc/meminfo; }
mem_ok() { [ "$(mem_available_kb)" -ge "$MIN_MEM_KB" ]; }
mem_idle_ok() { [ "$(mem_available_kb)" -ge "$IDLE_MIN_MEM_KB" ]; }

# The template's memory in kB (`memory:` in qm config is MiB), and when it was created (`meta: ...ctime=`, a unix time).
template_memory_kb() {
  local m
  m=$(qm config "$TEMPLATE_ID" 2> /dev/null | awk '/^memory:/ {print $2}')
  [[ $m =~ ^[0-9]+$ ]] && echo $((m * 1024))
}
template_ctime() { qm config "$TEMPLATE_ID" 2> /dev/null | awk -F'ctime=' '/^meta:/ {split($2, a, ","); print a[1]}'; }

# The disk size of the template, in GB (what a clone may fill).
rootfs_gb() { qm config "$TEMPLATE_ID" | sed -n 's/^scsi0:.*size=\([0-9]*\)G.*/\1/p'; }

# The thin pool must have room for a whole disk plus 20%, and its metadata must be below 80%:
# a full pool errors the writes of every volume on it, other guests included (R4).
pool_ok() {
  local line size dp mp need
  need=$(rootfs_gb)
  [[ $need =~ ^[0-9]+$ ]] || { log "cannot read the template's disk size"; return 1; }
  line=$(LC_ALL=C lvs --noheadings --nosuffix --units g -o lv_size,data_percent,metadata_percent "$POOL" 2> /dev/null) ||
    { log "cannot read the thin pool $POOL"; return 1; }
  read -r size dp mp <<< "$line"
  if awk -v s="${size:-0}" -v d="${dp:-100}" -v m="${mp:-100}" -v n="$need" 'BEGIN { exit !(s * (100 - d) / 100 >= n * 1.2 && m < 80) }'; then
    return 0
  fi
  log "thin pool $POOL is too full for a clone: size ${size} GB, data ${dp}%, metadata ${mp}%, a clone may need ${need} GB x 1.2"
  return 1
}

# --- making a clone ----------------------------------------------------------------------------

CLONE_MODE=
# clone_vm ID: a linked clone of the template (a full one if that fails).
clone_vm() {
  local id=$1
  if qm clone "$TEMPLATE_ID" "$id" --name "yantrik-ci-$id" > /dev/null 2>&1; then
    CLONE_MODE=linked
  else
    destroy_vm "$id" || true
    if qm clone "$TEMPLATE_ID" "$id" --name "yantrik-ci-$id" --full 1 > /dev/null 2>&1; then
      CLONE_MODE=full
    else
      destroy_vm "$id" || true
      return 1
    fi
  fi
}

# clone_configure ID: the clone's one network card on yci0 with its own MAC, its fixed address and
# resolver by cloud-init (no DHCP, no LAN; `--ciuser root` so that PVE's user-data does not ask cloud-init for
# a default user and its sudo rule), then a check that nothing else is attached: no second
# card, no host device, no shared folder.
clone_configure() {
  local id=$1 mac cfg
  mac=$(vm_mac "$id")
  [[ $mac =~ ^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$ ]] || { log "VM $id: no MAC in its net0"; return 1; }
  if ! qm set "$id" --net0 "virtio=$mac,bridge=$NET_BRIDGE,firewall=0" \
    --ipconfig0 "ip=$(net_ip "$id")/$NET_LEN,gw=$NET_GW" --nameserver "$NET_RESOLVER" --searchdomain yantrik-ci.invalid \
    --ciuser root --ciupgrade 0 --onboot 0 > /dev/null 2>&1; then
    log "VM $id: could not set its network"
    return 1
  fi
  cfg=$(qm config "$id" 2> /dev/null)
  if ! clone_keys_ok "$id" "$cfg" || ! grep -q "^net0: virtio=$mac,bridge=$NET_BRIDGE,firewall=0\$" <<< "$cfg"; then
    log "VM $id: its configuration is not the template's plain one: refusing it"
    return 1
  fi
  grep -q '^ciupgrade: 0$' <<< "$cfg" || { log "VM $id: cloud-init would upgrade packages at first boot"; return 1; }
}

# The keys a clone's configuration may have (L7): an allowlist, so that `args:`, `hostpci`, `ivshmem`, a second
# disk or card, `cicustom` and anything a later PVE adds are refused rather than missed. Every line of `qm config`
# is checked, a `[PENDING]` section header included. `ide2` is not in it: the rebuilt template's cloud-init drive
# is `scsi1`. For the keys that can name a host resource the VALUE is checked too (clone_values_ok).
CLONE_KEYS=" name meta vmgenid smbios1 ostype machine cpu cores sockets memory balloon cpuunits scsihw scsi0 scsi1 net0 boot agent serial0 vga tablet onboot ciuser ciupgrade ipconfig0 nameserver searchdomain template digest description allow-ksm "
CLONE_STORAGE=local-lvm
clone_keys_ok() {
  local line key
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    key=${line%%:*}
    case $CLONE_KEYS in *" $key "*) ;; *) log "a key a clone may not have: $key"; return 1 ;; esac
  done <<< "$2"
  clone_values_ok "$1" "$2"
}

# clone_values_ok ID CONFIG: the values of the keys that could reach a host resource are exactly the template's
# (N3): the root disk is a volume of this clone on $CLONE_STORAGE (or linked to the template's base), the cloud-init
# drive is this clone's own, the serial port is a PVE-managed socket and not a host device, the display is that
# serial port, and KSM is off. Each must be present: a clone that lost one is not the template's clone.
clone_values_ok() {
  local id=$1 cfg=$2 st=$CLONE_STORAGE bad=0 pat
  local opts='(,[a-z]+=[A-Za-z0-9]+)*'
  for pat in \
    "scsi0: $st:(vm-$id-disk-[0-9]+|base-$TEMPLATE_ID-disk-[0-9]+/vm-$id-disk-[0-9]+)$opts" \
    "scsi1: $st:vm-$id-cloudinit,media=cdrom(,size=[0-9]+M)?" \
    "serial0: socket" "vga: serial0" "allow-ksm: 0"; do
    grep -Eq "^$pat\$" <<< "$cfg" || { log "VM $id: its configuration lacks '${pat%%:*}' with the template's value"; bad=1; }
  done
  [ "$bad" -eq 0 ]
}

# vm_agent_wait ID: the guest agent answers (the guest has booted and has its address). Gives up after 180 s by
# the clock or when the controller is told to stop (L8).
vm_agent_wait() {
  local id=$1 t0=$SECONDS
  while [ $((SECONDS - t0)) -lt 180 ] && [ "${STOP:-0}" -eq 0 ]; do
    qm agent "$id" ping > /dev/null 2>&1 && return 0
    vm_running "$id" || return 1
    nap 2
  done
  return 1
}

# push_jit ID: the one-job config goes to the guest on stdin of a command run through the guest agent
# (no file on the node, no command line): the guest keeps it on tmpfs and starts the launcher, which
# reads it once and deletes it. printf is a builtin, so the config is on no process's command line.
push_jit() {
  local id=$1 out rc
  vm_agent_wait "$id" || { log "VM $id never became ready for the agent"; return 1; }
  out=$(printf '%s' "$JIT" | qm guest exec "$id" --pass-stdin 1 --timeout 60 -- /usr/local/sbin/yantrik-ci-start 2>&1)
  rc=$?
  [ $rc -eq 0 ] && [ "$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("exitcode"))' <<< "$out" 2> /dev/null)" = 0 ] ||
    { log "VM $id: handing over the config failed (qm rc $rc)"; return 1; }
}

# vm_agent_state ID: what QEMU itself says about the agent's device, from its monitor and not from anything the guest
# can answer: `gone`, `attached` or `unknown` (the monitor gave no usable answer, which proves nothing about the
# device). Two views must agree that `qga0`, the virtio-serial device PVE puts the agent on, is not there: `info pci`
# (the device on the bus) and `qom-list /machine/peripheral` (the device object). Returns 0 only for `gone`.
vm_agent_state() {
  local id=$1 pci qom
  pci=$(printf '%s\n' 'info pci' | qm monitor "$id" 2>&1)
  qom=$(printf '%s\n' 'qom-list /machine/peripheral' | qm monitor "$id" 2>&1)
  if ! grep -q '^ *Bus ' <<< "$pci" || ! grep -q '^pci\.0 ' <<< "$qom"; then
    echo unknown
    return 1
  fi
  if grep -q 'id "qga0"' <<< "$pci" || grep -q '^qga0 ' <<< "$qom"; then
    echo attached
    return 1
  fi
  echo gone
}

# vm_agent_close ID: after the hand-over, take the agent's device away from the VM from the host's side:
# `device_del qga0`, then ask QEMU (vm_agent_state) every second for up to $AGENT_CLOSE_S seconds until the device is
# gone. Returns 0 only when QEMU says it is gone. The unplug is a PCI hot-unplug that the guest must acknowledge;
# the guest at this moment is still the pristine template, and the launcher does not start the runner until the
# device has gone from the guest too, so a guest that does not acknowledge never runs a job. Whoever calls this
# destroys the VM when it returns 1 (L3, N1). The result is logged for every job.
vm_agent_close() {
  local id=$1 out st t0=$SECONDS
  out=$(printf '%s\n' 'device_del qga0' | qm monitor "$id" 2>&1)
  while :; do
    if st=$(vm_agent_state "$id"); then
      log "VM $id: the agent's device is gone from QEMU's view ($((SECONDS - t0)) s after device_del)"
      return 0
    fi
    { [ $((SECONDS - t0)) -lt "${AGENT_CLOSE_S:-20}" ] && [ "${STOP:-0}" -eq 0 ]; } || break
    nap 1
  done
  out=${out//$'\n'/ }
  log "VM $id: the agent's device is still $st in QEMU's view $((SECONDS - t0)) s after device_del: ${out:0:160}"
  return 1
}

# --- the node's OOM killer ------------------------------------------------------------------------

# Make the clone the host OOM killer's first victim: a global OOM then ends the CI job, not a VM or the
# nightly. Set on the VM's QEMU process, right after `qm start`.
vm_oom_first() {
  local pid
  pid=$(vm_pid "$1")
  [[ $pid =~ ^[0-9]+$ ]] || { log "VM $1: cannot find its QEMU pid for oom_score_adj"; return 1; }
  echo 1000 > "/proc/$pid/oom_score_adj" 2> /dev/null && [ "$(cat "/proc/$pid/oom_score_adj")" = 1000 ]
}

# --- numbers at the end of a job -----------------------------------------------------------

# vm_stats ID: the memory high-water mark and the oom counts from the host's cgroup files, and the disk
# the clone has written, from the thin pool (`lvs`). Nothing is asked of the guest. Call it before the
# stop, while the cgroup still exists.
vm_stats() {
  local id=$1 cg=/sys/fs/cgroup/qemu.slice/$1.scope peak oom disk
  peak=$(awk '{printf "%d", $1 / 1048576}' "$cg/memory.peak" 2> /dev/null)
  oom=$(awk '$1 == "oom" || $1 == "oom_kill" {printf "%s=%s ", $1, $2}' "$cg/memory.events" 2> /dev/null)
  disk=$(LC_ALL=C lvs --noheadings --nosuffix --units g -o lv_size,data_percent "${POOL%/*}/vm-$id-disk-0" 2> /dev/null |
    awk '{printf "%.0fG of %.0fG", $1 * $2 / 100, $1}')
  printf 'disk written %s, memory.peak %s MiB, %s' "${disk:-?}" "${peak:-?}" "${oom:-oom counts ?}"
}
