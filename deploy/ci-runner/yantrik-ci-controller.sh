#!/usr/bin/env bash
# Yantrik CI controller: one throwaway runner at a time (#1234).
#
# Loop: wait for memory, ask GitHub for a one-job (JIT) runner config, clone the template
# CT, start it, hand it the config through a root-only file, wait for its runner to take and
# finish its one job, destroy the CT. Nothing a job leaves behind survives it.
#
# Runs as root on the Proxmox node, under yantrik-ci-controller.service. It touches only
# CT ids 9600-9699 (hard-coded), and only ones named yantrik-ci-<id>; it clones only
# template 9530.
#
# The runner is registered at organisation level, in a runner group `yantrik-ci` that this
# script creates on first use (visible only to the one repository, public repositories
# allowed). The token needs only the organisation permission "Self-hosted runners:
# Read and write". Which jobs the runner accepts is decided inside the CT, by the
# job-started hook (ct/yantrik-ci-job-started.sh), not here.
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

ORG=${YANTRIK_CI_ORG:-yantrikos}
REPO=${YANTRIK_CI_REPO:-yantrikos/yantrik-os}
GROUP_NAME=${YANTRIK_CI_GROUP:-yantrik-ci}
API=${YANTRIK_CI_API:-https://api.github.com}
TOKEN_FILE=${YANTRIK_CI_TOKEN_FILE:-/root/.yantrik-ci/token}
LOG_DIR=${YANTRIK_CI_LOG_DIR:-/var/log/yantrik-ci}
RUN_DIR=${YANTRIK_CI_RUN_DIR:-/run/yantrik-ci}
TEMPLATE_ID=${YANTRIK_CI_TEMPLATE:-9530}
MIN_MEM_KB=${YANTRIK_CI_MIN_MEM_KB:-41943040} # 40 GiB
JOB_LIMIT_S=${YANTRIK_CI_JOB_LIMIT:-10800}    # 3 h from the moment the runner takes a job
IDLE_LIMIT_S=${YANTRIK_CI_IDLE_LIMIT:-10800}  # 3 h waiting for one: then a fresh runner
POLL_S=${YANTRIK_CI_POLL:-60}
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

mkdir -p "$LOG_DIR/runs" "$RUN_DIR"
chmod 700 "$LOG_DIR" "$RUN_DIR"

log() {
  local line
  line="$(date -u +%Y-%m-%dT%H:%M:%SZ) $*"
  printf '%s\n' "$line"
  printf '%s\n' "$line" >> "$LOG_FILE"
}

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
# header file (`-H @-`), so it is on no command line and in no file. (bash 5.1+ feeds a here-string
# this small through a pipe, not a temporary file; PVE 8 and 9 ship bash 5.2.)
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

# json_get EXPR: evaluate a python expression over the parsed API_BODY (as `d`); prints it.
# Errors print nothing. Never used on a body that holds a JIT config except to read ids.
json_get() {
  python3 -c 'import json,sys
d = json.load(sys.stdin)
try:
    r = eval(sys.argv[1])
    if r is not None: print(r)
except Exception:
    pass' "$1" <<< "$API_BODY"
}

api_message() { json_get 'str(d.get("message",""))[:200]'; }

# --- the runner group (created on first use) --------------------------------------------

# Find the org runner group by name, or create it: visibility `selected` with only $REPO,
# public repositories allowed, no workflow restriction (the hook does that). Idempotent.
ensure_group() {
  local found repo_id body
  api GET "orgs/$ORG/actions/runner-groups?per_page=100" || { log "runner groups: GitHub unreachable"; return 1; }
  if [ "$API_CODE" != 200 ]; then
    log "listing runner groups failed: HTTP $API_CODE ($(api_message))"
    return 1
  fi
  found=$(json_get "next(([g['id'], g.get('visibility','')] for g in d.get('runner_groups', []) if g.get('name') == '$GROUP_NAME'), None)")
  if [ -n "$found" ]; then
    GROUP_ID=$(python3 -c 'import sys,ast; print(ast.literal_eval(sys.argv[1])[0])' "$found")
    if [ "$(python3 -c 'import sys,ast; print(ast.literal_eval(sys.argv[1])[1])' "$found")" != selected ]; then
      log "WARNING: runner group $GROUP_NAME ($GROUP_ID) is not visibility=selected; fix it on GitHub"
    fi
    log "runner group $GROUP_NAME found (id $GROUP_ID)"
    return 0
  fi
  api GET "repos/$REPO" || { log "repository lookup: GitHub unreachable"; return 1; }
  repo_id=$(json_get "d['id']")
  [[ $repo_id =~ ^[0-9]+$ ]] || { log "cannot read the id of $REPO: HTTP $API_CODE ($(api_message))"; return 1; }
  body=$(printf '{"name":"%s","visibility":"selected","selected_repository_ids":[%s],"allows_public_repositories":true}' \
    "$GROUP_NAME" "$repo_id")
  api POST "orgs/$ORG/actions/runner-groups" "$body" || { log "creating runner group: GitHub unreachable"; return 1; }
  if [ "$API_CODE" != 201 ] && [ "$API_CODE" != 200 ]; then
    log "creating runner group failed: HTTP $API_CODE ($(api_message))"
    return 1
  fi
  GROUP_ID=$(json_get "d['id']")
  [[ $GROUP_ID =~ ^[0-9]+$ ]] || { GROUP_ID=; log "runner group created but no id came back"; return 1; }
  log "runner group $GROUP_NAME created (id $GROUP_ID, only $REPO, public repositories allowed)"
}

RUNNER_NAME=
RUNNER_GH_ID=
JIT=

# Delete this cycle's runner registration (best effort), whatever happened to it (N5).
remove_runner() {
  [ -n "$RUNNER_GH_ID" ] || return 0
  api DELETE "orgs/$ORG/actions/runners/$RUNNER_GH_ID" > /dev/null 2>&1 || true
  RUNNER_GH_ID=
}

# Delete leftover offline runners of ours (a killed controller leaves them behind).
sweep_runners() {
  local ids id n=0
  api GET "orgs/$ORG/actions/runners?per_page=100" || return 0
  [ "$API_CODE" = 200 ] || return 0
  ids=$(json_get "' '.join(str(r['id']) for r in d.get('runners', []) if r.get('name','').startswith('yantrik-ci-') and r.get('status') == 'offline')")
  for id in $ids; do
    api DELETE "orgs/$ORG/actions/runners/$id" > /dev/null 2>&1 && n=$((n + 1))
  done
  [ "$n" -eq 0 ] || log "removed $n offline runner registration(s) left behind"
}

# Ask for a one-job runner. Sets RUNNER_NAME, RUNNER_GH_ID and JIT; logs the status code
# only, and on failure GitHub's `message` (which carries no secret).
request_jit() {
  local body parsed
  RUNNER_NAME="yantrik-ci-$(date +%s)"
  body=$(printf '{"name":"%s","runner_group_id":%s,"labels":["%s"],"work_folder":"_work"}' \
    "$RUNNER_NAME" "$GROUP_ID" "$LABEL")
  if ! api POST "orgs/$ORG/actions/runners/generate-jitconfig" "$body"; then
    log "GitHub API unreachable (curl failed)"
    return 1
  fi
  if [ "$API_CODE" != 201 ] && [ "$API_CODE" != 200 ]; then
    log "generate-jitconfig failed: HTTP $API_CODE ($(api_message))"
    API_BODY=
    return 1
  fi
  RUNNER_GH_ID=$(json_get "d['runner']['id']")
  parsed=$(json_get "d['encoded_jit_config']")
  API_BODY=
  if [ -z "$parsed" ] || ! [[ $RUNNER_GH_ID =~ ^[0-9]+$ ]]; then
    log "generate-jitconfig: no usable config in the answer"
    [[ $RUNNER_GH_ID =~ ^[0-9]+$ ]] || RUNNER_GH_ID=
    remove_runner
    return 1
  fi
  JIT=$parsed
  parsed=
}

# --- one cycle ---------------------------------------------------------------------------

# nap N: a sleep that a signal can interrupt.
nap() {
  sleep "$1" &
  CHILD_PID=$!
  wait "$CHILD_PID" 2> /dev/null
  CHILD_PID=
}

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

# runner_busy: does GitHub say this runner has taken its job? 0 yes, 1 no, 2 cannot tell.
runner_busy() {
  api GET "orgs/$ORG/actions/runners/$RUNNER_GH_ID" || return 2
  [ "$API_CODE" = 200 ] || return 2
  case $(json_get "d.get('busy')") in
    True) return 0 ;;
    False) return 1 ;;
    *) return 2 ;;
  esac
}

# wait_for_job PID RCFILE: until the runner exits, the idle limit passes with no job, or the
# job has run past its limit. Prints nothing; sets WHY to idle|limit|stop|"" (exited).
WHY=
wait_for_job() {
  local pid=$1 t0 busy_since="" tick=0 polled_ok=0 now st
  t0=$(date +%s)
  WHY=
  while kill -0 "$pid" 2> /dev/null; do
    if [ "$STOP" -ne 0 ]; then WHY=stop; return; fi
    nap 5
    tick=$((tick + 5))
    now=$(date +%s)
    if [ -z "$busy_since" ] && [ $((tick % POLL_S)) -lt 5 ]; then
      runner_busy
      st=$?
      if [ $st -eq 0 ]; then busy_since=$now; log "runner $RUNNER_NAME took a job"; fi
      if [ $st -ne 2 ]; then polled_ok=1; fi
    fi
    if [ -n "$busy_since" ]; then
      if [ $((now - busy_since)) -ge "$JOB_LIMIT_S" ]; then WHY=limit; return; fi
    elif [ "$polled_ok" -eq 1 ]; then
      if [ $((now - t0)) -ge "$IDLE_LIMIT_S" ]; then WHY=idle; return; fi
    elif [ $((now - t0)) -ge "$JOB_LIMIT_S" ]; then
      WHY=limit # GitHub cannot be asked about the runner: fall back to one plain clock
      return
    fi
  done
}

cycle() {
  local id t0 job_rc runlog rcfile cmem out
  read_token || return 1
  if [ -z "$GROUP_ID" ]; then ensure_group || return 1; fi
  if [ "$SWEPT" -eq 0 ]; then sweep_runners; SWEPT=1; fi
  id=$(free_id) || { log "no free CT id in $ID_MIN-$ID_MAX"; return 1; }
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
  wait_for_job "$CHILD_PID"

  # Stopping the CT ends `pct exec` whatever the runner was doing.
  case $WHY in
    "") wait "$CHILD_PID" 2> /dev/null; job_rc=$(cat "$rcfile" 2> /dev/null || echo "unknown (run log full?)") ;;
    idle) job_rc="0 (no job in ${IDLE_LIMIT_S}s: recycled)" ;;
    limit) job_rc="124 (hit the ${JOB_LIMIT_S}s limit)" ;;
    stop) job_rc="143 (controller stopping)" ;;
  esac
  finish_ct "$id" "$t0" "$job_rc"
  if [ -n "$WHY" ]; then wait "$CHILD_PID" 2> /dev/null; fi
  CHILD_PID=
  rm -f "$rcfile"
  [ "${job_rc%% *}" = 0 ]
}

# finish_ct ID T0 REASON: stop, destroy, deregister, log. Retries destroy; never leaves a CT
# behind silently.
finish_ct() {
  local id=$1 t0=$2 why=$3
  for _ in 1 2 3; do
    destroy_ct "$id" && break
    sleep 5
  done
  if lxc_exists "$id"; then
    log "ERROR: CT $id could not be destroyed; the next start-up cleans it"
  fi
  remove_runner
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

  log "controller starting (template $TEMPLATE_ID, ids $ID_MIN-$ID_MAX, org $ORG, repo $REPO)"
  pct config "$TEMPLATE_ID" 2> /dev/null | grep -q '^template: 1' || {
    log "CT $TEMPLATE_ID is not a template: build it with make-template.sh"
    exit 1
  }
  destroy_all_clones || log "start-up cleanup left a CT behind"
  find "$LOG_DIR/runs" -name '*.log' -mtime +"$RUNLOG_DAYS" -delete 2> /dev/null || true

  while [ "$STOP" -eq 0 ]; do
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
