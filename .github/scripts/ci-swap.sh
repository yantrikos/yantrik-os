#!/usr/bin/env bash
# Make and enable a swap file on a GitHub-hosted runner, printing memory and disk before and after.
# Usage: ci-swap.sh <size, e.g. 12G>      Skipped by the workflows on self-hosted runners.
set -euo pipefail
size="${1:?usage: ci-swap.sh SIZE}"
file=/mnt/yantrik-swap

echo "== before"; free -h; df -h
sudo swapoff -a || true
sudo rm -f "$file"
sudo fallocate -l "$size" "$file"
sudo chmod 600 "$file"
sudo mkswap "$file" > /dev/null
sudo swapon "$file"
echo "== after"; free -h; df -h
