#!/usr/bin/env bash
# One line every 20 s about the machine, printed into the log of the step that started it.
#
# Why this exists: the test job has been killed while compiling test binaries ("The runner has
# received a shutdown signal", exit 143, minutes of silence, no test output). When the runner is
# torn down no later step runs, not even `if: always()`, so a sampler that writes to a file and
# prints it afterwards says nothing. Printed live, the samples are in the log whatever kills it.
# Costs one sleeping process and a few hundred bytes a minute.
while true; do
  mem=$(free -m | awk '/^Mem:/{printf "mem used %s/%s MB avail %s", $3, $2, $7} /^Swap:/{printf " | swap used %s/%s MB", $3, $2}')
  # `/` only: the swap file is on it, and /mnt is not a mount of its own on this image.
  disk=$(df -h / | awk 'NR>1{printf " | %s %s used (%s free)", $6, $5, $4}')
  top=$(ps -eo rss,comm --sort=-rss | awk 'NR>1 && NR<=4{printf " %s:%dMB", $2, $1/1024}')
  echo "[sample $(date -u +%T)] $mem$disk | top:$top"
  sleep 20
done
