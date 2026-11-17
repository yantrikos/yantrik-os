#!/usr/bin/env bash
# Yantrik CI controller: one throwaway runner VM at a time (#1234, #1252).
#
# Loop: wait for memory, ask GitHub for a one-job (JIT) runner config, clone the template VM, start
# it on the private bridge yci0, hand it the config through the QEMU guest agent, wait for its runner
# to take and finish its one job (the guest powers itself off), destroy the VM. Nothing a job leaves
# behind survives it, and a kernel bug in a job gives root only inside a VM that is then destroyed.
#
# Runs as root on the Proxmox node, under yantrik-ci-controller.service. It touches only VM ids
# 9541-9549 (hard-coded), and only ones named yantrik-ci-<id>; it clones only template 9540; on the
# node it makes and removes only the bridge yci0 and its own nft tables `inet yantrik_ci` and
# `netdev yantrik_ci_l2` (lib/net.sh). If it dies (a lost SIGTERM, SIGKILL, an OOM kill) the unit's
# ExecStopPost= runs `yantrik-ci-net cleanup`, which destroys the clones and removes the network.
#
# The runner is registered at organisation level, in a runner group `yantrik-ci` that this
# script creates on first use (visible only to the one repository, public repositories
# allowed). The token needs only the organisation permission "Self-hosted runners:
# Read and write". Which jobs the runner accepts is decided twice: inside the VM, by the
# job-started hook, and here, by asking GitHub which job the runner took (check_job).
#
# The GitHub token is read from a root-only file and never printed, logged, put in an
# environment variable that is exported, or passed on a command line. The JIT config lives
# in a shell variable and goes to the guest on the stdin of a guest-agent command.
#
# Environment overrides exist for the stub test (README.md); the id range does not have one.
# The functions live in lib/github.sh, lib/host.sh and lib/net.sh.

set -uo pipefail
# Secrets are written in a subshell with their own umask, and the directories holding them are
# mode 700.
umask 022

ORG=${YANTRIK_CI_ORG:-yantrikos}
REPO=${YANTRIK_CI_REPO:-yantrikos/yantrik-os}
GROUP_NAME=${YANTRIK_CI_GROUP:-yantrik-ci}
API=${YANTRIK_CI_API:-https://api.github.com}
TOKEN_FILE=${YANTRIK_CI_TOKEN_FILE:-/root/.yantrik-ci/token}
LOG_DIR=${YANTRIK_CI_LOG_DIR:-/var/log/yantrik-ci}
RUN_DIR=${YANTRIK_CI_RUN_DIR:-/run/yantrik-ci}
LIB_DIR=${YANTRIK_CI_LIB:-/usr/local/lib/yantrik-ci}
OWN_PUBLIC_FILE=${YANTRIK_CI_OWN_PUBLIC:-/etc/yantrik-ci/own-public-ips}
TEMPLATE_ID=${YANTRIK_CI_TEMPLATE:-9540}
POOL=${YANTRIK_CI_POOL:-pve/data}
MIN_MEM_KB=${YANTRIK_CI_MIN_MEM_KB:-41943040} # 40 GiB, checked before a runner and while it idles
JOB_LIMIT_S=${YANTRIK_CI_JOB_LIMIT:-10800}    # 3 h from the moment the runner takes a job
IDLE_LIMIT_S=${YANTRIK_CI_IDLE_LIMIT:-10800}  # 3 h waiting for one: then a fresh runner
IDLE_MIN_MEM_KB=${YANTRIK_CI_IDLE_MIN_MEM_KB:-$MIN_MEM_KB} # an idle runner is recycled below this
APILOST_S=${YANTRIK_CI_APILOST:-300} # no answer from GitHub for this long: end the VM
JOB_LOOKS=${YANTRIK_CI_LOOKS:-4} # looks for the job before it is refused (N4)
POLL_S=${YANTRIK_CI_POLL:-10} # about 360 GitHub calls an hour while a runner idles
BACKOFF_MIN=${YANTRIK_CI_BACKOFF_MIN:-30}
BACKOFF_MAX=${YANTRIK_CI_BACKOFF_MAX:-600}
MAX_CYCLES=${YANTRIK_CI_MAX_CYCLES:-0} # 0 = forever (tests set it)
LABEL=yantrik-build
ID_MIN=9541
ID_MAX=9549

LOG_FILE=$LOG_DIR/controller.log
STOP_FILE=$RUN_DIR/stop # ExecStop= of the unit makes it: a stop request that does not rest on a signal trap (L1)
STOP=0
CHILD_PID=
RUNNER_PID=
SURVIVOR=0
NET_UP=0
GROUP_ID=
SWEPT=0
WHY=
VERDICT_DONE=0

mkdir -p "$LOG_DIR" "$RUN_DIR"
chmod 700 "$LOG_DIR" "$RUN_DIR"

log() {
  local line
  line="$(date -u +%Y-%m-%dT%H:%M:%SZ) $*"
  printf '%s\n' "$line"
  printf '%s\n' "$line" >> "$LOG_FILE"
}

# stop_requested: the stop file is there (a SIGTERM can be lost in bash if it arrives while the main shell is
# parsing a command substitution; the file cannot). Sets STOP; always returns 0.
stop_requested() {
  if [ -e "$STOP_FILE" ]; then STOP=1; fi
  return 0
}

# nap N: a sleep that a signal can interrupt (and that sees the stop file when it ends).
nap() {
  sleep "$1" &
  CHILD_PID=$!
  wait "$CHILD_PID" 2> /dev/null
  CHILD_PID=
  stop_requested
}

# The libraries are sourced as root, so they must be root's alone (L3). trust.sh is the check itself:
# its own owner and mode are tested here, in line, before it is loaded; it then tests the rest.
for f in "$LIB_DIR" "$LIB_DIR/trust.sh"; do
  m=$(stat -c '%u %a' "$f" 2> /dev/null) || { log "cannot stat $f"; exit 1; }
  if [ "${m%% *}" != 0 ] || [ $((8#${m#* } & 8#022)) -ne 0 ]; then log "REFUSING to use $f: owner/mode '$m'"; exit 1; fi
done
# shellcheck source=lib/trust.sh
. "$LIB_DIR/trust.sh"
for f in "$LIB_DIR/github.sh" "$LIB_DIR/host.sh" "$LIB_DIR/net.sh"; do
  trusted_path "$f" || exit 1
done

# shellcheck source=lib/github.sh
. "$LIB_DIR/github.sh"
# shellcheck source=lib/host.sh
. "$LIB_DIR/host.sh"
# shellcheck source=lib/net.sh
. "$LIB_DIR/net.sh"

# --- one cycle ---------------------------------------------------------------------------

wait_for_memory() {
  local waited=0
  while [ "$STOP" -eq 0 ]; do
    if mem_ok; then return 0; fi
    if [ $((waited % 600)) -eq 0 ]; then
      log "waiting for memory: $(mem_available_kb) kB available, need $MIN_MEM_KB kB"
    fi
    nap 30
    waited=$((waited + 30))
  done
  return 1
}

# idle_recycle_ok: the runner looks idle and should go (memory short, or the idle limit). One more
# look at GitHub first (R12): a job assigned a moment ago is not killed mid-start. Returns 0 to
# recycle, and only on a clear "not busy": busy, or an answer that cannot be read, leaves it (L4).
idle_recycle_ok() {
  local st=0
  runner_busy || st=$?
  [ "$st" -eq 1 ]
}

# vm_watch ID PARENT: a background watcher that ends when the VM is no longer running (the guest powers
# itself off when its runner exits) or the controller is gone. Its pid is what wait_for_job waits on.
vm_watch() {
  local id=$1 parent=$2
  trap 'exit 0' TERM INT
  while vm_running "$id" && kill -0 "$parent" 2> /dev/null; do
    sleep 2 &
    wait $!
  done
}

# wait_for_job PID ID: until the VM stops, the idle limit passes with no job, memory runs short
# while it idles, its job is refused, the network filter is lost, GitHub has not answered for APILOST_S,
# or the job has run past its limit. Every POLL_S: the filter is checked (M4); the runner is
# asked whether it is busy; once it is, its job is looked up on GitHub by runner_name until it is
# judged (M2); and if GitHub cannot say whether it is busy, the job is looked up anyway (N2). Sets WHY
# to idle|lowmem|limit|stop|refused|filter|apilost|"" (the VM stopped by itself), and VERDICT_DONE.
wait_for_job() {
  local pid=$1 id=$2 t0 busy_since="" last_ok now st tries=0 cj
  t0=$(date +%s)
  last_ok=$t0
  WHY=
  VERDICT_DONE=0
  while kill -0 "$pid" 2> /dev/null; do
    if [ "$STOP" -ne 0 ]; then WHY=stop; return; fi
    nap "$POLL_S"
    now=$(date +%s)
    if [ "$STOP" -ne 0 ]; then WHY=stop; return; fi
    kill -0 "$pid" 2> /dev/null || break
    if ! net_ok "$id"; then log "the network filter was lost while VM $id ran: ending it"; WHY=filter; return; fi
    st=0
    runner_busy || st=$?
    if [ $st -ne 2 ]; then last_ok=$now; fi
    if [ $st -eq 0 ] && [ -z "$busy_since" ]; then busy_since=$now; log "runner $RUNNER_NAME took a job"; fi
    # The job is looked up on GitHub, out of the workflow's reach: found and wrong, or not found in
    # JOB_LOOKS looks, is a refusal (R1). When GitHub cannot say whether the runner is busy, look
    # anyway: a job found is judged at once (N2).
    if { [ -n "$busy_since" ] || [ $st -eq 2 ]; } && [ "$VERDICT_DONE" -eq 0 ]; then
      cj=0
      check_job || cj=$?
      case $cj in
        0)
          VERDICT_DONE=1
          [ -n "$busy_since" ] || { busy_since=$now; last_ok=$now; log "runner $RUNNER_NAME took a job"; }
          log "job admitted: $(job_audit_line)"
          ;;
        1) log "REFUSING the job of runner $RUNNER_NAME: $(job_audit_line): $JOB_VERDICT"; WHY=refused; return ;;
        *)
          if [ -n "$busy_since" ]; then
            tries=$((tries + 1))
            if [ "$tries" -ge "$JOB_LOOKS" ]; then
              log "REFUSING the job of runner $RUNNER_NAME: it could not be found on GitHub in $tries looks"
              WHY=refused
              return
            fi
          fi
          ;;
      esac
    fi
    if [ $((now - last_ok)) -ge "$APILOST_S" ]; then
      log "GitHub has not answered about runner $RUNNER_NAME for $((now - last_ok)) s: ending VM $id"
      WHY=apilost
      return
    fi
    if [ -z "$busy_since" ] && ! mem_idle_ok && idle_recycle_ok; then WHY=lowmem; return; fi
    if [ -n "$busy_since" ]; then
      if [ $((now - busy_since)) -ge "$JOB_LIMIT_S" ]; then WHY=limit; return; fi
    elif [ $((now - t0)) -ge "$IDLE_LIMIT_S" ] && idle_recycle_ok; then
      WHY=idle
      return
    fi
    # An absolute cap, whatever GitHub says.
    if [ $((now - t0)) -ge $((IDLE_LIMIT_S + JOB_LIMIT_S)) ]; then WHY=limit; return; fi
  done
}

cycle() {
  local id t0 tc job_rc cmem
  read_token || return 1
  ensure_group || return 1
  if [ "$SWEPT" -eq 0 ]; then sweep_runners; SWEPT=1; fi
  id=$(free_id) || { log "no free VM id in $ID_MIN-$ID_MAX"; return 1; }
  pool_ok || return 1
  net_up || { log "REFUSING to clone: the clones' network and its filter are not in place"; return 1; }
  NET_UP=1
  request_jit || return 1

  t0=$(date +%s)
  log "cycle: runner $RUNNER_NAME (GitHub id $RUNNER_GH_ID), VM $id"

  if ! clone_vm "$id"; then
    log "clone of $TEMPLATE_ID to $id failed"
    JIT=
    finish_vm "$id" "$t0" "clone-failed"
    return 1
  fi
  tc=$(date +%s)
  log "VM $id cloned ($CLONE_MODE clone) in $((tc - t0)) s"

  # Its one card on yci0 with its fixed address, then its port, MAC and address pinned in the filter,
  # before it exists on the bridge (H1, N3).
  if ! clone_configure "$id"; then
    JIT=
    finish_vm "$id" "$t0" "config-failed"
    return 1
  fi
  if ! net_pin "$id"; then
    log "REFUSING to start VM $id: the filter could not pin it"
    JIT=
    finish_vm "$id" "$t0" "filter-failed"
    return 1
  fi

  if ! qm start "$id" > /dev/null 2>&1; then
    log "VM $id failed to start"
    JIT=
    finish_vm "$id" "$t0" "start-failed"
    return 1
  fi

  # Its port isolated and locked on the bridge, before any job code can exist in it.
  if ! net_lock "$id"; then
    log "REFUSING to run in VM $id: its port could not be isolated and locked"
    JIT=
    finish_vm "$id" "$t0" "lock-failed"
    return 1
  fi

  # Fatal (L1): without it a global OOM on the node can kill a VM or the nightly, not this job.
  if ! vm_oom_first "$id"; then
    log "REFUSING to run in VM $id: its oom_score_adj could not be set"
    JIT=
    finish_vm "$id" "$t0" "oom-adj-failed"
    return 1
  fi

  if ! push_jit "$id"; then
    log "could not hand the config to VM $id"
    JIT=
    finish_vm "$id" "$t0" "push-failed"
    return 1
  fi
  JIT=
  # Nothing uses the guest agent from here on (and the launcher stops it inside): take its device away from the
  # host's side too, so whatever a hostile guest root says to it later reaches no caller (L3). Best effort.
  vm_agent_close "$id" || log "VM $id: the agent stays attached (the launcher has stopped it inside)"
  cmem=$(awk '{printf "%d", $1/1048576}' "/sys/fs/cgroup/qemu.slice/$id.scope/memory.current" 2> /dev/null || true)
  log "VM $id running, runner launching (idle memory ${cmem:-?} MiB)"

  vm_watch "$id" "$$" &
  RUNNER_PID=$!
  wait_for_job "$RUNNER_PID" "$id"

  # Destroying the VM ends the watcher whatever the runner was doing.
  case $WHY in
    "")
      wait "$RUNNER_PID" 2> /dev/null
      job_rc="0 (the guest powered off)"
      # No verdict yet (a job shorter than a poll): look it up now, so every job leaves an audit line (M2).
      if [ "$VERDICT_DONE" -eq 0 ] && ! audit_job; then job_rc="1 (a job this runner must not take ran: see VIOLATION)"; fi
      ;;
    idle) job_rc="0 (no job in ${IDLE_LIMIT_S}s: recycled)" ;;
    lowmem) job_rc="0 (memory short while idle: recycled)" ;;
    limit) job_rc="124 (hit the ${JOB_LIMIT_S}s limit)" ;;
    refused) job_rc="1 (job refused)" ;;
    filter) job_rc="1 (network filter lost)" ;;
    apilost) job_rc="1 (GitHub did not answer)" ;;
    stop) job_rc="143 (controller stopping)" ;;
  esac
  finish_vm "$id" "$t0" "$job_rc"
  if [ -n "$RUNNER_PID" ]; then kill "$RUNNER_PID" 2> /dev/null || true; wait "$RUNNER_PID" 2> /dev/null; fi
  RUNNER_PID=
  [ "${job_rc%% *}" = 0 ]
}

# finish_vm ID T0 REASON: read the numbers from the host's cgroup and the thin pool (nothing is asked of
# the guest, M3), stop and destroy the VM, drop its pin, deregister the runner, log. Retries destroy;
# never leaves a VM behind silently.
finish_vm() {
  local id=$1 t0=$2 why=$3 stats=
  if vm_running "$id"; then stats=$(vm_stats "$id"); fi
  for _ in 1 2 3; do
    destroy_vm "$id" && break
    sleep 5
  done
  if vm_exists "$id"; then
    # A VM that survives is unpinned, so the table (left loaded) drops its port and it has no network;
    # the controller stops and exits 3, which the unit does not restart (N1, L1, L2).
    log "ERROR: VM $id could not be destroyed: unpinning it, stopping the controller, the network filter stays loaded"
    net_unpin "$id"
    SURVIVOR=1
    STOP=1
  fi
  remove_runner
  log "done: VM $id, runner ${RUNNER_NAME:-?}, started $(date -u -d "@$t0" +%H:%M:%SZ), ended $(date -u +%H:%M:%SZ), job exit: $why${stats:+; $stats}"
}

# --- signals and main --------------------------------------------------------------------

on_term() {
  STOP=1
  if [ -n "$CHILD_PID" ]; then kill "$CHILD_PID" 2> /dev/null || true; fi
}
trap on_term TERM INT

main() {
  local backoff=$BACKOFF_MIN cycles=0 rc

  exec 8> "$RUN_DIR/controller.lock"
  flock -n 8 || { log "another controller is running"; exit 1; }
  rm -f "$STOP_FILE"

  log "controller starting (template $TEMPLATE_ID, ids $ID_MIN-$ID_MAX, org $ORG, repo $REPO)"
  qm config "$TEMPLATE_ID" 2> /dev/null | grep -q '^template: 1' || {
    log "VM $TEMPLATE_ID is not a template: build it with make-template.sh"
    exit 1
  }
  if ! destroy_all_clones; then
    log "ERROR: a VM in $ID_MIN-$ID_MAX survived the start-up cleanup: exiting 3, the network is left alone"
    exit 3
  fi

  while [ "$STOP" -eq 0 ]; do
    wait_for_memory || break
    cycle
    rc=$?
    remove_runner
    [ "$STOP" -eq 0 ] || break
    cycles=$((cycles + 1))
    if [ "$MAX_CYCLES" -gt 0 ] && [ "$cycles" -ge "$MAX_CYCLES" ]; then
      log "reached $MAX_CYCLES cycle(s): stopping"
      break
    fi
    if [ $rc -eq 0 ]; then
      backoff=$BACKOFF_MIN
    else
      log "backing off ${backoff}s"
      nap "$backoff"
      backoff=$((backoff * 2))
      [ "$backoff" -le "$BACKOFF_MAX" ] || backoff=$BACKOFF_MAX
    fi
  done

  remove_runner
  # At stop the bridge and the table go (a VM that survived keeps them: its port is dropped until then).
  if [ "$SURVIVOR" -eq 0 ] && [ "$NET_UP" -eq 1 ]; then net_down || log "ERROR: the network could not be removed: run yantrik-ci-net down"; fi
  log "controller stopped"
  [ "$SURVIVOR" -eq 0 ] || exit 3
}

main "$@"
