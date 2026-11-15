#!/usr/bin/env bash
# Yantrik CI controller: one throwaway runner at a time (#1234).
#
# Loop: wait for memory, ask GitHub for a one-job (JIT) runner config, clone the template
# CT, start it, hand it the config through a root-only file, wait for its runner to exit
# (its one job is done), destroy the CT. Nothing a job leaves behind survives it.
#
# Runs as root on the Proxmox node, under yantrik-ci-controller.service. It touches only
# CT ids 9600-9699 (hard-coded) and clones only template 9530.
#
# The GitHub token is read from a root-only file and never printed, logged, put in an
# environment variable that is exported, or passed on a command line. The JIT config lives
# in a shell variable and in a root-only tmpfs file for the moment it takes to push it.
#
# Environment overrides exist for the stub test (README.md); the id range does not have one.

set -uo pipefail
# pct and lxc create directories an unprivileged CT's root must be able to traverse
# (/var/lib/lxc/<id>/rootfs); a 077 umask makes the start fail. Secrets are written in
# a subshell with their own umask, and the directories holding them are mode 700.
umask 022

REPO=${YANTRIK_CI_REPO:-yantrikos/yantrik-os}
API=${YANTRIK_CI_API:-https://api.github.com}
TOKEN_FILE=${YANTRIK_CI_TOKEN_FILE:-/root/.yantrik-ci/token}
LOG_DIR=${YANTRIK_CI_LOG_DIR:-/var/log/yantrik-ci}
RUN_DIR=${YANTRIK_CI_RUN_DIR:-/run/yantrik-ci}
TEMPLATE_ID=${YANTRIK_CI_TEMPLATE:-9530}
MIN_MEM_KB=${YANTRIK_CI_MIN_MEM_KB:-41943040} # 40 GiB
JOB_LIMIT_S=${YANTRIK_CI_JOB_LIMIT:-10800}    # 3 h
BACKOFF_MIN=${YANTRIK_CI_BACKOFF_MIN:-30}
BACKOFF_MAX=${YANTRIK_CI_BACKOFF_MAX:-600}
MAX_CYCLES=${YANTRIK_CI_MAX_CYCLES:-0} # 0 = forever (tests set it)
RUNNER_GROUP_ID=1
LABEL=yantrik-build
ID_MIN=9600
ID_MAX=9699

LOG_FILE=$LOG_DIR/controller.log
STOP=0
CHILD_PID=
CUR_ID=

mkdir -p "$LOG_DIR" "$RUN_DIR"
chmod 700 "$LOG_DIR" "$RUN_DIR"

log() {
  local line
  line="$(date -u +%Y-%m-%dT%H:%M:%SZ) $*"
  printf '%s\n' "$line"
  printf '%s\n' "$line" >> "$LOG_FILE"
}

# --- ids ------------------------------------------------------------------------------

id_in_use() { compgen -G "/etc/pve/nodes/*/lxc/$1.conf" > /dev/null || compgen -G "/etc/pve/nodes/*/qemu-server/$1.conf" > /dev/null; }

free_id() {
  local id
  for ((id = ID_MIN; id <= ID_MAX; id++)); do
    if ! id_in_use "$id"; then printf '%s\n' "$id"; return 0; fi
  done
  return 1
}

in_range() { [[ $1 =~ ^[0-9]+$ ]] && [ "$1" -ge "$ID_MIN" ] && [ "$1" -le "$ID_MAX" ]; }

# Stop and destroy one clone. Refuses anything outside the range or the template itself.
destroy_ct() {
  local id=$1
  in_range "$id" || { log "refusing to destroy $id: outside $ID_MIN-$ID_MAX"; return 1; }
  [ "$id" != "$TEMPLATE_ID" ] || return 1
  compgen -G "/etc/pve/nodes/*/lxc/$id.conf" > /dev/null || return 0
  pct stop "$id" > /dev/null 2>&1 || true
  pct destroy "$id" --purge > /dev/null 2>&1 || true
  ! compgen -G "/etc/pve/nodes/*/lxc/$id.conf" > /dev/null
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

# --- the token and GitHub ----------------------------------------------------------------

TOKEN=
read_token() {
  local meta
  [ -f "$TOKEN_FILE" ] || { log "no token at $TOKEN_FILE: waiting for the token"; return 1; }
  meta=$(stat -c '%u %a' "$TOKEN_FILE")
  case "$meta" in
    "0 600" | "0 400") ;;
    *) log "refusing the token file: owner/mode is '$meta', need root and 600"; return 2 ;;
  esac
  TOKEN=$(< "$TOKEN_FILE")
  TOKEN=${TOKEN//[$' \t\r\n']/}
  [ -n "$TOKEN" ] || { log "token file is empty"; return 1; }
}

API_BODY=
API_CODE=000
# api METHOD PATH [JSON]  -> sets API_CODE and API_BODY. The token goes in on stdin as a
# header file (`-H @-`), so it is on no command line and in no file.
api() {
  local method=$1 path=$2 data=${3:-} out rc
  local -a args=(-sS --max-time 60 -X "$method" -H @- -w $'\n%{http_code}')
  if [ -n "$data" ]; then args+=(-H 'Content-Type: application/json' -d "$data"); fi
  out=$(curl "${args[@]}" -H 'Accept: application/vnd.github+json' -H 'X-GitHub-Api-Version: 2022-11-28' \
    "$API/$path" 2> /dev/null <<< "Authorization: Bearer $TOKEN")
  rc=$?
  if [ $rc -ne 0 ] && [ -z "$out" ]; then API_CODE=000; API_BODY=; return 1; fi
  API_CODE=${out##*$'\n'}
  API_BODY=${out%$'\n'*}
  return 0
}

RUNNER_NAME=
RUNNER_GH_ID=
JIT=
# Ask for a one-job runner. Sets RUNNER_NAME, RUNNER_GH_ID and JIT; logs the status code
# only, and on failure GitHub's `message` (which carries no secret).
request_jit() {
  local body parsed
  RUNNER_NAME="yantrik-ci-$(date +%s)"
  body=$(printf '{"name":"%s","runner_group_id":%s,"labels":["%s"],"work_folder":"_work"}' \
    "$RUNNER_NAME" "$RUNNER_GROUP_ID" "$LABEL")
  if ! api POST "repos/$REPO/actions/runners/generate-jitconfig" "$body"; then
    log "GitHub API unreachable (curl failed)"
    return 1
  fi
  if [ "$API_CODE" != 201 ] && [ "$API_CODE" != 200 ]; then
    local msg
    msg=$(python3 -c 'import json,sys
try: print(str(json.load(sys.stdin).get("message",""))[:200])
except Exception: print("")' <<< "$API_BODY")
    log "generate-jitconfig failed: HTTP $API_CODE ${msg:+($msg)}"
    API_BODY=
    return 1
  fi
  parsed=$(python3 -c 'import json,sys
d = json.load(sys.stdin)
print(d["runner"]["id"]); print(d["encoded_jit_config"])' <<< "$API_BODY") || parsed=
  API_BODY=
  RUNNER_GH_ID=${parsed%%$'\n'*}
  JIT=${parsed#*$'\n'}
  parsed=
  if [ -z "$JIT" ] || [ "$JIT" = "$RUNNER_GH_ID" ]; then
    log "generate-jitconfig: no config in the answer"
    JIT=
    return 1
  fi
}

# --- one cycle ---------------------------------------------------------------------------

wait_for_memory() {
  local avail waited=0
  while [ "$STOP" -eq 0 ]; do
    avail=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
    if [ "${avail:-0}" -ge "$MIN_MEM_KB" ]; then return 0; fi
    if [ $((waited % 600)) -eq 0 ]; then
      log "waiting for memory: ${avail:-0} kB available, need $MIN_MEM_KB kB"
    fi
    nap 30
    waited=$((waited + 30))
  done
  return 1
}

# nap N: a sleep that a signal can interrupt.
nap() {
  sleep "$1" &
  CHILD_PID=$!
  wait "$CHILD_PID" 2> /dev/null
  CHILD_PID=
}

CLONE_MODE=
clone_ct() {
  local id=$1
  if pct clone "$TEMPLATE_ID" "$id" --hostname "yantrik-ci-$id" > /dev/null 2>&1; then
    CLONE_MODE=linked
  else
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
  local id=$1 f="$RUN_DIR/jit.$1" ready=0
  for _ in $(seq 1 60); do
    if pct exec "$id" -- install -d -m 700 /run/yantrik-ci > /dev/null 2>&1; then ready=1; break; fi
    sleep 2
  done
  [ "$ready" -eq 1 ] || { log "CT $id never became ready for exec"; return 1; }
  (umask 077 && printf '%s' "$JIT" > "$f")
  chmod 600 "$f"
  pct push "$id" "$f" /run/yantrik-ci/jit --perms 600 --user 0 --group 0 > /dev/null 2>&1
  local rc=$?
  rm -f "$f"
  return $rc
}

cycle() {
  local id t0 job_rc runlog cmem out
  free_id > /dev/null || { log "no free CT id in $ID_MIN-$ID_MAX"; return 1; }
  read_token || return 1
  request_jit || return 1

  id=$(free_id)
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

  if ! out=$(pct start "$id" 2>&1); then
    out=${out//$'\n'/ }
    log "CT $id failed to start: ${out:0:300}"
    JIT=
    finish_ct "$id" "$t0" "start-failed"
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
  mkdir -p "$LOG_DIR/runs"
  timeout --kill-after=60 "$JOB_LIMIT_S" pct exec "$id" -- /usr/local/sbin/yantrik-ci-launch \
    >> "$runlog" 2>&1 &
  CHILD_PID=$!
  wait "$CHILD_PID"
  job_rc=$?
  CHILD_PID=
  [ "$job_rc" -ne 124 ] || job_rc="124 (hit the ${JOB_LIMIT_S}s limit)"

  finish_ct "$id" "$t0" "$job_rc"
  # A runner that is gone from the CT should be gone from GitHub: best effort.
  api DELETE "repos/$REPO/actions/runners/$RUNNER_GH_ID" > /dev/null 2>&1 || true
  [ "${job_rc%% *}" = 0 ]
}

# finish_ct ID T0 REASON: stop, destroy, log. Retries destroy; never leaves a CT behind
# silently.
finish_ct() {
  local id=$1 t0=$2 why=$3
  for _ in 1 2 3; do
    destroy_ct "$id" && break
    sleep 5
  done
  if compgen -G "/etc/pve/nodes/*/lxc/$id.conf" > /dev/null; then
    log "ERROR: CT $id could not be destroyed; the next start-up cleans it"
  fi
  log "done: CT $id, runner ${RUNNER_NAME:-?}, started $(date -u -d "@$t0" +%H:%M:%SZ), ended $(date -u +%H:%M:%SZ), job exit: $why"
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

  log "controller starting (template $TEMPLATE_ID, ids $ID_MIN-$ID_MAX, repo $REPO)"
  pct config "$TEMPLATE_ID" 2> /dev/null | grep -q '^template: 1' || {
    log "CT $TEMPLATE_ID is not a template: build it with make-template.sh"
    exit 1
  }
  destroy_all_clones || log "start-up cleanup left a CT behind"

  while [ "$STOP" -eq 0 ]; do
    wait_for_memory || break
    cycle
    rc=$?
    # A CT that survives a cycle is cleaned before the next one starts.
    if [ -n "$CUR_ID" ]; then destroy_ct "$CUR_ID" || true; CUR_ID=; fi
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
  log "controller stopped"
}

main "$@"
