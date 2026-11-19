#!/bin/bash
# Refuses any job that is not ci.yml's `test` for this repository, from main or a same-repo branch.
# Run by the runner before the first step of a job (ACTIONS_RUNNER_HOOK_JOB_STARTED), from the
# event payload GitHub wrote, so job code cannot change it. Root-owned in the template.
set -euo pipefail
die() { echo "::error::yantrik-build refuses this job: $*"; exit 1; }
[ "${GITHUB_REPOSITORY:-}" = yantrikos/yantrik-os ] || die "repository ${GITHUB_REPOSITORY:-?}"
case "${GITHUB_WORKFLOW_REF:-}" in yantrikos/yantrik-os/.github/workflows/ci.yml@*) ;; *) die "workflow ${GITHUB_WORKFLOW_REF:-?}";; esac
[ "${GITHUB_JOB:-}" = test ] || die "job ${GITHUB_JOB:-?}"
case "${GITHUB_EVENT_NAME:-}" in
  push|workflow_dispatch) ;;
  pull_request)
    [ "$(jq -r .pull_request.head.repo.full_name "$GITHUB_EVENT_PATH")" = yantrikos/yantrik-os ] || die "fork PR"
    [ "$(jq -r .pull_request.head.repo.fork "$GITHUB_EVENT_PATH")" = false ] || die "fork PR" ;;
  *) die "event ${GITHUB_EVENT_NAME:-?}" ;;
esac
