#!/usr/bin/env bash
# One line every 20 s about the machine, printed into the log of the step that started it.
#
# Why this exists: CI and ISO builds have lost their hosted runner while compiling ("The runner
# has received a shutdown signal" / "lost communication with the server", minutes of silence, no
# output). When the runner is torn down no later step runs, not even `if: always()`, so a sampler
# that writes to a file and prints it afterwards says nothing. Printed live, the samples are in
# the log whatever kills it. Costs one sleeping process and a few hundred bytes a minute.
#
# Usage, in the step that does the heavy work:
#   .github/scripts/ci-sampler.sh &
#   trap 'kill $! 2>/dev/null || true' EXIT
#   cargo build ...
#
# Every line has memory (used/total, available), swap (used/total), free disk on / (the swap
# file is on it) and the three largest processes. A header with `free -h` and `df -h` comes
# first, and on SIGTERM (the step ending) the lowest available memory and the highest swap use
# seen are printed. A lost runner prints no summary; its last sample line is then the evidence.
min_avail=""
max_swap=0

summary() {
  echo "[sample-summary] lowest available ${min_avail:-?} MB, peak swap used ${max_swap} MB"
  exit 0
}
trap summary TERM INT

echo "[sample-start $(date -u +%T)] machine before the step's work:"
free -h
df -h /
nproc | sed 's/^/cpus: /'

while true; do
  read -r used total avail < <(free -m | awk '/^Mem:/{print $3, $2, $7}')
  read -r sused stotal < <(free -m | awk '/^Swap:/{print $3, $2}')
  { [ -z "$min_avail" ] || [ "$avail" -lt "$min_avail" ]; } && min_avail=$avail
  [ "$sused" -gt "$max_swap" ] && max_swap=$sused
  # `/` only: the swap file is on it, and /mnt is not a mount of its own on this image.
  disk=$(df -h / | awk 'NR>1{printf " | %s %s used (%s free)", $6, $5, $4}')
  top=$(ps -eo rss,comm --sort=-rss | awk 'NR>1 && NR<=4{printf " %s:%dMB", $2, $1/1024}')
  echo "[sample $(date -u +%T)] mem used $used/$total MB avail $avail | swap used $sused/$stotal MB$disk | top:$top"
  # sleep in the background so a TERM is handled at once, not after the 20 s
  sleep 20 &
  wait $! || true
done
