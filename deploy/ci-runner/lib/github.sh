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

JOB_VERDICT=
# check_job: find the job this runner took through the public API, which a workflow cannot
# influence (R1: the hook alone could be bypassed through BASH_ENV). Returns 0 admitted,
# 1 refused, 2 not found (yet). It requires ci.yml, the job `test`, an event that is a push, a
# manual run or a pull request, and a head repository that is $REPO itself. Sets JOB_VERDICT.
check_job() {
  local runs rids rid jobs verdict
  JOB_VERDICT=
  api GET "repos/$REPO/actions/runs?status=in_progress&per_page=100" || return 2
  [ "$API_CODE" = 200 ] || return 2
  runs=$API_BODY
  rids=$(json_get "' '.join(str(r['id']) for r in d.get('workflow_runs', []))" "$runs")
  for rid in $rids; do
    api GET "repos/$REPO/actions/runs/$rid/jobs?per_page=100" || continue
    [ "$API_CODE" = 200 ] || continue
    jobs=$API_BODY
    verdict=$(RUNS_JSON=$runs RID=$rid RUNNER=$RUNNER_NAME REPO_FULL=$REPO python3 -c '
import json, os, sys
jobs = json.load(sys.stdin).get("jobs", [])
mine = [j for j in jobs if j.get("runner_name") == os.environ["RUNNER"]]
if not mine:
    print("none"); sys.exit()
run = next(r for r in json.loads(os.environ["RUNS_JSON"])["workflow_runs"] if str(r["id"]) == os.environ["RID"])
path = (run.get("path") or "").split("@")[0]
head = (run.get("head_repository") or {}).get("full_name")
bad = []
if path != ".github/workflows/ci.yml": bad.append("workflow " + path)
if mine[0].get("name") != "test": bad.append("job " + str(mine[0].get("name")))
if run.get("event") not in ("push", "workflow_dispatch", "pull_request"): bad.append("event " + str(run.get("event")))
if head != os.environ["REPO_FULL"]: bad.append("head repository " + str(head))
print("refuse: " + "; ".join(bad) if bad else "admit")
' <<< "$jobs" 2> /dev/null) || verdict=none
    case $verdict in
      admit) JOB_VERDICT="run $rid"; return 0 ;;
      refuse*) JOB_VERDICT="run $rid ${verdict#refuse: }"; return 1 ;;
    esac
  done
  return 2
}
