#!/usr/bin/env bash
# Run one heavy compile on a GitHub-hosted runner: memory samples in the log, and the compile
# in a cgroup of its own whose memory is capped below the machine's, so the runner survives it.
#
#   .github/heavy-build.sh cargo build --release -p yantrik-ui-slint --locked -j 1
#
# Why (#1234). The generated UI crate makes one rustc hold about 15 GB on a 16 GB runner. On the
# good ISO run of 21 Nov RAM sat at 15.5 of 16 GB with 450 MB available; on the bad runs the same
# step thrashed for 50 minutes and GitHub declared the runner lost ("lost communication"), with no
# log. Under memory pressure the kernel swaps out whatever is cold, the runner's own agent
# included, and an agent that cannot answer GitHub is a lost job. `MemoryHigh` makes the kernel
# reclaim from the compile's cgroup first, into the swap the workflow made, and leaves the rest
# of the machine (the agent, this script's sampler) its memory. The compile is slower near the
# cap; a slow step finishes, a lost runner does not.
#
# HEAVY_BUILD_HEADROOM_MB (default 2560) is what stays outside the cap.
set -euo pipefail

[ "$#" -gt 0 ] || { echo "usage: $0 <command> [args...]" >&2; exit 2; }

"$(dirname "$0")/ci-sampler.sh" &
sampler=$!
trap 'kill "$sampler" 2>/dev/null || true' EXIT

total_mb=$(awk '/^MemTotal:/{print int($2 / 1024)}' /proc/meminfo)
high_mb=$((total_mb - ${HEAVY_BUILD_HEADROOM_MB:-2560}))
echo "heavy-build: memory ${total_mb} MB, compile capped at MemoryHigh=${high_mb}M"

# A transient scope, run as this user with this environment: sudo is only for systemd-run.
# `env` puts PATH and HOME back, which sudo's own policy would otherwise replace.
sudo --preserve-env systemd-run --quiet --scope --collect \
  -p MemoryHigh="${high_mb}M" -p MemorySwapMax=infinity \
  --uid="$(id -u)" --gid="$(id -g)" \
  -- env "PATH=$PATH" "HOME=$HOME" "$@"
