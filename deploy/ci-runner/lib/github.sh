# GitHub side of the CI controller (#1234): the token, the API, the org runner group, runner
# registrations and the out-of-band check of the job a runner took. Sourced by
# yantrik-ci-controller.sh, which defines log(), the config variables and the globals used here.
# shellcheck shell=bash disable=SC2034

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

# json_get EXPR [BODY]: evaluate a python expression over parsed JSON (as `d`); prints it.
# BODY defaults to API_BODY. Errors print nothing.
json_get() {
  python3 -c 'import json,sys
d = json.load(sys.stdin)
try:
    r = eval(sys.argv[1])
    if r is not None: print(r)
except Exception:
    pass' "$1" <<< "${2-$API_BODY}"
}

api_message() { json_get 'str(d.get("message",""))[:200]'; }

# --- the runner group ---------------------------------------------------------------------

# Find the org runner group by name, or create it: visibility `selected` with only $REPO,
# public repositories allowed, no workflow restriction (the hook does that). Idempotent. A group
# that is not `selected`, or that lists anything but $REPO, is refused (R8): someone with org
# rights may have widened it.
ensure_group() {
  local found vis repo_id body
  api GET "orgs/$ORG/actions/runner-groups?per_page=100" || { log "runner groups: GitHub unreachable"; return 1; }
  if [ "$API_CODE" != 200 ]; then
    log "listing runner groups failed: HTTP $API_CODE ($(api_message))"
    return 1
  fi
  found=$(json_get "next((str(g['id']) + ' ' + g.get('visibility','')) for g in d.get('runner_groups', []) if g.get('name') == '$GROUP_NAME')")
  if [ -n "$found" ]; then
    GROUP_ID=${found%% *}
    vis=${found#* }
    if [ "$vis" != selected ]; then
      log "REFUSING: runner group $GROUP_NAME ($GROUP_ID) has visibility '$vis', not 'selected'; fix it on GitHub"
      GROUP_ID=
      return 1
    fi
    api GET "orgs/$ORG/actions/runner-groups/$GROUP_ID/repositories?per_page=100" || { GROUP_ID=; return 1; }
    if [ "$API_CODE" != 200 ] ||
      [ "$(json_get "d.get('total_count') == 1 and d['repositories'][0]['full_name'] == '$REPO'")" != True ]; then
      log "REFUSING: runner group $GROUP_NAME ($GROUP_ID) is not limited to $REPO alone (HTTP $API_CODE)"
      GROUP_ID=
      return 1
    fi
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

# --- runner registrations -----------------------------------------------------------------

RUNNER_NAME=
RUNNER_GH_ID=
JIT=

# Delete this cycle's runner registration (best effort), whatever happened to it (N5). A DELETE
# that failed re-arms the offline sweep (R13).
remove_runner() {
  [ -n "$RUNNER_GH_ID" ] || return 0
  if ! api DELETE "orgs/$ORG/actions/runners/$RUNNER_GH_ID" > /dev/null 2>&1 ||
    { [ "$API_CODE" != 204 ] && [ "$API_CODE" != 404 ]; }; then
    SWEPT=0
  fi
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
# only, and on failure GitHub's `message` (which carries no secret). `self-hosted` is sent
# beside our own label in case GitHub does not add it to a JIT runner (R15).
request_jit() {
  local body parsed
  RUNNER_NAME="yantrik-ci-$(date +%s)"
  body=$(printf '{"name":"%s","runner_group_id":%s,"labels":["self-hosted","%s"],"work_folder":"_work"}' \
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

# --- the job the runner took ----------------------------------------------------------------

JOB_RUN="" JOB_WF="" JOB_NAME="" JOB_EVENT="" JOB_HEAD="" JOB_VERDICT=""

# find_job QUERY: list runs with that query (`status=in_progress&per_page=100`, or the newest
# `per_page=30`), then each run's jobs, and stop at the job whose runner_name is this runner. A
# workflow cannot set runner_name. Sets JOB_RUN, JOB_WF, JOB_NAME, JOB_EVENT and JOB_HEAD. Returns
# 0 found, 2 not found (or GitHub unreachable). Nothing from the API goes through the environment
# or a command line (M1): the run list is reduced to four short fields per run, on stdin.
find_job() {
  local runs rid path event head jname
  api GET "repos/$REPO/actions/runs?$1" || return 2
  [ "$API_CODE" = 200 ] || return 2
  runs=$(json_get "'\n'.join('%s|%s|%s|%s' % (r['id'], r.get('path') or '', r.get('event') or '', (r.get('head_repository') or {}).get('full_name') or '') for r in d.get('workflow_runs', []))")
  while IFS='|' read -r rid path event head; do
    [[ $rid =~ ^[0-9]+$ ]] || continue
    api GET "repos/$REPO/actions/runs/$rid/jobs?per_page=100" || continue
    [ "$API_CODE" = 200 ] || continue
    jname=$(json_get "next((chr(35) + (j.get('name') or '') for j in d.get('jobs', []) if j.get('runner_name') == '$RUNNER_NAME'), None)")
    [ -n "$jname" ] || continue
    JOB_RUN=$rid JOB_WF=${path%%@*} JOB_NAME=${jname#\#} JOB_EVENT=$event JOB_HEAD=$head
    return 0
  done <<< "$runs"
  return 2
}

# job_verdict: is the job found by find_job one this runner may take? It must be ci.yml's job
# `test`, from a push, a manual run or a pull request, with a head repository that is $REPO itself.
# Sets JOB_VERDICT to the reasons when it is not. Returns 0 admitted, 1 refused.
job_verdict() {
  local bad=()
  [ "$JOB_WF" = ".github/workflows/ci.yml" ] || bad+=("workflow ${JOB_WF:-?}")
  [ "$JOB_NAME" = test ] || bad+=("job ${JOB_NAME:-?}")
  case $JOB_EVENT in push | workflow_dispatch | pull_request) ;; *) bad+=("event ${JOB_EVENT:-?}") ;; esac
  [ "$JOB_HEAD" = "$REPO" ] || bad+=("head repository ${JOB_HEAD:-?}")
  JOB_VERDICT=
  [ "${#bad[@]}" -eq 0 ] || { local IFS=';'; JOB_VERDICT="${bad[*]}"; return 1; }
  return 0
}

# job_audit_line: the audit record of a job, one line, for every job (M2).
job_audit_line() {
  printf 'audit: runner %s ran run %s: workflow %s, job %s, event %s, head %s' \
    "$RUNNER_NAME" "$JOB_RUN" "${JOB_WF:-?}" "${JOB_NAME:-?}" "${JOB_EVENT:-?}" "${JOB_HEAD:-?}"
}

# check_job: while the job runs. Returns 0 admitted, 1 refused (JOB_VERDICT says why), 2 not
# found yet.
check_job() {
  find_job "status=in_progress&per_page=100" || return 2
  job_verdict
}

# audit_job: when the runner has exited with no verdict (a job shorter than a poll): look the job
# up among the newest runs. Logs the audit line, and a loud one if the job was not allowed.
# Returns 0 admitted or nothing to say, 1 if it was a job this runner must not have taken.
audit_job() {
  if ! find_job "per_page=30"; then
    log "audit: runner $RUNNER_NAME: no job of this runner found among the newest runs"
    return 0
  fi
  if job_verdict; then
    log "$(job_audit_line) (admitted)"
    return 0
  fi
  log "VIOLATION: $(job_audit_line): $JOB_VERDICT; it had already finished"
  return 1
}
