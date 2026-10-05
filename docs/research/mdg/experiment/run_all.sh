#!/bin/sh
# Main R1 runs (CPU: 4 parallel single-thread jobs). On a GPU add --device cuda --threads 1.
cd "$(dirname "$0")"
{ echo "plain 0"
  for c in compact mdg_seq compact_bpe mdg_seq_bpe mdg_packed compact_packed; do for s in 0 1 2; do echo "$c $s"; done; done
} | xargs -P ${JOBS:-4} -L 1 sh -c 'python3 train.py --cond $0 --seed $1 --d 64 --layers 3 --bs 32 --steps ${STEPS:-8000} --threads 1 --out results > logs/$0_s$1.log 2>&1'
