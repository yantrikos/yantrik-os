#!/usr/bin/env bash
# Yantrik CI controller: one throwaway runner at a time (#1234).
#
# Loop: wait for memory, ask GitHub for a one-job (JIT) runner config, clone the template
# CT, start it, hand it the config through a root-only file, wait for its runner to take and
# finish its one job, destroy the CT. Nothing a job leaves behind survives it.
#
# Runs as root on the Proxmox node, under yantrik-ci-controller.service. It touches only
# CT ids 9600-9699 (hard-coded), and only ones named yantrik-ci-<id>; it clones only
# template 9530; on the node it touches only its own nft table `bridge yantrik_ci`.
#
# The runner is registered at organisation level, in a runner group `yantrik-ci` that this
# script creates on first use (visible only to the one repository, public repositories
# allowed). The token needs only the organisation permission "Self-hosted runners:
# Read and write". Which jobs the runner accepts is decided twice: inside the CT, by the
# job-started hook, and here, by asking GitHub which job the runner took (check_job).
#
# The GitHub token is read from a root-only file and never printed, logged, put in an
# environment variable that is exported, or passed on a command line. The JIT config lives
# in a shell variable and in a root-only tmpfs file for the moment it takes to push it.
#
# Environment overrides exist for the stub test (README.md); the id range does not have one.
# The functions live in lib/github.sh and lib/host.sh.

set -uo pipefail
# pct and lxc create directories an unprivileged CT's root must be able to traverse
# (/var/lib/lxc/<id>/rootfs); a 077 umask makes the start fail. Secrets are written in
# a subshell with their own umask, and the directories holding them are mode 700.
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
TEMPLATE_ID=${YANTRIK_CI_TEMPLATE:-9530}
POOL=${YANTRIK_CI_POOL:-pve/data}
MIN_MEM_KB=${YANTRIK_CI_MIN_MEM_KB:-41943040} # 40 GiB, checked before a runner and while it idles
JOB_LIMIT_S=${YANTRIK_CI_JOB_LIMIT:-10800}    # 3 h from the moment the runner takes a job
IDLE_LIMIT_S=${YANTRIK_CI_IDLE_LIMIT:-10800}  # 3 h waiting for one: then a fresh runner
IDLE_MIN_MEM_KB=${YANTRIK_CI_IDLE_MIN_MEM_KB:-$MIN_MEM_KB} # an idle runner is recycled below this
POLL_S=${YANTRIK_CI_POLL:-10} # about 360 GitHub calls an hour while a runner idles
RUNLOG_KB=${YANTRIK_CI_RUNLOG_KB:-51200} # 50 MB cap on a run's log (review N4)
RUNLOG_DAYS=${YANTRIK_CI_RUNLOG_DAYS:-30}
BACKOFF_MIN=${YANTRIK_CI_BACKOFF_MIN:-30}
BACKOFF_MAX=${YANTRIK_CI_BACKOFF_MAX:-600}
MAX_CYCLES=${YANTRIK_CI_MAX_CYCLES:-0} # 0 = forever (tests set it)
LABEL=yantrik-build
ID_MIN=9600
ID_MAX=9699

LOG_FILE=$LOG_DIR/controller.log
STOP=0
CHILD_PID=
CUR_ID=
GROUP_ID=
SWEPT=0
WHY=
VERDICT_DONE=0

mkdir -p "$LOG_DIR/runs" "$RUN_DIR"
chmod 700 "$LOG_DIR" "$RUN_DIR"

log() {
  local line
  line="$(date -u +%Y-%m-%dT%H:%M:%SZ) $*"
  printf '%s\n' "$line"
  printf '%s\n' "$line" >> "$LOG_FILE"
}

# nap N: a sleep that a signal can interrupt.
nap() {
  sleep "$1" &
  CHILD_PID=$!
  wait "$CHILD_PID" 2> /dev/null
  CHILD_PID=
}

# trusted_path PATH: root-owned and not writable by group or others. Root sources these files, splices
# /etc/yantrik-ci/own-public-ips into an nft ruleset and includes host-egress.nft, so anything that
# is not root's alone is refused (L3). Used here before sourcing and by lib/host.sh.
trusted_path() {
  local m
  m=$(stat -c '%u %a' "$1" 2> /dev/null) || { log "cannot stat $1"; return 1; }
  if [ "${m%% *}" != 0 ] || [ $((8#${m#* } & 8#022)) -ne 0 ]; then
    log "REFUSING to use $1: owner/mode '$m' (need root, not group- or other-writable)"
    return 1
  fi
}
for f in "$LIB_DIR" "$LIB_DIR/github.sh" "$LIB_DIR/host.sh"; do
  trusted_path "$f" || exit 1
done

# shellcheck source=lib/github.sh
. "$LIB_DIR/github.sh"
# shellcheck source=lib/host.sh
. "$LIB_DIR/host.sh"

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

CLONE_MODE=
clone_ct() {
  local id=$1
  if pct clone "$TEMPLATE_ID" "$id" --hostname "yantrik-ci-$id" > /dev/null 2>&1; then
    CLONE_MODE=linked
  else
    # A failed clone may have left our own half-made CT; destroy_ct leaves anyone else's alone.
    destroy_ct "$id" || true
    if pct clone "$TEMPLATE_ID" "$id" --hostname "yantrik-ci-$id" --full > /dev/null 2>&1; then
      CLONE_MODE=full
    else
      destroy_ct "$id" || true
      return 1
    fi
  fi
}

# Push the config in, as a root-only file read once by the launcher inside the CT.
push_jit() {
  local id=$1 f="$RUN_DIR/jit.$1" ready=0 rc
  for _ in $(seq 1 60); do
    if pct exec "$id" -- install -d -m 700 /run/yantrik-ci > /dev/null 2>&1; then ready=1; break; fi
    sleep 2
  done
  [ "$ready" -eq 1 ] || { log "CT $id never became ready for exec"; return 1; }
  (umask 077 && printf '%s' "$JIT" > "$f")
  chmod 600 "$f"
  pct push "$id" "$f" /run/yantrik-ci/jit --perms 600 --user 0 --group 0 > /dev/null 2>&1
  rc=$?
  rm -f "$f"
  return $rc
}

# idle_recycle_ok: the runner looks idle and should go (memory short, or the idle limit). One more
# look at GitHub first (R12): a job assigned a moment ago is not killed mid-start. Returns 0 to
# recycle, and only on a clear "not busy": busy, or an answer that cannot be read, leaves it (L4).
idle_recycle_ok() {
  local st=0
  runner_busy || st=$?
  [ "$st" -eq 1 ]
}

# wait_for_job PID ID: until the runner exits, the idle limit passes with no job, memory runs short
# while it idles, its job is refused, the host filter is lost, or the job has run past its limit.
# Every POLL_S: the host filter is checked (M4); the runner is asked whether it is busy; once it is,
# its job is looked up on GitHub by runner_name until it is judged (M2). Sets WHY to
# idle|lowmem|limit|stop|refused|filter|"" (the runner exited), and VERDICT_DONE.
wait_for_job() {
  local pid=$1 id=$2 t0 busy_since="" polled_ok=0 now st tries=0
  t0=$(date +%s)
  WHY=
  VERDICT_DONE=0
  while kill -0 "$pid" 2> /dev/null; do
    if [ "$STOP" -ne 0 ]; then WHY=stop; return; fi
    nap "$POLL_S"
    now=$(date +%s)
    if [ "$STOP" -ne 0 ]; then WHY=stop; return; fi
    kill -0 "$pid" 2> /dev/null || break
    if ! host_filter_ok "$id"; then log "the host filter was lost while CT $id ran: ending it"; WHY=filter; return; fi
    if [ -z "$busy_since" ]; then
      st=0
      runner_busy || st=$?
      if [ $st -eq 0 ]; then busy_since=$now; log "runner $RUNNER_NAME took a job"; fi
      if [ $st -ne 2 ]; then polled_ok=1; fi
    fi
    # The job is looked up on GitHub, out of the workflow's reach: found and wrong, or not found
    # in two looks, is a refusal (R1).
    if [ -n "$busy_since" ] && [ "$VERDICT_DONE" -eq 0 ]; then
      st=0
      check_job || st=$?
      case $st in
        0) VERDICT_DONE=1; log "job admitted: $(job_audit_line)" ;;
        1) log "REFUSING the job of runner $RUNNER_NAME: $(job_audit_line): $JOB_VERDICT"; WHY=refused; return ;;
        *)
          tries=$((tries + 1))
          if [ "$tries" -ge 2 ]; then
            log "REFUSING the job of runner $RUNNER_NAME: it could not be found on GitHub in $tries looks"
            WHY=refused
            return
          fi
          ;;
      esac
    fi
    if [ -z "$busy_since" ] && ! mem_idle_ok && idle_recycle_ok; then WHY=lowmem; return; fi
    if [ -n "$busy_since" ]; then
      if [ $((now - busy_since)) -ge "$JOB_LIMIT_S" ]; then WHY=limit; return; fi
    elif [ "$polled_ok" -eq 1 ]; then
      if [ $((now - t0)) -ge "$IDLE_LIMIT_S" ] && idle_recycle_ok; then WHY=idle; return; fi
    elif [ $((now - t0)) -ge "$JOB_LIMIT_S" ]; then
      WHY=limit # GitHub cannot be asked about the runner: fall back to one plain clock
      return
    fi
  done
}

cycle() {
  local id t0 job_rc runlog rcfile cmem out
  read_token || return 1
  ensure_group || return 1
  if [ "$SWEPT" -eq 0 ]; then sweep_runners; SWEPT=1; fi
  id=$(free_id) || { log "no free CT id in $ID_MIN-$ID_MAX"; return 1; }
  pool_ok || return 1
  host_filter_load || { log "REFUSING to clone: the host-side firewall is not in place"; return 1; }
  request_jit || return 1

  CUR_ID=$id
  t0=$(date +%s)
  log "cycle: runner $RUNNER_NAME (GitHub id $RUNNER_GH_ID), CT $id"

  if ! clone_ct "$id"; then
    log "clone of $TEMPLATE_ID to $id failed"
    JIT=
    finish_ct "$id" "$t0" "clone-failed"
    return 1
  fi
  log "CT $id cloned ($CLONE_MODE clone) in $(($(date +%s) - t0)) s"

  # Pin the clone's port and own MAC in the host filter before it exists on the bridge (H1).
  if ! host_filter_add_clone "$id"; then
    log "REFUSING to start CT $id: the host filter could not pin it"
    JIT=
    finish_ct "$id" "$t0" "filter-failed"
    return 1
  fi

  if ! out=$(pct start "$id" 2>&1); then
    out=${out//$'\n'/ }
    log "CT $id failed to start: ${out:0:300}"
    JIT=
    finish_ct "$id" "$t0" "start-failed"
    return 1
  fi

  # Fatal (L1): without it a global OOM on the node can kill a VM or the nightly, not this job.
  if ! ct_oom_first "$id"; then
    log "REFUSING to run in CT $id: its oom_score_adj could not be set"
    JIT=
    finish_ct "$id" "$t0" "oom-adj-failed"
    return 1
  fi

  if ! push_jit "$id"; then
    log "could not hand the config to CT $id"
    JIT=
    finish_ct "$id" "$t0" "push-failed"
    return 1
  fi
  JIT=
  cmem=$(awk '{printf "%d", $1/1048576}' "/sys/fs/cgroup/lxc/$id/memory.current" 2> /dev/null || true)
  log "CT $id running, runner launching (idle memory ${cmem:-?} MiB)"

  runlog="$LOG_DIR/runs/$RUNNER_NAME.log"
  rcfile="$RUN_DIR/rc.$id"
  rm -f "$rcfile"
  # The runner's output goes to a log capped at RUNLOG_KB: a job writing without end into the
  # listener's stdout stops at the cap (`head` exits, `pct exec` gets SIGPIPE, the CT is
  # destroyed) instead of filling node2's /. A `ulimit -f` on `pct exec` itself would be
  # inherited by everything in the CT, and the job's own large files would hit it.
  (
    pct exec "$id" -- /usr/local/sbin/yantrik-ci-launch 2>&1
    echo $? > "$rcfile"
  ) | head -c $((RUNLOG_KB * 1024)) >> "$runlog" &
  CHILD_PID=$!
  wait_for_job "$CHILD_PID" "$id"

  # Stopping the CT ends `pct exec` whatever the runner was doing.
  case $WHY in
    "")
      wait "$CHILD_PID" 2> /dev/null
      job_rc=$(cat "$rcfile" 2> /dev/null || echo "unknown (run log full?)")
      # No verdict yet (a job shorter than a poll): look it up now, so every job leaves an audit line (M2).
      if [ "$VERDICT_DONE" -eq 0 ] && ! audit_job; then job_rc="1 (a job this runner must not take ran: see VIOLATION)"; fi
      ;;
    idle) job_rc="0 (no job in ${IDLE_LIMIT_S}s: recycled)" ;;
    lowmem) job_rc="0 (memory short while idle: recycled)" ;;
    limit) job_rc="124 (hit the ${JOB_LIMIT_S}s limit)" ;;
    refused) job_rc="1 (job refused)" ;;
    filter) job_rc="1 (host filter lost)" ;;
    stop) job_rc="143 (controller stopping)" ;;
  esac
  finish_ct "$id" "$t0" "$job_rc"
  if [ -n "$WHY" ]; then wait "$CHILD_PID" 2> /dev/null; fi
  CHILD_PID=
  rm -f "$rcfile"
  [ "${job_rc%% *}" = 0 ]
}

# finish_ct ID T0 REASON: read the numbers from the host's cgroup and the thin pool (no `pct exec`:
# the CT may be hostile, M3), stop and destroy the CT, drop its pin, deregister the runner, log.
# Retries destroy; never leaves a CT behind silently.
finish_ct() {
  local id=$1 t0=$2 why=$3 stats=
  if lxc_exists "$id"; then stats=$(ct_stats "$id"); fi
  for _ in 1 2 3; do
    destroy_ct "$id" && break
    sleep 5
  done
  if lxc_exists "$id"; then
    log "ERROR: CT $id could not be destroyed; the next start-up cleans it"
  fi
  host_filter_remove_clone "$id"
  remove_runner
  log "done: CT $id, runner ${RUNNER_NAME:-?}, started $(date -u -d "@$t0" +%H:%M:%SZ), ended $(date -u +%H:%M:%SZ), job exit: $why${stats:+; $stats}"
  CUR_ID=
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

  log "controller starting (template $TEMPLATE_ID, ids $ID_MIN-$ID_MAX, org $ORG, repo $REPO)"
  pct config "$TEMPLATE_ID" 2> /dev/null | grep -q '^template: 1' || {
    log "CT $TEMPLATE_ID is not a template: build it with make-template.sh"
    exit 1
  }
  destroy_all_clones || log "start-up cleanup left a CT behind"

  while [ "$STOP" -eq 0 ]; do
    find "$LOG_DIR/runs" -name '*.log' -mtime +"$RUNLOG_DAYS" -delete 2> /dev/null || true
    wait_for_memory || break
    cycle
    rc=$?
    # A CT that survives a cycle is cleaned before the next one starts.
    if [ -n "$CUR_ID" ]; then destroy_ct "$CUR_ID" || true; CUR_ID=; fi
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

  if [ -n "$CUR_ID" ]; then destroy_ct "$CUR_ID" || true; fi
  remove_runner
  log "controller stopped"
}

main "$@"
